use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/LockerWindow.ui")]
pub struct LockerWindowImpl {
    #[template_child(id = "entry")]
    pub entry: TemplateChild<gtk4::Entry>,
    #[template_child(id = "time_label")]
    pub time_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "date_label")]
    pub date_label: TemplateChild<gtk4::Label>,
}

#[glib::object_subclass]
impl ObjectSubclass for LockerWindowImpl {
    const NAME: &'static str = "SwaySettingsLockerWindow";
    type Type = LockerWindow;
    type ParentType = libadwaita::ApplicationWindow;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for LockerWindowImpl {}
impl WidgetImpl for LockerWindowImpl {}
impl WindowImpl for LockerWindowImpl {}
impl ApplicationWindowImpl for LockerWindowImpl {}
impl AdwApplicationWindowImpl for LockerWindowImpl {}

glib::wrapper! {
    pub struct LockerWindow(ObjectSubclass<LockerWindowImpl>)
        @extends libadwaita::ApplicationWindow, gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk4::Accessible, gtk4::Buildable,
                    gtk4::ConstraintTarget, gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl LockerWindow {
    pub fn new(app: &libadwaita::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    pub fn entry(&self) -> gtk4::Entry {
        self.imp().entry.get()
    }

    pub fn time_label(&self) -> gtk4::Label {
        self.imp().time_label.get()
    }

    pub fn date_label(&self) -> gtk4::Label {
        self.imp().date_label.get()
    }
}
