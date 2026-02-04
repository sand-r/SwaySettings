//! BluezDaemon - Coordinator for BlueZ D-Bus interactions.
//!
//! Manages the connection to BlueZ, tracks adapters and devices,
//! and handles the pairing agent lifecycle.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gio::prelude::*;
use glib::clone;

/// Helper to get interface from DBusObject, avoiding method ambiguity.
fn dbus_object_interface(obj: &gio::DBusObject, name: &str) -> Option<gio::DBusInterface> {
    gio::prelude::DBusObjectExt::interface(obj, name)
}

use super::adapter::BluezAdapter;
use super::agent::{AgentRequest, BluezAgent, AGENT_CAPABILITY, AGENT_PATH};
use super::agent_manager::BluezAgentManager;
use super::device::BluezDevice;

/// Callback types for daemon events.
pub type AdapterCallback = Rc<RefCell<Option<Box<dyn Fn(&BluezAdapter)>>>>;
pub type DeviceCallback = Rc<RefCell<Option<Box<dyn Fn(&BluezDevice)>>>>;
pub type StateCallback = Rc<RefCell<Option<Box<dyn Fn(bool)>>>>;
pub type AgentRequestCallback = Rc<RefCell<Option<Box<dyn Fn(AgentRequest) -> Option<bool>>>>>;

/// BlueZ daemon coordinator.
///
/// Tracks all BlueZ objects (adapters and devices) via the ObjectManager
/// interface and provides callbacks for state changes.
pub struct BluezDaemon {
    /// Currently known adapters, keyed by object path.
    /// Wrapped in Rc to allow sharing with property change callbacks.
    adapters: Rc<RefCell<HashMap<String, BluezAdapter>>>,
    /// Currently known devices, keyed by object path.
    /// Wrapped in Rc to allow sharing with property change callbacks.
    devices: Rc<RefCell<HashMap<String, BluezDevice>>>,
    /// The pairing agent (if registered).
    agent: RefCell<Option<BluezAgent>>,
    /// The agent manager proxy.
    agent_manager: RefCell<Option<BluezAgentManager>>,
    /// Whether BlueZ service is available.
    service_available: RefCell<bool>,
    /// Bus name watcher ID (stored as raw ID for type compatibility).
    #[allow(dead_code)]
    watcher_active: RefCell<bool>,
    /// Object manager proxy.
    object_manager: RefCell<Option<gio::DBusObjectManagerClient>>,
    /// Object manager signal handlers.
    manager_handlers: RefCell<Vec<glib::SignalHandlerId>>,

    // Callbacks
    on_adapter_added: AdapterCallback,
    on_adapter_removed: AdapterCallback,
    on_adapter_changed: AdapterCallback,
    on_device_added: DeviceCallback,
    on_device_removed: DeviceCallback,
    on_device_changed: DeviceCallback,
    on_service_state_changed: StateCallback,
    on_agent_request: AgentRequestCallback,
}

