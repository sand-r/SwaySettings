use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use glib::clone;
use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;
use upower_dbus::{BatteryState, BatteryType, DeviceProxyBlocking, UPowerProxyBlocking};
use zbus::blocking::Connection;

use swaysettings_core::constants::SETTINGS_POWER_AUTO_POWER_SAVER;

const POWER_PROFILES_BUS: &str = "org.freedesktop.UPower.PowerProfiles";
const POWER_PROFILES_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
const POWER_PROFILES_IFACE: &str = "org.freedesktop.UPower.PowerProfiles";

#[derive(Debug, Clone)]
struct BatteryInfo {
    percent: f64,
    icon_name: String,
    state: BatteryState,
    is_present: bool,
}

#[derive(Debug, Clone)]
struct DeviceInfo {
    title: String,
    percent: f64,
    icon_name: String,
}

#[derive(Debug, Clone)]
enum PowerEvent {
    Battery(Option<BatteryInfo>),
    Devices(Vec<DeviceInfo>),
    Unavailable,
}

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/PowerPageContent.ui")]
    pub struct PowerContent {
        #[template_child(id = "stack")]
        pub stack: TemplateChild<gtk4::Stack>,
        #[template_child(id = "main_box")]
        pub main_box: TemplateChild<gtk4::Box>,

        #[template_child(id = "battery_group")]
        pub battery_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "battery_list")]
        pub battery_list: TemplateChild<gtk4::ListBox>,

        #[template_child(id = "battery_history_row")]
        pub battery_history_row: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "history_graph")]
        pub history_graph: TemplateChild<gtk4::Label>,

        #[template_child(id = "devices_group")]
        pub devices_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "devices_list")]
        pub devices_list: TemplateChild<gtk4::ListBox>,

        #[template_child(id = "battery_health_group")]
        pub battery_health_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "health_status_label")]
        pub health_status_label: TemplateChild<gtk4::Label>,
        #[template_child(id = "health_capacity_label")]
        pub health_capacity_label: TemplateChild<gtk4::Label>,
        #[template_child(id = "health_cycles_row")]
        pub health_cycles_row: TemplateChild<libadwaita::ActionRow>,
        #[template_child(id = "health_cycles_label")]
        pub health_cycles_label: TemplateChild<gtk4::Label>,
        #[template_child(id = "threshold_toggle")]
        pub threshold_toggle: TemplateChild<libadwaita::SwitchRow>,

        #[template_child(id = "options_group")]
        pub options_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "auto_power_saver_row")]
        pub auto_power_saver_row: TemplateChild<libadwaita::SwitchRow>,

        #[template_child(id = "power_mode_group")]
        pub power_mode_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "modes_listbox")]
        pub modes_listbox: TemplateChild<gtk4::ListBox>,
        #[template_child(id = "power_info_listbox")]
        pub power_info_listbox: TemplateChild<gtk4::ListBox>,

        pub settings: OnceCell<gio::Settings>,
        pub profiles_proxy: RefCell<Option<gio::DBusProxy>>,
        pub profile_buttons: RefCell<HashMap<String, gtk4::CheckButton>>,
        pub updating_ui: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerContent {
        const NAME: &'static str = "SwaySettingsPowerPageContent";
        type Type = super::PowerContent;
        type ParentType = libadwaita::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PowerContent {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup();
        }
    }

    impl WidgetImpl for PowerContent {}
    impl BinImpl for PowerContent {}
}

