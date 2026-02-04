//! Bluetooth settings page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;

use crate::services::bluez::{AgentRequest, BluezDaemon, BluezDevice};

use super::device_row::BluetoothDeviceRow;
use super::pair_dialog::BluetoothPairDialog;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/BluetoothContent.ui")]
pub struct BluetoothContentImpl {
    #[template_child(id = "stack")]
    pub stack: TemplateChild<gtk4::Stack>,
    #[template_child(id = "power_switch")]
    pub power_switch: TemplateChild<libadwaita::SwitchRow>,
    #[template_child(id = "paired_group")]
    pub paired_group: TemplateChild<libadwaita::PreferencesGroup>,
    #[template_child(id = "paired_list_box")]
    pub paired_list_box: TemplateChild<gtk4::ListBox>,
    #[template_child(id = "nearby_group")]
    pub nearby_group: TemplateChild<libadwaita::PreferencesGroup>,
    #[template_child(id = "nearby_list_box")]
    pub nearby_list_box: TemplateChild<gtk4::ListBox>,
    #[template_child(id = "discovery_spinner")]
    pub discovery_spinner: TemplateChild<gtk4::Spinner>,
    #[template_child(id = "scan_button")]
    pub scan_button: TemplateChild<gtk4::Button>,
    #[template_child(id = "status_page")]
    pub status_page: TemplateChild<libadwaita::StatusPage>,

    pub daemon: RefCell<Option<Rc<BluezDaemon>>>,
    pub current_dialog: RefCell<Option<BluetoothPairDialog>>,
    /// Flag to prevent feedback loop when updating UI programmatically.
    pub updating_ui: Cell<bool>,
}

#[glib::object_subclass]
impl ObjectSubclass for BluetoothContentImpl {
    const NAME: &'static str = "SwaySettingsBluetoothContent";
    type Type = BluetoothContent;
    type ParentType = libadwaita::Bin;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for BluetoothContentImpl {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        obj.setup();
    }

    fn dispose(&self) {
        if let Some(daemon) = self.daemon.borrow_mut().take() {
            daemon.stop();
        }
    }
}
impl WidgetImpl for BluetoothContentImpl {}
impl BinImpl for BluetoothContentImpl {}

