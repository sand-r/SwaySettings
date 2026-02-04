//! BluezAdapter - D-Bus proxy for org.bluez.Adapter1 interface.

use std::cell::RefCell;

use gio::prelude::*;
use glib::variant::ObjectPath;

/// D-Bus proxy for BlueZ Adapter1 interface.
///
/// Provides async access to Bluetooth adapter properties and methods
/// like power control and device discovery.
pub struct BluezAdapter {
    proxy: gio::DBusProxy,
    changed_handler_id: RefCell<Option<glib::SignalHandlerId>>,
}

impl BluezAdapter {
    /// Create a proxy for an adapter at the given object path.
    pub async fn new(object_path: &str) -> Result<Self, glib::Error> {
        let proxy = gio::DBusProxy::for_bus_future(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "org.bluez",
            object_path,
            "org.bluez.Adapter1",
        )
        .await?;

        Ok(Self {
            proxy,
            changed_handler_id: RefCell::new(None),
        })
    }

    /// Create a proxy from an existing DBusProxy.
    pub fn from_proxy(proxy: gio::DBusProxy) -> Self {
        Self {
            proxy,
            changed_handler_id: RefCell::new(None),
        }
    }

    /// Get the object path of this adapter.
    pub fn object_path(&self) -> String {
        self.proxy.object_path().to_string()
    }

    /// Get the underlying proxy.
    pub fn proxy(&self) -> &gio::DBusProxy {
        &self.proxy
    }

    /// Check if the adapter is powered on.
    pub fn powered(&self) -> bool {
        self.proxy
            .cached_property("Powered")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Check if discovery is currently active.
    pub fn discovering(&self) -> bool {
        self.proxy
            .cached_property("Discovering")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Check if the adapter is in discoverable mode.
    pub fn discoverable(&self) -> bool {
        self.proxy
            .cached_property("Discoverable")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Get the adapter's friendly name.
    pub fn alias(&self) -> Option<String> {
        self.proxy
            .cached_property("Alias")
            .and_then(|v| v.get::<String>())
    }

    /// Get the adapter's Bluetooth address.
    pub fn address(&self) -> Option<String> {
        self.proxy
            .cached_property("Address")
            .and_then(|v| v.get::<String>())
    }

    /// Set the adapter's powered state.
    pub async fn set_powered(&self, powered: bool) -> Result<(), glib::Error> {
        // Properties.Set expects (ssv) - interface, property name, variant-wrapped value
        // When a Variant is put in a tuple and .to_variant() is called, it gets wrapped in 'v'
        let value = glib::Variant::from(powered);
        self.proxy
            .call_future(
                "org.freedesktop.DBus.Properties.Set",
                Some(&("org.bluez.Adapter1", "Powered", value).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Set the adapter's discoverable state.
    pub async fn set_discoverable(&self, discoverable: bool) -> Result<(), glib::Error> {
        let value = glib::Variant::from(discoverable);
        self.proxy
            .call_future(
                "org.freedesktop.DBus.Properties.Set",
                Some(&("org.bluez.Adapter1", "Discoverable", value).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Start device discovery.
    pub async fn start_discovery(&self) -> Result<(), glib::Error> {
        self.proxy
            .call_future("StartDiscovery", None, gio::DBusCallFlags::NONE, -1)
            .await?;
        Ok(())
    }

    /// Stop device discovery.
    pub async fn stop_discovery(&self) -> Result<(), glib::Error> {
        self.proxy
            .call_future("StopDiscovery", None, gio::DBusCallFlags::NONE, -1)
            .await?;
        Ok(())
    }

    /// Remove a device from the adapter.
    pub async fn remove_device(&self, device_path: &str) -> Result<(), glib::Error> {
        let path = ObjectPath::try_from(device_path)
            .map_err(|e| glib::Error::new(gio::IOErrorEnum::InvalidArgument, &e.to_string()))?;
        self.proxy
            .call_future(
                "RemoveDevice",
                Some(&(path,).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Connect a callback to property changes.
    pub fn connect_properties_changed<F: Fn(&str, &glib::Variant) + 'static>(&self, callback: F) {
        let handler_id = self
            .proxy
            .connect_local("g-properties-changed", false, move |values| {
                // The second argument is the changed properties dict as a{sv}
                if let Some(changed) = values.get(1).and_then(|v| v.get::<glib::Variant>().ok()) {
                    // The changed dict contains property name -> value pairs
                    // We just notify that something changed without detailed info
                    callback("properties-changed", &changed);
                }
                None
            });
        *self.changed_handler_id.borrow_mut() = Some(handler_id);
    }

    /// Disconnect from property change signals.
    pub fn disconnect_changed(&self) {
        if let Some(handler_id) = self.changed_handler_id.borrow_mut().take() {
            self.proxy.disconnect(handler_id);
        }
    }
}

impl Drop for BluezAdapter {
    fn drop(&mut self) {
        self.disconnect_changed();
    }
}