glib::wrapper! {
    pub struct PowerContent(ObjectSubclass<imp::PowerContent>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

pub fn build_page() -> gtk4::Widget {
    PowerContent::new().upcast()
}

impl PowerContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        self.setup_settings();
        self.setup_power_profiles();
        self.setup_upower_poll();
    }

    fn setup_settings(&self) {
        let settings = gio::Settings::new("org.erikreider.swaysettings");
        let imp = self.imp();
        imp.settings.set(settings.clone()).ok();
        settings
            .bind::<libadwaita::SwitchRow>(
                SETTINGS_POWER_AUTO_POWER_SAVER,
                imp.auto_power_saver_row.as_ref(),
                "active",
            )
            .build();
    }

    fn setup_power_profiles(&self) {
        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let proxy = gio::DBusProxy::for_bus_future(
                    gio::BusType::System,
                    gio::DBusProxyFlags::NONE,
                    None::<&gio::DBusInterfaceInfo>,
                    POWER_PROFILES_BUS,
                    POWER_PROFILES_PATH,
                    POWER_PROFILES_IFACE,
                )
                .await;

                let imp = obj.imp();
                match proxy {
                    Ok(proxy) => {
                        let proxy_clone = proxy.clone();
                        imp.profiles_proxy.replace(Some(proxy_clone));

                        let obj_weak = obj.downgrade();
                        proxy.connect_local("g-properties-changed", false, move |_| {
                            if let Some(obj) = obj_weak.upgrade() {
                                obj.refresh_power_profiles();
                            }
                            None
                        });

                        obj.refresh_power_profiles();
                    }
                    Err(err) => {
                        log::warn!("Failed to connect to power-profiles-daemon: {}", err);
                        imp.power_mode_group.set_visible(false);
                        obj.update_stack_visibility();
                    }
                }
            }
        ));
    }

    fn refresh_power_profiles(&self) {
        let imp = self.imp();
        let Some(proxy) = imp.profiles_proxy.borrow().clone() else {
            imp.power_mode_group.set_visible(false);
            self.update_stack_visibility();
            return;
        };

        let active_profile = proxy
            .cached_property("ActiveProfile")
            .and_then(|v| v.get::<String>())
            .unwrap_or_default();

        let mut profiles: Vec<String> = Vec::new();
        if let Some(list) = proxy.cached_property("Profiles") {
            for profile in list.iter() {
                let dict = glib::VariantDict::new(Some(&profile));
                if let Ok(Some(name)) = dict.lookup::<String>("Profile") {
                    profiles.push(name);
                }
            }
        }
        profiles.sort();

        imp.updating_ui.set(true);
        self.clear_listbox(&imp.modes_listbox);
        imp.profile_buttons.borrow_mut().clear();

        let mut group: Option<gtk4::CheckButton> = None;
        for profile in profiles.iter() {
            let (title, subtitle, _icon_name) = profile_info(profile);
            let row = libadwaita::ActionRow::new();
            row.set_title(title);
            row.set_subtitle(subtitle);
            row.set_selectable(false);
            row.set_activatable(true);

            let button = gtk4::CheckButton::new();
            if let Some(group_button) = group.as_ref() {
                button.set_group(Some(group_button));
            } else {
                group = Some(button.clone());
            }
            button.set_valign(gtk4::Align::Center);
            button.set_active(profile == &active_profile);
            row.set_activatable_widget(Some(&button));
            row.add_prefix(&button);

            row.add_css_class("power-profile");
            match profile.as_str() {
                "power-saver" => row.add_css_class("low-power"),
                "performance" => row.add_css_class("performance"),
                _ => {}
            }

            let profile_name = profile.clone();
            row.connect_activated(clone!(
                #[weak(rename_to = obj)]
                self,
                move |_| {
                    obj.set_active_profile(&profile_name);
                }
            ));

            let profile_name_toggle = profile.clone();
            button.connect_toggled(clone!(
                #[weak(rename_to = obj)]
                self,
                move |btn| {
                    let imp = obj.imp();
                    if imp.updating_ui.get() {
                        return;
                    }
                    if btn.is_active() {
                        obj.set_active_profile(&profile_name_toggle);
                    }
                }
            ));

            imp.profile_buttons
                .borrow_mut()
                .insert(profile.clone(), button);
            imp.modes_listbox.append(&row);
        }
        imp.updating_ui.set(false);

        // Power profile holds & degraded info
        self.refresh_power_info(&proxy);

        imp.power_mode_group.set_visible(!profiles.is_empty());
        self.update_stack_visibility();
    }

    fn refresh_power_info(&self, proxy: &gio::DBusProxy) {
        let imp = self.imp();
        self.clear_listbox(&imp.power_info_listbox);

        if let Some(holds) = proxy.cached_property("ActiveProfileHolds") {
            for hold in holds.iter() {
                let dict = glib::VariantDict::new(Some(&hold));
                let profile = dict.lookup::<String>("Profile").ok().flatten();
                let reason = dict.lookup::<String>("Reason").ok().flatten();
                if let (Some(profile), Some(reason)) = (profile, reason) {
                    let icon = match profile.as_str() {
                        "performance" => "power-profile-performance-symbolic",
                        "power-saver" => "power-profile-power-saver-symbolic",
                        _ => "power-profile-balanced-symbolic",
                    };
                    let row = info_row(icon, &reason);
                    imp.power_info_listbox.append(&row);
                }
            }
        }

        if let Some(degraded) = proxy
            .cached_property("PerformanceDegraded")
            .and_then(|v| v.get::<String>())
        {
            if !degraded.is_empty() {
                let (icon, text) = match degraded.as_str() {
                    "lap-detected" => (
                        "info-outline-symbolic",
                        "Lap detected: performance mode temporarily unavailable. Move the device to a stable surface to restore.",
                    ),
                    "high-operating-temperature" => (
                        "thermometer-symbolic",
                        "Performance mode temporarily disabled.",
                    ),
                    _ => ("warning-outline-symbolic", "Performance mode temporarily disabled."),
                };
                let row = info_row(icon, text);
                imp.power_info_listbox.append(&row);
            }
        }

        imp.power_info_listbox
            .set_visible(imp.power_info_listbox.first_child().is_some());
    }

    fn set_active_profile(&self, profile: &str) {
        let imp = self.imp();
        let Some(proxy) = imp.profiles_proxy.borrow().clone() else {
            return;
        };
        let profile = profile.to_string();
        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let value = glib::Variant::from(profile.as_str());
                let result = proxy
                    .call_future(
                        "org.freedesktop.DBus.Properties.Set",
                        Some(&(POWER_PROFILES_IFACE, "ActiveProfile", value).to_variant()),
                        gio::DBusCallFlags::NONE,
                        -1,
                    )
                    .await;
                if let Err(err) = result {
                    log::warn!("Failed to set power profile: {}", err);
                }
                obj.refresh_power_profiles();
            }
        ));
    }

    fn setup_upower_poll(&self) {
        let context = glib::MainContext::default();
        let obj_weak = glib::SendWeakRef::from(self.downgrade());

        std::thread::spawn(move || {
            let conn = Connection::system();
            let Ok(conn) = conn else {
                let obj_weak = obj_weak.clone();
                context.invoke(move || {
                    if let Some(obj) = obj_weak.upgrade() {
                        obj.handle_power_event(PowerEvent::Unavailable);
                    }
                });
                return;
            };
            let upower = UPowerProxyBlocking::new(&conn);
            let Ok(upower) = upower else {
                let obj_weak = obj_weak.clone();
                context.invoke(move || {
                    if let Some(obj) = obj_weak.upgrade() {
                        obj.handle_power_event(PowerEvent::Unavailable);
                    }
                });
                return;
            };

            loop {
                let battery = fetch_battery_info(&conn, &upower);
                let weak = obj_weak.clone();
                context.invoke(move || {
                    if let Some(obj) = weak.upgrade() {
                        obj.handle_power_event(PowerEvent::Battery(battery));
                    }
                });

                let devices = fetch_devices(&conn, &upower);
                let weak = obj_weak.clone();
                context.invoke(move || {
                    if let Some(obj) = weak.upgrade() {
                        obj.handle_power_event(PowerEvent::Devices(devices));
                    }
                });

                std::thread::sleep(Duration::from_secs(10));
            }
        });
    }

    fn handle_power_event(&self, event: PowerEvent) {
        let imp = self.imp();
        match event {
            PowerEvent::Unavailable => {
                imp.battery_group.set_visible(false);
                imp.devices_group.set_visible(false);
                imp.options_group.set_visible(false);
                imp.battery_health_group.set_visible(false);
                imp.battery_history_row.set_visible(false);
                self.update_stack_visibility();
            }
            PowerEvent::Battery(info) => {
                self.clear_listbox(&imp.battery_list);
                if let Some(info) = info {
                    if info.is_present {
                        let details = battery_state_label(info.state);
                        let row = battery_level_row(&info, details);
                        imp.battery_list.append(&row);
                        imp.battery_group.set_visible(true);
                        imp.options_group.set_visible(true);
                    } else {
                        imp.battery_group.set_visible(false);
                        imp.options_group.set_visible(false);
                    }
                } else {
                    imp.battery_group.set_visible(false);
                    imp.options_group.set_visible(false);
                }
                self.update_stack_visibility();
            }
            PowerEvent::Devices(devices) => {
                self.clear_listbox(&imp.devices_list);
                for device in devices {
                    let row = device_row(&device);
                    imp.devices_list.append(&row);
                }
                imp.devices_group
                    .set_visible(imp.devices_list.first_child().is_some());
                self.update_stack_visibility();
            }
        }
    }

    fn clear_listbox(&self, listbox: &gtk4::ListBox) {
        while let Some(child) = listbox.first_child() {
            listbox.remove(&child);
        }
    }

    fn update_stack_visibility(&self) {
        let imp = self.imp();
        let mut any_visible = false;
        let mut child = imp.main_box.first_child();
        while let Some(widget) = child {
            if widget.is_visible() {
                any_visible = true;
                break;
            }
            child = widget.next_sibling();
        }
        let page = if any_visible { "page" } else { "placeholder" };
        imp.stack.set_visible_child_name(page);
    }
}

