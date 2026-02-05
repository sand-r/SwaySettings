use std::cell::{Cell, RefCell};

use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::services::sway_input::{
    apply_setting, load_first_device, InputDeviceSettings, InputKind,
};

const ACCEL_PROFILE_OPTIONS: [&str; 2] = ["Adaptive", "Flat"];

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/MouseContent.ui")]
pub struct MouseContentImpl {
    #[template_child(id = "stack")]
    pub stack: TemplateChild<gtk4::Stack>,
    #[template_child(id = "natural_scroll_row")]
    pub natural_scroll_row: TemplateChild<libadwaita::SwitchRow>,
    #[template_child(id = "scroll_factor_scale")]
    pub scroll_factor_scale: TemplateChild<gtk4::Scale>,
    #[template_child(id = "accel_profile_row")]
    pub accel_profile_row: TemplateChild<libadwaita::ComboRow>,
    #[template_child(id = "pointer_accel_scale")]
    pub pointer_accel_scale: TemplateChild<gtk4::Scale>,

    pub device: RefCell<Option<InputDeviceSettings>>,
    pub updating_ui: Cell<bool>,
}

#[glib::object_subclass]
impl ObjectSubclass for MouseContentImpl {
    const NAME: &'static str = "SwaySettingsMouseContent";
    type Type = MouseContent;
    type ParentType = libadwaita::Bin;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for MouseContentImpl {
    fn constructed(&self) {
        self.parent_constructed();
        self.obj().setup();
    }
}

impl WidgetImpl for MouseContentImpl {}
impl BinImpl for MouseContentImpl {}

glib::wrapper! {
    pub struct MouseContent(ObjectSubclass<MouseContentImpl>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl MouseContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        let imp = self.imp();
        self.setup_combo_row(&imp.accel_profile_row, &ACCEL_PROFILE_OPTIONS);

        imp.scroll_factor_scale
            .add_mark(1.0, gtk4::PositionType::Bottom, None);
        imp.pointer_accel_scale
            .add_mark(0.0, gtk4::PositionType::Bottom, None);

        self.connect_signals();
        self.reload_device_async();
    }

    fn setup_combo_row(&self, row: &libadwaita::ComboRow, options: &[&str]) {
        let model = gio::ListStore::new::<gtk4::StringObject>();
        for option in options {
            model.append(&gtk4::StringObject::new(option));
        }
        row.set_model(Some(&model));
    }

    fn connect_signals(&self) {
        let imp = self.imp();

        imp.natural_scroll_row.connect_active_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let imp = this.imp();
                if imp.updating_ui.get() || imp.device.borrow().is_none() {
                    return;
                }

                let value = row.is_active();
                this.write_setting("natural_scroll", if value { "enabled" } else { "disabled" });
                if let Some(device) = imp.device.borrow_mut().as_mut() {
                    device.natural_scroll = value;
                }
            }
        ));

        imp.scroll_factor_scale.connect_value_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |scale| {
                let imp = this.imp();
                if imp.updating_ui.get() || imp.device.borrow().is_none() {
                    return;
                }

                let value = scale.value();
                this.write_setting("scroll_factor", &value.to_string());
                if let Some(device) = imp.device.borrow_mut().as_mut() {
                    device.scroll_factor = value;
                }
            }
        ));

        imp.accel_profile_row.connect_selected_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let imp = this.imp();
                if imp.updating_ui.get() || imp.device.borrow().is_none() {
                    return;
                }

                let profile = match accel_profile_from_index(row.selected()) {
                    Some(profile) => profile,
                    None => return,
                };
                this.write_setting("accel_profile", profile);
                if let Some(device) = imp.device.borrow_mut().as_mut() {
                    device.accel_profile = profile.to_string();
                }
            }
        ));

        imp.pointer_accel_scale.connect_value_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |scale| {
                let imp = this.imp();
                if imp.updating_ui.get() || imp.device.borrow().is_none() {
                    return;
                }

                let value = scale.value();
                this.write_setting("pointer_accel", &value.to_string());
                if let Some(device) = imp.device.borrow_mut().as_mut() {
                    device.accel_speed = value;
                }
            }
        ));
    }

    fn reload_device_async(&self) {
        let context = glib::MainContext::default();
        let weak = glib::SendWeakRef::from(self.downgrade());
        std::thread::spawn(move || {
            let device = match load_first_device(InputKind::Pointer) {
                Ok(device) => device,
                Err(err) => {
                    log::warn!("Failed to read pointer settings from sway: {err}");
                    None
                }
            };

            context.invoke(move || {
                if let Some(this) = weak.upgrade() {
                    this.load_device(device);
                }
            });
        });
    }

    fn load_device(&self, device: Option<InputDeviceSettings>) {
        let imp = self.imp();
        imp.updating_ui.set(true);

        if let Some(device) = device {
            imp.stack.set_visible_child_name("page");
            imp.natural_scroll_row.set_active(device.natural_scroll);
            imp.scroll_factor_scale.set_value(device.scroll_factor);
            imp.accel_profile_row
                .set_selected(accel_profile_to_index(&device.accel_profile));
            imp.pointer_accel_scale.set_value(device.accel_speed);
            *imp.device.borrow_mut() = Some(device);
        } else {
            *imp.device.borrow_mut() = None;
            imp.stack.set_visible_child_name("placeholder");
        }

        imp.updating_ui.set(false);
    }

    fn write_setting(&self, setting: &str, value: &str) {
        if let Err(err) = apply_setting(InputKind::Pointer, setting, value) {
            log::warn!("Failed to apply pointer setting {setting}: {err}");
            self.reload_device_async();
        }
    }
}

pub fn build_page() -> gtk4::Widget {
    MouseContent::new().upcast()
}

fn accel_profile_from_index(index: u32) -> Option<&'static str> {
    match index {
        0 => Some("adaptive"),
        1 => Some("flat"),
        _ => None,
    }
}

fn accel_profile_to_index(value: &str) -> u32 {
    match value {
        "flat" => 1,
        _ => 0,
    }
}
