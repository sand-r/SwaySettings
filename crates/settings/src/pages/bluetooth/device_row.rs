//! Bluetooth device row widget.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::services::bluez::{BluezDaemon, BluezDevice};

/// Device connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceState {
    #[default]
    Unknown,
    Unpaired,
    Pairing,
    Connected,
    Connecting,
    Disconnecting,
    NotConnected,
}

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/BluetoothDeviceRow.ui")]
pub struct BluetoothDeviceRowImpl {
    #[template_child(id = "device_image")]
    pub device_image: TemplateChild<gtk4::Image>,
    #[template_child(id = "status_spinner")]
    pub status_spinner: TemplateChild<gtk4::Spinner>,
    #[template_child(id = "remove_button")]
    pub remove_button: TemplateChild<gtk4::Button>,
    #[template_child(id = "connect_button")]
    pub connect_button: TemplateChild<gtk4::Button>,

    pub device_path: RefCell<String>,
    pub device_name: RefCell<String>,
    pub device_rssi: Cell<i16>,
    pub state: Cell<DeviceState>,
    pub daemon: RefCell<Option<Rc<BluezDaemon>>>,
}

#[glib::object_subclass]
impl ObjectSubclass for BluetoothDeviceRowImpl {
    const NAME: &'static str = "SwaySettingsBluetoothDeviceRow";
    type Type = BluetoothDeviceRow;
    type ParentType = libadwaita::ActionRow;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for BluetoothDeviceRowImpl {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        obj.setup_buttons();
    }
}
impl WidgetImpl for BluetoothDeviceRowImpl {}
impl ListBoxRowImpl for BluetoothDeviceRowImpl {}
impl PreferencesRowImpl for BluetoothDeviceRowImpl {}
impl ActionRowImpl for BluetoothDeviceRowImpl {}