fn fetch_battery_info(
    conn: &Connection,
    upower: &UPowerProxyBlocking,
) -> Option<BatteryInfo> {
    let device = upower.get_display_device().ok()?;
    let percent = device.percentage().ok().unwrap_or(0.0);
    let icon_name = device.icon_name().ok().unwrap_or_else(|| "battery-symbolic".to_string());
    let state = device.state().ok().unwrap_or(BatteryState::Unknown);
    let is_present = device.is_present().ok().unwrap_or(false);
    let _ = conn;
    Some(BatteryInfo {
        percent,
        icon_name,
        state,
        is_present,
    })
}

fn fetch_devices(conn: &Connection, upower: &UPowerProxyBlocking) -> Vec<DeviceInfo> {
    let mut devices_info = Vec::new();
    let Ok(devices) = upower.enumerate_devices() else {
        return devices_info;
    };

    for path in devices {
        let proxy = DeviceProxyBlocking::builder(conn)
            .path(path)
            .and_then(|builder| builder.build())
            .ok();
        let Some(proxy) = proxy else { continue };
        if proxy.power_supply().ok().unwrap_or(false) {
            continue;
        }
        if !proxy.is_present().ok().unwrap_or(true) {
            continue;
        }

        let percent = proxy.percentage().ok().unwrap_or(0.0);
        let title = device_title(&proxy);
        let icon_name = proxy
            .icon_name()
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| device_icon_from_type(proxy.type_().ok().unwrap_or(BatteryType::Unknown)).to_string());

        devices_info.push(DeviceInfo {
            title,
            percent,
            icon_name,
        });
    }

    devices_info
}