glib::wrapper! {
    pub struct BluetoothContent(ObjectSubclass<BluetoothContentImpl>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl BluetoothContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        self.setup_list_sorting();
        self.setup_power_switch();
        self.setup_scan_button();
        self.start_daemon();
    }

    fn setup_list_sorting(&self) {
        let imp = self.imp();

        // Sort paired devices by name
        imp.paired_list_box.set_sort_func(|row1, row2| {
            let name1 = row1
                .downcast_ref::<BluetoothDeviceRow>()
                .map(|r| r.device_name())
                .unwrap_or_default();
            let name2 = row2
                .downcast_ref::<BluetoothDeviceRow>()
                .map(|r| r.device_name())
                .unwrap_or_default();
            name1.cmp(&name2).into()
        });

        // Sort nearby devices by RSSI (signal strength, higher = better)
        imp.nearby_list_box.set_sort_func(|row1, row2| {
            let rssi1 = row1
                .downcast_ref::<BluetoothDeviceRow>()
                .map(|r| r.device_rssi())
                .unwrap_or(i16::MIN);
            let rssi2 = row2
                .downcast_ref::<BluetoothDeviceRow>()
                .map(|r| r.device_rssi())
                .unwrap_or(i16::MIN);
            // Higher RSSI = better signal = should be first
            rssi2.cmp(&rssi1).into()
        });
    }

    fn setup_power_switch(&self) {
        let imp = self.imp();

        imp.power_switch.connect_active_notify(clone!(
            #[weak(rename_to = obj)]
            self,
            move |switch| {
                obj.on_power_switch_toggled(switch.is_active());
            }
        ));
    }

    fn setup_scan_button(&self) {
        let imp = self.imp();

        imp.scan_button.connect_clicked(clone!(
            #[weak(rename_to = obj)]
            self,
            move |_| {
                obj.toggle_discovery();
            }
        ));
    }

    fn start_daemon(&self) {
        let daemon = BluezDaemon::new();

        // Set up callbacks before starting
        daemon.set_on_service_state_changed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |available| {
                obj.on_service_state_changed(available);
            }
        ));

        daemon.set_on_adapter_added(clone!(
            #[weak(rename_to = obj)]
            self,
            move |adapter| {
                log::debug!("Adapter added: {}", adapter.object_path());
                obj.update_ui_state();
            }
        ));

        daemon.set_on_adapter_removed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |adapter| {
                log::debug!("Adapter removed: {}", adapter.object_path());
                obj.update_ui_state();
            }
        ));

        daemon.set_on_adapter_changed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |_adapter| {
                obj.on_adapter_changed();
            }
        ));

        daemon.set_on_device_added(clone!(
            #[weak(rename_to = obj)]
            self,
            move |device| {
                obj.add_device(device);
            }
        ));

        daemon.set_on_device_removed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |device| {
                obj.remove_device(&device.object_path());
            }
        ));

        daemon.set_on_device_changed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |device| {
                obj.update_device(device);
            }
        ));

        daemon.set_on_agent_request(clone!(
            #[weak(rename_to = obj)]
            self,
            #[upgrade_or]
            Some(false),
            move |request| { obj.handle_agent_request(request) }
        ));

        // Start the daemon
        daemon.start();

        *self.imp().daemon.borrow_mut() = Some(daemon);
    }

    fn on_service_state_changed(&self, available: bool) {
        log::info!("BlueZ service state changed: {}", available);
        self.update_ui_state();
    }

    fn on_adapter_changed(&self) {
        let imp = self.imp();
        let daemon = imp.daemon.borrow();
        let Some(daemon) = daemon.as_ref() else {
            return;
        };

        // Update power switch state with guard to prevent feedback loop
        let is_powered = daemon.is_powered();
        let switch_active = imp.power_switch.is_active();
        log::debug!(
            "on_adapter_changed: is_powered={}, switch_active={}",
            is_powered,
            switch_active
        );

        if switch_active != is_powered {
            log::debug!("Updating switch to match adapter state: {}", is_powered);
            imp.updating_ui.set(true);
            imp.power_switch.set_active(is_powered);
            imp.updating_ui.set(false);
        }

        // Update discovery UI
        self.update_discovery_ui();

        // Update device group visibility based on power state
        imp.nearby_group.set_visible(is_powered);
    }

    fn on_power_switch_toggled(&self, active: bool) {
        let imp = self.imp();

        // Ignore if we're updating the UI programmatically
        if imp.updating_ui.get() {
            log::debug!("Power switch toggled but updating_ui is set, ignoring");
            return;
        }

        let Some(daemon) = imp.daemon.borrow().clone() else {
            log::warn!("Power switch toggled but no daemon");
            return;
        };

        let Some(adapter) = daemon.default_adapter() else {
            log::warn!("Power switch toggled but no adapter");
            return;
        };

        let current_powered = adapter.powered();
        log::info!(
            "Power switch toggled: active={}, current_powered={}",
            active,
            current_powered
        );

        // Don't do anything if the adapter is already in the requested state
        if current_powered == active {
            log::debug!("Adapter already in requested state, skipping");
            return;
        }

        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                log::info!("Calling set_powered({})", active);
                if let Err(err) = adapter.set_powered(active).await {
                    log::error!("Failed to set adapter power to {}: {}", active, err);
                    // Revert the switch state
                    let imp = obj.imp();
                    imp.updating_ui.set(true);
                    imp.power_switch.set_active(!active);
                    imp.updating_ui.set(false);
                } else {
                    log::info!("Successfully set adapter power to {}", active);
                    if active {
                        // Auto-start discovery when powering on
                        obj.start_discovery();
                    }
                }
            }
        ));
    }

    fn toggle_discovery(&self) {
        let imp = self.imp();
        let daemon = imp.daemon.borrow();
        let Some(daemon) = daemon.as_ref() else {
            return;
        };

        if daemon.is_discovering() {
            self.stop_discovery();
        } else {
            self.start_discovery();
        }
    }

    fn start_discovery(&self) {
        let imp = self.imp();
        let Some(daemon) = imp.daemon.borrow().clone() else {
            return;
        };

        let Some(adapter) = daemon.default_adapter() else {
            return;
        };

        // Don't start if already discovering
        if adapter.discovering() {
            return;
        }

        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                if let Err(err) = adapter.start_discovery().await {
                    log::warn!("Failed to start discovery: {}", err);
                }
                obj.update_discovery_ui();
            }
        ));
    }

    fn stop_discovery(&self) {
        let imp = self.imp();
        let Some(daemon) = imp.daemon.borrow().clone() else {
            return;
        };

        let Some(adapter) = daemon.default_adapter() else {
            return;
        };

        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                if let Err(err) = adapter.stop_discovery().await {
                    log::warn!("Failed to stop discovery: {}", err);
                }
                obj.update_discovery_ui();
            }
        ));
    }

    fn update_ui_state(&self) {
        let imp = self.imp();
        let daemon = imp.daemon.borrow();
        let Some(daemon) = daemon.as_ref() else {
            imp.stack.set_visible_child_name("status");
            imp.status_page.set_title("Bluetooth Unavailable");
            imp.status_page
                .set_description(Some("Bluetooth service is not running"));
            return;
        };

        if !daemon.is_service_available() {
            imp.stack.set_visible_child_name("status");
            imp.status_page.set_title("Bluetooth Unavailable");
            imp.status_page
                .set_description(Some("Bluetooth service is not running"));
            return;
        }

        let adapters = daemon.adapters();
        if adapters.is_empty() {
            imp.stack.set_visible_child_name("status");
            imp.status_page.set_title("No Bluetooth Adapters");
            imp.status_page
                .set_description(Some("No Bluetooth adapters were found on this system"));
            return;
        }

        imp.stack.set_visible_child_name("content");

        // Update power switch state with guard to prevent feedback loop
        let is_powered = daemon.is_powered();
        if imp.power_switch.is_active() != is_powered {
            imp.updating_ui.set(true);
            imp.power_switch.set_active(is_powered);
            imp.updating_ui.set(false);
        }

        self.update_discovery_ui();
        self.update_device_lists();

        // Auto-start discovery if powered on and not already discovering
        if is_powered && !daemon.is_discovering() {
            self.start_discovery();
        }
    }

    fn update_discovery_ui(&self) {
        let imp = self.imp();
        let daemon = imp.daemon.borrow();
        let Some(daemon) = daemon.as_ref() else {
            return;
        };

        let is_discovering = daemon.is_discovering();
        imp.discovery_spinner.set_visible(is_discovering);
        imp.scan_button
            .set_label(if is_discovering { "Stop" } else { "Scan" });
    }

    fn update_device_lists(&self) {
        let imp = self.imp();
        let daemon = imp.daemon.borrow();
        let Some(daemon) = daemon.as_ref() else {
            return;
        };

        // Show/hide groups based on whether there are devices
        let paired = daemon.paired_devices();
        let _nearby = daemon.nearby_devices();

        imp.paired_group.set_visible(!paired.is_empty());
        imp.nearby_group.set_visible(daemon.is_powered());
    }

    fn add_device(&self, device: &BluezDevice) {
        let imp = self.imp();
        let daemon = imp.daemon.borrow().clone();

        let row = BluetoothDeviceRow::new(device, daemon);

        // Add to appropriate list
        let list_box = if device.paired() {
            &imp.paired_list_box
        } else {
            &imp.nearby_list_box
        };

        list_box.append(&row);
        list_box.invalidate_sort();

        self.update_device_lists();
    }

    fn remove_device(&self, device_path: &str) {
        let imp = self.imp();

        // Remove from both lists (it should only be in one)
        self.remove_device_from_list(&imp.paired_list_box, device_path);
        self.remove_device_from_list(&imp.nearby_list_box, device_path);

        self.update_device_lists();
    }

    fn remove_device_from_list(&self, list_box: &gtk4::ListBox, device_path: &str) {
        let mut row = list_box.first_child();
        while let Some(child) = row {
            let next = child.next_sibling();
            if let Some(device_row) = child.downcast_ref::<BluetoothDeviceRow>() {
                if device_row.device_path() == device_path {
                    list_box.remove(&child);
                    break;
                }
            }
            row = next;
        }
    }

    fn update_device(&self, device: &BluezDevice) {
        let imp = self.imp();
        let device_path = device.object_path();

        // Find the row in either list
        let row = self
            .find_device_row(&imp.paired_list_box, &device_path)
            .or_else(|| self.find_device_row(&imp.nearby_list_box, &device_path));

        if let Some(row) = row {
            row.update_from_device(device);

            // Check if the device moved between paired and nearby
            let should_be_paired = device.paired();
            let is_in_paired = row.parent() == Some(imp.paired_list_box.clone().upcast());

            if should_be_paired != is_in_paired {
                // Move to the correct list
                if let Some(parent) = row.parent() {
                    if let Some(list_box) = parent.downcast_ref::<gtk4::ListBox>() {
                        list_box.remove(&row);
                    }
                }

                let target_list = if should_be_paired {
                    &imp.paired_list_box
                } else {
                    &imp.nearby_list_box
                };

                target_list.append(&row);
                target_list.invalidate_sort();
            } else {
                // Just resort the current list
                if let Some(parent) = row.parent() {
                    if let Some(list_box) = parent.downcast_ref::<gtk4::ListBox>() {
                        list_box.invalidate_sort();
                    }
                }
            }

            self.update_device_lists();
        }
    }

    fn find_device_row(
        &self,
        list_box: &gtk4::ListBox,
        device_path: &str,
    ) -> Option<BluetoothDeviceRow> {
        let mut row = list_box.first_child();
        while let Some(child) = row {
            if let Some(device_row) = child.downcast_ref::<BluetoothDeviceRow>() {
                if device_row.device_path() == device_path {
                    return Some(device_row.clone());
                }
            }
            row = child.next_sibling();
        }
        None
    }

    fn handle_agent_request(&self, request: AgentRequest) -> Option<bool> {
        match request {
            AgentRequest::RequestConfirmation {
                device_path,
                passkey,
            } => self.show_confirmation_dialog(&device_path, passkey),
            AgentRequest::DisplayPasskey {
                device_path,
                passkey,
                entered: _,
            } => {
                self.show_passkey_display(&device_path, passkey);
                None
            }
            AgentRequest::DisplayPinCode {
                device_path,
                pincode,
            } => {
                self.show_pincode_display(&device_path, &pincode);
                None
            }
            AgentRequest::RequestAuthorization { device_path } => {
                self.show_authorization_dialog(&device_path)
            }
            AgentRequest::AuthorizeService { .. } => {
                // Auto-accept service authorization for paired devices
                Some(true)
            }
            AgentRequest::Cancel => {
                self.close_current_dialog();
                None
            }
        }
    }

    fn get_device_name(&self, device_path: &str) -> String {
        let daemon = self.imp().daemon.borrow();
        daemon
            .as_ref()
            .and_then(|d| d.get_device(device_path))
            .map(|d| d.display_name())
            .unwrap_or_else(|| "Unknown Device".to_string())
    }

    fn show_confirmation_dialog(&self, device_path: &str, passkey: u32) -> Option<bool> {
        let device_name = self.get_device_name(device_path);
        let dialog = BluetoothPairDialog::new_confirmation(&device_name, passkey);

        let window = self.root().and_downcast::<gtk4::Window>();
        dialog.set_transient_for(window.as_ref());

        // For now, use a simple blocking approach
        // In a real implementation, we'd need async dialog handling
        dialog.present();
        *self.imp().current_dialog.borrow_mut() = Some(dialog);

        // Return true to accept (simplified - real impl would be async)
        Some(true)
    }

    fn show_passkey_display(&self, device_path: &str, passkey: u32) {
        let device_name = self.get_device_name(device_path);
        let dialog = BluetoothPairDialog::new_display_passkey(&device_name, passkey);

        let window = self.root().and_downcast::<gtk4::Window>();
        dialog.set_transient_for(window.as_ref());
        dialog.present();

        *self.imp().current_dialog.borrow_mut() = Some(dialog);
    }

    fn show_pincode_display(&self, device_path: &str, pincode: &str) {
        let device_name = self.get_device_name(device_path);
        let dialog = BluetoothPairDialog::new_display_pincode(&device_name, pincode);

        let window = self.root().and_downcast::<gtk4::Window>();
        dialog.set_transient_for(window.as_ref());
        dialog.present();

        *self.imp().current_dialog.borrow_mut() = Some(dialog);
    }

    fn show_authorization_dialog(&self, device_path: &str) -> Option<bool> {
        let device_name = self.get_device_name(device_path);
        let dialog = BluetoothPairDialog::new_authorization(&device_name);

        let window = self.root().and_downcast::<gtk4::Window>();
        dialog.set_transient_for(window.as_ref());
        dialog.present();

        *self.imp().current_dialog.borrow_mut() = Some(dialog);

        // Return true to accept (simplified - real impl would be async)
        Some(true)
    }

    fn close_current_dialog(&self) {
        if let Some(dialog) = self.imp().current_dialog.borrow_mut().take() {
            dialog.close();
        }
    }
}

pub fn build_page() -> gtk4::Widget {
    BluetoothContent::new().upcast()
}