glib::wrapper! {
    pub struct BluetoothDeviceRow(ObjectSubclass<BluetoothDeviceRowImpl>)
        @extends libadwaita::ActionRow, libadwaita::PreferencesRow, gtk4::ListBoxRow, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Actionable, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl BluetoothDeviceRow {
    pub fn new(device: &BluezDevice, daemon: Option<Rc<BluezDaemon>>) -> Self {
        let row: Self = glib::Object::builder().build();
        *row.imp().daemon.borrow_mut() = daemon;
        row.update_from_device(device);
        row
    }

    fn setup_buttons(&self) {
        let imp = self.imp();

        imp.connect_button.connect_clicked(clone!(
            #[weak(rename_to = obj)]
            self,
            move |_| {
                obj.on_connect_clicked();
            }
        ));

        imp.remove_button.connect_clicked(clone!(
            #[weak(rename_to = obj)]
            self,
            move |_| {
                obj.on_remove_clicked();
            }
        ));
    }

    pub fn update_from_device(&self, device: &BluezDevice) {
        let imp = self.imp();

        // Store device info
        *imp.device_path.borrow_mut() = device.object_path();
        *imp.device_name.borrow_mut() = device.display_name();
        imp.device_rssi.set(device.rssi().unwrap_or(i16::MIN));

        // Update title and icon
        self.set_title(&device.display_name());
        imp.device_image.set_icon_name(Some(&device.display_icon()));

        // Determine state
        let state = if !device.paired() {
            DeviceState::Unpaired
        } else if device.connected() {
            DeviceState::Connected
        } else {
            DeviceState::NotConnected
        };

        self.set_state(state);
    }

    pub fn set_state(&self, state: DeviceState) {
        let imp = self.imp();
        let old_state = imp.state.replace(state);

        if old_state == state {
            return;
        }

        // Update subtitle
        let subtitle = match state {
            DeviceState::Unknown => "",
            DeviceState::Unpaired => "Not Paired",
            DeviceState::Pairing => "Pairing...",
            DeviceState::Connected => "Connected",
            DeviceState::Connecting => "Connecting...",
            DeviceState::Disconnecting => "Disconnecting...",
            DeviceState::NotConnected => "Not Connected",
        };
        self.set_subtitle(subtitle);

        // Update spinner visibility
        let show_spinner = matches!(
            state,
            DeviceState::Pairing | DeviceState::Connecting | DeviceState::Disconnecting
        );
        imp.status_spinner.set_visible(show_spinner);

        // Update button visibility and label
        let (show_remove, show_connect, connect_label) = match state {
            DeviceState::Unpaired => (false, true, "Pair"),
            DeviceState::Pairing => (false, false, ""),
            DeviceState::Connected => (true, true, "Disconnect"),
            DeviceState::Connecting | DeviceState::Disconnecting => (false, false, ""),
            DeviceState::NotConnected => (true, true, "Connect"),
            DeviceState::Unknown => (false, false, ""),
        };

        imp.remove_button.set_visible(show_remove);
        imp.connect_button.set_visible(show_connect);
        if !connect_label.is_empty() {
            imp.connect_button.set_label(connect_label);
        }
    }

    pub fn device_path(&self) -> String {
        self.imp().device_path.borrow().clone()
    }

    pub fn device_name(&self) -> String {
        self.imp().device_name.borrow().clone()
    }

    pub fn device_rssi(&self) -> i16 {
        self.imp().device_rssi.get()
    }

    fn on_connect_clicked(&self) {
        let imp = self.imp();
        let state = imp.state.get();
        let device_path = self.device_path();

        let Some(daemon) = imp.daemon.borrow().clone() else {
            return;
        };

        let Some(device) = daemon.get_device(&device_path) else {
            return;
        };

        match state {
            DeviceState::Unpaired => {
                self.set_state(DeviceState::Pairing);
                glib::MainContext::default().spawn_local(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    async move {
                        match device.pair().await {
                            Ok(()) => {
                                log::info!("Pairing initiated with {}", obj.device_name());
                                // Trust the device after pairing
                                if let Err(err) = device.set_trusted(true).await {
                                    log::warn!("Failed to trust device: {}", err);
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to pair: {}", err);
                                obj.set_state(DeviceState::Unpaired);
                            }
                        }
                    }
                ));
            }
            DeviceState::Connected => {
                self.set_state(DeviceState::Disconnecting);
                glib::MainContext::default().spawn_local(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    async move {
                        match device.disconnect().await {
                            Ok(()) => {
                                log::info!("Disconnected from {}", obj.device_name());
                            }
                            Err(err) => {
                                log::warn!("Failed to disconnect: {}", err);
                                obj.set_state(DeviceState::Connected);
                            }
                        }
                    }
                ));
            }
            DeviceState::NotConnected => {
                self.set_state(DeviceState::Connecting);
                glib::MainContext::default().spawn_local(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    async move {
                        match device.connect().await {
                            Ok(()) => {
                                log::info!("Connected to {}", obj.device_name());
                            }
                            Err(err) => {
                                log::warn!("Failed to connect: {}", err);
                                obj.set_state(DeviceState::NotConnected);
                            }
                        }
                    }
                ));
            }
            _ => {}
        }
    }

    fn on_remove_clicked(&self) {
        let imp = self.imp();
        let device_path = self.device_path();

        let Some(daemon) = imp.daemon.borrow().clone() else {
            return;
        };

        let Some(device) = daemon.get_device(&device_path) else {
            return;
        };

        let Some(adapter_path) = device.adapter() else {
            log::warn!("Device has no adapter");
            return;
        };

        let Some(adapter) = daemon.get_adapter(&adapter_path) else {
            log::warn!("Adapter not found: {}", adapter_path);
            return;
        };

        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                match adapter.remove_device(&device_path).await {
                    Ok(()) => {
                        log::info!("Removed device {}", obj.device_name());
                    }
                    Err(err) => {
                        log::warn!("Failed to remove device: {}", err);
                    }
                }
            }
        ));
    }
}