fn device_title(proxy: &DeviceProxyBlocking) -> String {
    let model = proxy.model().ok().unwrap_or_default();
    if !model.is_empty() {
        return model;
    }
    let vendor = proxy.vendor().ok().unwrap_or_default();
    if !vendor.is_empty() {
        return vendor;
    }
    "Device".to_string()
}

fn profile_info(profile: &str) -> (&'static str, &'static str, &'static str) {
    match profile {
        "power-saver" => (
            "Power Saver",
            "Reduced performance and power usage",
            "power-profile-power-saver-symbolic",
        ),
        "performance" => (
            "Performance",
            "High performance and power usage",
            "power-profile-performance-symbolic",
        ),
        "balanced" => (
            "Balanced",
            "Standard performance and power usage",
            "power-profile-balanced-symbolic",
        ),
        _ => ("Unknown", "Unknown profile", "power-profile-balanced-symbolic"),
    }
}

fn info_row(icon_name: &str, text: &str) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::new();
    row.set_title(text);
    row.set_selectable(false);
    row.set_activatable(false);
    row.add_css_class("power-profile-info-row");

    let icon = gtk4::Image::from_icon_name(icon_name);
    icon.set_pixel_size(24);
    row.add_prefix(&icon);
    row
}

fn battery_level_row(info: &BatteryInfo, details: Option<&str>) -> gtk4::ListBoxRow {
    build_level_row(
        "Battery Level",
        info.percent,
        Some(info.icon_name.as_str()),
        details,
    )
}