impl BluezDaemon {
    /// Create a new daemon instance.
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            adapters: Rc::new(RefCell::new(HashMap::new())),
            devices: Rc::new(RefCell::new(HashMap::new())),
            agent: RefCell::new(None),
            agent_manager: RefCell::new(None),
            service_available: RefCell::new(false),
            watcher_active: RefCell::new(false),
            object_manager: RefCell::new(None),
            manager_handlers: RefCell::new(Vec::new()),
            on_adapter_added: Rc::new(RefCell::new(None)),
            on_adapter_removed: Rc::new(RefCell::new(None)),
            on_adapter_changed: Rc::new(RefCell::new(None)),
            on_device_added: Rc::new(RefCell::new(None)),
            on_device_removed: Rc::new(RefCell::new(None)),
            on_device_changed: Rc::new(RefCell::new(None)),
            on_service_state_changed: Rc::new(RefCell::new(None)),
            on_agent_request: Rc::new(RefCell::new(None)),
        })
    }

    /// Start monitoring the BlueZ service.
    pub fn start(self: &Rc<Self>) {
        if *self.watcher_active.borrow() {
            return;
        }
        *self.watcher_active.borrow_mut() = true;

        let daemon = self.clone();

        // Note: bus_watch_name keeps the watch active as long as the closures are alive.
        // The returned WatcherId could be used for bus_unwatch_name, but we rely on
        // the closures being dropped when the daemon is dropped.
        let _watcher_id = gio::bus_watch_name(
            gio::BusType::System,
            "org.bluez",
            gio::BusNameWatcherFlags::NONE,
            clone!(
                #[strong]
                daemon,
                move |_conn, _name, _owner| {
                    log::info!("BlueZ service appeared");
                    *daemon.service_available.borrow_mut() = true;
                    daemon.notify_service_state(true);
                    daemon.init_object_manager();
                }
            ),
            clone!(
                #[strong]
                daemon,
                move |_conn, _name| {
                    log::info!("BlueZ service disappeared");
                    *daemon.service_available.borrow_mut() = false;
                    daemon.notify_service_state(false);
                    daemon.cleanup();
                }
            ),
        );
    }

    /// Stop monitoring and clean up.
    pub fn stop(&self) {
        *self.watcher_active.borrow_mut() = false;
        self.cleanup();
    }

    fn cleanup(&self) {
        // Disconnect object manager signals
        if let Some(ref manager) = *self.object_manager.borrow() {
            for handler_id in self.manager_handlers.borrow_mut().drain(..) {
                manager.disconnect(handler_id);
            }
        }
        *self.object_manager.borrow_mut() = None;
        *self.agent.borrow_mut() = None;
        *self.agent_manager.borrow_mut() = None;
        self.adapters.borrow_mut().clear();
        self.devices.borrow_mut().clear();
    }

    fn init_object_manager(self: &Rc<Self>) {
        // Use the sync API - this blocks briefly but only during init
        match gio::DBusObjectManagerClient::for_bus_sync(
            gio::BusType::System,
            gio::DBusObjectManagerClientFlags::NONE,
            "org.bluez",
            "/",
            None::<&gio::Cancellable>,
        ) {
            Ok(manager) => {
                self.setup_object_manager(manager);
            }
            Err(err) => {
                log::error!("Failed to create BlueZ object manager: {}", err);
            }
        }
    }

    fn setup_object_manager(self: &Rc<Self>, manager: gio::DBusObjectManagerClient) {
        // Process existing objects
        for obj in manager.objects() {
            self.process_object_added(&obj);
        }

        // Connect to interface-added signal
        let daemon = self.clone();
        let handler1 = manager.connect_interface_added(move |_mgr, obj, iface| {
            daemon.handle_interface_added(obj, iface);
        });

        // Connect to interface-removed signal
        let daemon = self.clone();
        let handler2 = manager.connect_interface_removed(move |_mgr, obj, iface| {
            daemon.handle_interface_removed(obj, iface);
        });

        // Connect to object-added signal
        let daemon = self.clone();
        let handler3 = manager.connect_object_added(move |_mgr, obj| {
            daemon.process_object_added(obj);
        });

        // Connect to object-removed signal
        let daemon = self.clone();
        let handler4 = manager.connect_object_removed(move |_mgr, obj| {
            daemon.process_object_removed(obj);
        });

        self.manager_handlers
            .borrow_mut()
            .extend([handler1, handler2, handler3, handler4]);
        *self.object_manager.borrow_mut() = Some(manager);

        // Register the pairing agent
        self.register_agent();
    }

    fn process_object_added(&self, obj: &gio::DBusObject) {
        let path = obj.object_path().to_string();

        // Check for Adapter1 interface
        if let Some(iface) = dbus_object_interface(&obj, "org.bluez.Adapter1") {
            if let Some(proxy) = iface.downcast_ref::<gio::DBusProxy>() {
                let adapter = BluezAdapter::from_proxy(proxy.clone());
                log::debug!("Adapter added: {}", path);

                // Connect to property changes
                let path_clone = path.clone();
                let on_changed = self.on_adapter_changed.clone();
                let adapters = self.adapters.clone();
                adapter.connect_properties_changed(move |prop_name, _value| {
                    log::trace!("Adapter {} property changed: {}", path_clone, prop_name);
                    if let Some(adapter) = adapters.borrow().get(&path_clone) {
                        if let Some(ref cb) = *on_changed.borrow() {
                            cb(adapter);
                        }
                    }
                });

                self.adapters.borrow_mut().insert(path.clone(), adapter);
                if let Some(adapter) = self.adapters.borrow().get(&path) {
                    self.notify_adapter_added(adapter);
                }
            }
        }

        // Check for Device1 interface
        if let Some(iface) = dbus_object_interface(&obj, "org.bluez.Device1") {
            if let Some(proxy) = iface.downcast_ref::<gio::DBusProxy>() {
                let device = BluezDevice::from_proxy(proxy.clone());
                log::debug!("Device added: {} ({})", device.display_name(), path);

                // Connect to property changes
                let path_clone = path.clone();
                let on_changed = self.on_device_changed.clone();
                let devices = self.devices.clone();
                device.connect_properties_changed(move |prop_name, _value| {
                    log::trace!("Device {} property changed: {}", path_clone, prop_name);
                    if let Some(device) = devices.borrow().get(&path_clone) {
                        if let Some(ref cb) = *on_changed.borrow() {
                            cb(device);
                        }
                    }
                });

                self.devices.borrow_mut().insert(path.clone(), device);
                if let Some(device) = self.devices.borrow().get(&path) {
                    self.notify_device_added(device);
                }
            }
        }
    }

    fn process_object_removed(&self, obj: &gio::DBusObject) {
        let path = obj.object_path().to_string();

        // Remove adapter and drop borrow before calling callback
        let removed_adapter = self.adapters.borrow_mut().remove(&path);
        if let Some(adapter) = removed_adapter {
            log::debug!("Adapter removed: {}", path);
            self.notify_adapter_removed(&adapter);
        }

        // Remove device and drop borrow before calling callback
        let removed_device = self.devices.borrow_mut().remove(&path);
        if let Some(device) = removed_device {
            log::debug!("Device removed: {}", path);
            self.notify_device_removed(&device);
        }
    }

    fn handle_interface_added(&self, obj: &gio::DBusObject, iface: &gio::DBusInterface) {
        let path = obj.object_path().to_string();
        let iface_name = iface.info().map(|i| i.name().to_string());

        match iface_name.as_deref() {
            Some("org.bluez.Adapter1") => {
                if let Some(proxy) = iface.downcast_ref::<gio::DBusProxy>() {
                    let adapter = BluezAdapter::from_proxy(proxy.clone());
                    log::debug!("Adapter interface added: {}", path);

                    // Connect to property changes
                    let path_clone = path.clone();
                    let on_changed = self.on_adapter_changed.clone();
                    let adapters = self.adapters.clone();
                    adapter.connect_properties_changed(move |prop_name, _value| {
                        log::trace!("Adapter {} property changed: {}", path_clone, prop_name);
                        if let Some(adapter) = adapters.borrow().get(&path_clone) {
                            if let Some(ref cb) = *on_changed.borrow() {
                                cb(adapter);
                            }
                        }
                    });

                    self.adapters.borrow_mut().insert(path.clone(), adapter);
                    if let Some(adapter) = self.adapters.borrow().get(&path) {
                        self.notify_adapter_added(adapter);
                    }
                }
            }
            Some("org.bluez.Device1") => {
                if let Some(proxy) = iface.downcast_ref::<gio::DBusProxy>() {
                    let device = BluezDevice::from_proxy(proxy.clone());
                    log::debug!(
                        "Device interface added: {} ({})",
                        device.display_name(),
                        path
                    );

                    let path_clone = path.clone();
                    let on_changed = self.on_device_changed.clone();
                    let devices = self.devices.clone();
                    device.connect_properties_changed(move |prop_name, _value| {
                        log::trace!("Device {} property changed: {}", path_clone, prop_name);
                        if let Some(device) = devices.borrow().get(&path_clone) {
                            if let Some(ref cb) = *on_changed.borrow() {
                                cb(device);
                            }
                        }
                    });

                    self.devices.borrow_mut().insert(path.clone(), device);
                    if let Some(device) = self.devices.borrow().get(&path) {
                        self.notify_device_added(device);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_interface_removed(&self, obj: &gio::DBusObject, iface: &gio::DBusInterface) {
        let path = obj.object_path().to_string();
        let iface_name = iface.info().map(|i| i.name().to_string());

        match iface_name.as_deref() {
            Some("org.bluez.Adapter1") => {
                // Remove and drop borrow before calling callback
                let removed = self.adapters.borrow_mut().remove(&path);
                if let Some(adapter) = removed {
                    log::debug!("Adapter interface removed: {}", path);
                    self.notify_adapter_removed(&adapter);
                }
            }
            Some("org.bluez.Device1") => {
                // Remove and drop borrow before calling callback
                let removed = self.devices.borrow_mut().remove(&path);
                if let Some(device) = removed {
                    log::debug!("Device interface removed: {}", path);
                    self.notify_device_removed(&device);
                }
            }
            _ => {}
        }
    }

    fn register_agent(self: &Rc<Self>) {
        let daemon = self.clone();

        glib::MainContext::default().spawn_local(async move {
            // Create the agent
            match BluezAgent::new().await {
                Ok(agent) => {
                    // Set up the callback
                    let on_request = daemon.on_agent_request.clone();
                    agent.set_callback(move |request| {
                        if let Some(ref cb) = *on_request.borrow() {
                            cb(request)
                        } else {
                            // Auto-accept if no callback
                            Some(true)
                        }
                    });

                    *daemon.agent.borrow_mut() = Some(agent);

                    // Register with agent manager
                    match BluezAgentManager::new().await {
                        Ok(manager) => {
                            if let Err(err) =
                                manager.register_agent(AGENT_PATH, AGENT_CAPABILITY).await
                            {
                                log::warn!("Failed to register agent: {}", err);
                            } else {
                                log::info!("BlueZ agent registered at {}", AGENT_PATH);
                                // Try to become the default agent
                                if let Err(err) = manager.request_default_agent(AGENT_PATH).await {
                                    log::debug!("Could not become default agent: {}", err);
                                }
                            }
                            *daemon.agent_manager.borrow_mut() = Some(manager);
                        }
                        Err(err) => {
                            log::warn!("Failed to create agent manager: {}", err);
                        }
                    }
                }
                Err(err) => {
                    log::warn!("Failed to create agent: {}", err);
                }
            }
        });
    }

    // Notification helpers

    fn notify_service_state(&self, available: bool) {
        if let Some(ref cb) = *self.on_service_state_changed.borrow() {
            cb(available);
        }
    }

    fn notify_adapter_added(&self, adapter: &BluezAdapter) {
        if let Some(ref cb) = *self.on_adapter_added.borrow() {
            cb(adapter);
        }
    }

    fn notify_adapter_removed(&self, adapter: &BluezAdapter) {
        if let Some(ref cb) = *self.on_adapter_removed.borrow() {
            cb(adapter);
        }
    }

    fn notify_adapter_changed(&self, adapter: &BluezAdapter) {
        if let Some(ref cb) = *self.on_adapter_changed.borrow() {
            cb(adapter);
        }
    }

    fn notify_device_added(&self, device: &BluezDevice) {
        if let Some(ref cb) = *self.on_device_added.borrow() {
            cb(device);
        }
    }

    fn notify_device_removed(&self, device: &BluezDevice) {
        if let Some(ref cb) = *self.on_device_removed.borrow() {
            cb(device);
        }
    }

    // Public API

    /// Check if the BlueZ service is available.
    pub fn is_service_available(&self) -> bool {
        *self.service_available.borrow()
    }

    /// Get all known adapters.
    pub fn adapters(&self) -> Vec<BluezAdapter> {
        self.adapters
            .borrow()
            .values()
            .map(|a| BluezAdapter::from_proxy(a.proxy().clone()))
            .collect()
    }

    /// Get a specific adapter by path.
    pub fn get_adapter(&self, path: &str) -> Option<BluezAdapter> {
        self.adapters
            .borrow()
            .get(path)
            .map(|a| BluezAdapter::from_proxy(a.proxy().clone()))
    }

    /// Get the first (default) adapter.
    pub fn default_adapter(&self) -> Option<BluezAdapter> {
        self.adapters
            .borrow()
            .values()
            .next()
            .map(|a| BluezAdapter::from_proxy(a.proxy().clone()))
    }

    /// Get all known devices.
    pub fn devices(&self) -> Vec<BluezDevice> {
        self.devices
            .borrow()
            .values()
            .map(|d| BluezDevice::from_proxy(d.proxy().clone()))
            .collect()
    }

    /// Get paired devices.
    pub fn paired_devices(&self) -> Vec<BluezDevice> {
        self.devices
            .borrow()
            .values()
            .filter(|d| d.paired())
            .map(|d| BluezDevice::from_proxy(d.proxy().clone()))
            .collect()
    }

    /// Get nearby (unpaired) devices.
    pub fn nearby_devices(&self) -> Vec<BluezDevice> {
        self.devices
            .borrow()
            .values()
            .filter(|d| !d.paired())
            .map(|d| BluezDevice::from_proxy(d.proxy().clone()))
            .collect()
    }

    /// Get a specific device by path.
    pub fn get_device(&self, path: &str) -> Option<BluezDevice> {
        self.devices
            .borrow()
            .get(path)
            .map(|d| BluezDevice::from_proxy(d.proxy().clone()))
    }

    /// Check if any adapter is powered on.
    pub fn is_powered(&self) -> bool {
        self.adapters.borrow().values().any(|a| a.powered())
    }

    /// Check if any adapter is discovering.
    pub fn is_discovering(&self) -> bool {
        self.adapters.borrow().values().any(|a| a.discovering())
    }

    // Callback setters

    pub fn set_on_adapter_added<F: Fn(&BluezAdapter) + 'static>(&self, callback: F) {
        *self.on_adapter_added.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_adapter_removed<F: Fn(&BluezAdapter) + 'static>(&self, callback: F) {
        *self.on_adapter_removed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_adapter_changed<F: Fn(&BluezAdapter) + 'static>(&self, callback: F) {
        *self.on_adapter_changed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_device_added<F: Fn(&BluezDevice) + 'static>(&self, callback: F) {
        *self.on_device_added.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_device_removed<F: Fn(&BluezDevice) + 'static>(&self, callback: F) {
        *self.on_device_removed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_device_changed<F: Fn(&BluezDevice) + 'static>(&self, callback: F) {
        *self.on_device_changed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_service_state_changed<F: Fn(bool) + 'static>(&self, callback: F) {
        *self.on_service_state_changed.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_on_agent_request<F: Fn(AgentRequest) -> Option<bool> + 'static>(&self, callback: F) {
        *self.on_agent_request.borrow_mut() = Some(Box::new(callback));
    }
}

impl Default for BluezDaemon {
    fn default() -> Self {
        Self {
            adapters: Rc::new(RefCell::new(HashMap::new())),
            devices: Rc::new(RefCell::new(HashMap::new())),
            agent: RefCell::new(None),
            agent_manager: RefCell::new(None),
            service_available: RefCell::new(false),
            watcher_active: RefCell::new(false),
            object_manager: RefCell::new(None),
            manager_handlers: RefCell::new(Vec::new()),
            on_adapter_added: Rc::new(RefCell::new(None)),
            on_adapter_removed: Rc::new(RefCell::new(None)),
            on_adapter_changed: Rc::new(RefCell::new(None)),
            on_device_added: Rc::new(RefCell::new(None)),
            on_device_removed: Rc::new(RefCell::new(None)),
            on_device_changed: Rc::new(RefCell::new(None)),
            on_service_state_changed: Rc::new(RefCell::new(None)),
            on_agent_request: Rc::new(RefCell::new(None)),
        }
    }
}

impl Drop for BluezDaemon {
    fn drop(&mut self) {
        self.stop();
    }
}
