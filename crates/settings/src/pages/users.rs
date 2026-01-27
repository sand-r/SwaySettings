use std::path::PathBuf;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/Users.ui")]
pub struct UsersContentImpl {
    #[template_child(id = "avatar")]
    pub avatar: TemplateChild<libadwaita::Avatar>,
    #[template_child(id = "title_stack")]
    pub title_stack: TemplateChild<gtk4::Stack>,
    #[template_child(id = "title_button")]
    pub title_button: TemplateChild<gtk4::ToggleButton>,
    #[template_child(id = "title")]
    pub title: TemplateChild<gtk4::Label>,
    #[template_child(id = "title_entry")]
    pub title_entry: TemplateChild<gtk4::Entry>,
    #[template_child(id = "subtitle")]
    pub subtitle: TemplateChild<gtk4::Label>,
    #[template_child(id = "subtitle2")]
    pub subtitle2: TemplateChild<gtk4::Label>,
    #[template_child(id = "popover")]
    pub popover: TemplateChild<gtk4::Popover>,
    #[template_child(id = "popover_flowbox")]
    pub popover_flowbox: TemplateChild<gtk4::FlowBox>,
}

#[glib::object_subclass]
impl ObjectSubclass for UsersContentImpl {
    const NAME: &'static str = "SwaySettingsUsersContent";
    type Type = UsersContent;
    type ParentType = libadwaita::Bin;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for UsersContentImpl {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        obj.setup();
    }
}
impl WidgetImpl for UsersContentImpl {}
impl BinImpl for UsersContentImpl {}

glib::wrapper! {
    pub struct UsersContent(ObjectSubclass<UsersContentImpl>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl UsersContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        let imp = self.imp();

        imp.title_button.set_visible(false);
        imp.title_button.set_sensitive(false);
        imp.title_stack.set_visible_child_name("title");
        imp.title_entry.set_sensitive(false);

        imp.popover.set_sensitive(false);
        imp.popover.set_visible(false);

        let username = whoami::username();
        let real_name = normalize_name(whoami::realname(), &username);

        imp.avatar.set_text(Some(&real_name));
        if let Some(path) = find_avatar_path(&username) {
            let file = gio::File::for_path(path);
            let paintable = gtk4::IconPaintable::for_file(&file, imp.avatar.size(), 1);
            imp.avatar.set_custom_image(Some(&paintable));
        }

        imp.title.set_text(&real_name);
        imp.title_entry.set_text(&real_name);

        imp.subtitle.set_text(&username);
        imp.subtitle2
            .set_text(if is_root_user() { "Root User" } else { "Regular User" });
    }
}

pub fn build_page() -> gtk4::Widget {
    UsersContent::new().upcast()
}

fn normalize_name(real_name: String, username: &str) -> String {
    let trimmed = real_name.trim();
    if trimmed.is_empty() {
        username.to_string()
    } else {
        trimmed.to_string()
    }
}

fn is_root_user() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn find_avatar_path(username: &str) -> Option<PathBuf> {
    let home = glib::home_dir();
    let face = home.join(".face");
    if face.exists() {
        return Some(face);
    }

    let accounts = PathBuf::from("/var/lib/AccountsService/icons").join(username);
    if accounts.exists() {
        return Some(accounts);
    }

    None
}