fn device_row(info: &DeviceInfo) -> gtk4::ListBoxRow {
    build_level_row(&info.title, info.percent, Some(info.icon_name.as_str()), None)
}

fn build_level_row(
    title: &str,
    percent: f64,
    icon_name: Option<&str>,
    details: Option<&str>,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    outer.set_valign(gtk4::Align::Center);
    outer.set_margin_start(12);
    outer.set_margin_end(12);
    outer.set_margin_top(16);
    outer.set_margin_bottom(14);

    let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);

    let name_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let name_label = gtk4::Label::new(Some(title));
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    name_label.set_xalign(0.0);
    name_box.append(&name_label);

    if let Some(icon_name) = icon_name {
        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_valign(gtk4::Align::Center);
        icon.add_css_class("dim-label");
        name_box.append(&icon);
    }

    let percent_label = gtk4::Label::new(Some(&format!("{:.0}%", percent)));
    percent_label.set_halign(gtk4::Align::End);
    percent_label.add_css_class("dim-label");

    let levelbar = gtk4::LevelBar::new();
    levelbar.set_min_value(0.0);
    levelbar.set_max_value(1.0);
    levelbar.set_value((percent / 100.0).clamp(0.0, 1.0));
    levelbar.set_hexpand(true);
    levelbar.set_halign(gtk4::Align::Fill);
    levelbar.set_valign(gtk4::Align::Center);
    levelbar.add_offset_value("warning-battery-offset", 0.03);
    levelbar.add_offset_value("low-battery-offset", 0.1);
    levelbar.add_offset_value("high-battery-offset", 1.0);

    top.append(&name_box);
    top.append(&percent_label);
    top.append(&levelbar);
    outer.append(&top);

    if let Some(details) = details {
        let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        bottom.set_hexpand(true);

        let details_label = gtk4::Label::new(Some(details));
        details_label.set_hexpand(true);
        details_label.set_xalign(0.0);
        details_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        details_label.add_css_class("dim-label");
        bottom.append(&details_label);

        outer.append(&bottom);
    }

    row.set_child(Some(&outer));
    row
}

fn battery_state_label(state: BatteryState) -> Option<&'static str> {
    match state {
        BatteryState::Charging => Some("Charging"),
        BatteryState::Discharging | BatteryState::PendingDischarge => Some("Discharging"),
        BatteryState::FullyCharged => Some("Full"),
        BatteryState::Empty => Some("Empty"),
        BatteryState::PendingCharge => Some("Charge limit reached"),
        BatteryState::Unknown => None,
    }
}

fn device_icon_from_type(kind: BatteryType) -> &'static str {
    match kind {
        BatteryType::LinePower => "ac-adapter-symbolic",
        BatteryType::Battery => "battery-symbolic",
        BatteryType::Ups => "uninterruptible-power-supply-symbolic",
        BatteryType::Monitor => "video-display-symbolic",
        BatteryType::Mouse => "input-mouse-symbolic",
        BatteryType::Keyboard => "input-keyboard-symbolic",
        BatteryType::Pda => "pda-symbolic",
        BatteryType::Phone => "phone-symbolic",
        BatteryType::Unknown => "battery-symbolic",
    }
}
