use std::cell::RefCell;
use std::path::PathBuf;

use gdk4::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;

use crate::services::AccountsServiceUser;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/Users.ui")]
pub struct UsersContentImpl {
    #[template_child(id = "avatar")]
    pub avatar: TemplateChild<libadwaita::Avatar>,
    #[template_child(id = "avatar_button")]
    pub avatar_button: TemplateChild<gtk4::MenuButton>,
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

    pub accounts_user: RefCell<Option<AccountsServiceUser>>,
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
impl WidgetImpl for UsersContentImpl {
    fn realize(&self) {
        self.parent_realize();
        // Refresh data after widget is realized to ensure proper rendering
        self.obj().refresh_display();
    }
}
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
        self.setup_name_editing();
        self.setup_avatar_popover();
        self.load_accounts_service();
    }

    fn setup_name_editing(&self) {
        let imp = self.imp();

        imp.title_stack.set_visible_child_name("title");

        // Toggle button switches between label and entry
        imp.title_button.connect_toggled(clone!(
            #[weak(rename_to = obj)]
            self,
            move |btn| {
                obj.on_edit_toggled(btn.is_active());
            }
        ));

        // Enter key saves the name
        imp.title_entry.connect_activate(clone!(
            #[weak(rename_to = obj)]
            self,
            move |entry| {
                obj.save_name(&entry.text());
            }
        ));

        // Escape key cancels editing
        let controller = gtk4::EventControllerKey::new();
        controller.connect_key_pressed(clone!(
            #[weak(rename_to = obj)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gdk4::Key::Escape {
                    obj.cancel_edit();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        imp.title_entry.add_controller(controller);
    }

    fn setup_avatar_popover(&self) {
        let imp = self.imp();

        // Clear any existing children
        while let Some(child) = imp.popover_flowbox.first_child() {
            imp.popover_flowbox.remove(&child);
        }

        // Add file chooser button
        let add_button = gtk4::Button::from_icon_name("list-add-symbolic");
        add_button.add_css_class("circular");
        add_button.set_valign(gtk4::Align::Center);
        add_button.set_halign(gtk4::Align::Center);
        add_button.set_tooltip_text(Some("Choose custom image"));
        add_button.connect_clicked(clone!(
            #[weak(rename_to = obj)]
            self,
            move |_| {
                obj.open_avatar_file_chooser();
            }
        ));
        imp.popover_flowbox.append(&add_button);

        // Add predefined avatars from system directories
        let avatar_dirs = [
            "/usr/share/plasma/avatars",
            "/usr/share/pixmaps/faces",
        ];

        for dir in avatar_dirs {
            self.add_avatars_from_directory(dir, 0, 3);
        }
    }

    fn add_avatars_from_directory(&self, path: &str, depth: u32, max_depth: u32) {
        let Ok(dir) = std::fs::read_dir(path) else {
            return;
        };

        for entry in dir.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() && depth < max_depth {
                self.add_avatars_from_directory(&path.to_string_lossy(), depth + 1, max_depth);
            } else if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext = ext.to_string_lossy().to_lowercase();
                    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "svg" | "webp") {
                        self.add_avatar_item(&path);
                    }
                }
            }
        }
    }

    fn add_avatar_item(&self, path: &PathBuf) {
        let avatar = libadwaita::Avatar::new(64, None, true);
        let file = gio::File::for_path(path);
        let paintable = gtk4::IconPaintable::for_file(&file, 64, 1);
        avatar.set_custom_image(Some(&paintable));

        let button = gtk4::Button::new();
        button.set_child(Some(&avatar));
        button.add_css_class("flat");
        button.set_tooltip_text(path.file_name().and_then(|n| n.to_str()));

        let path_clone = path.clone();
        button.connect_clicked(clone!(
            #[weak(rename_to = obj)]
            self,
            move |_| {
                obj.set_avatar_from_path(&path_clone);
            }
        ));

        self.imp().popover_flowbox.append(&button);
    }

    fn load_accounts_service(&self) {
        // Load initial data from whoami while AccountsService connects
        self.load_fallback_data();

        // Try to connect to AccountsService
        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                match AccountsServiceUser::for_current_user().await {
                    Ok(user) => {
                        user.connect_changed(clone!(
                            #[weak]
                            obj,
                            move || {
                                obj.refresh_user_data();
                            }
                        ));

                        *obj.imp().accounts_user.borrow_mut() = Some(user);
                        obj.refresh_user_data();
                    }
                    Err(err) => {
                        log::warn!("Failed to connect to AccountsService: {err}");
                    }
                }
            }
        ));
    }

    fn load_fallback_data(&self) {
        let imp = self.imp();

        let username = whoami::username();
        let real_name = normalize_name(whoami::realname(), &username);

        imp.avatar.set_text(Some(&real_name));
        self.load_avatar_image(&username);

        imp.title.set_text(&real_name);
        imp.title_entry.set_text(&real_name);
        imp.subtitle.set_text(&username);
        imp.subtitle2
            .set_text(if is_root_user() { "Root User" } else { "Regular User" });
    }

    fn load_avatar_image(&self, username: &str) {
        let imp = self.imp();
        if let Some(path) = find_avatar_path(username) {
            let file = gio::File::for_path(&path);
            // Use explicit size (144) to ensure correct loading before widget is sized
            let size = imp.avatar.size();
            let size = if size > 0 { size } else { 144 };
            let paintable = gtk4::IconPaintable::for_file(&file, size, 1);
            imp.avatar.set_custom_image(Some(&paintable));
        }
    }

    /// Refresh the display after the widget is realized.
    fn refresh_display(&self) {
        let imp = self.imp();

        // If we have AccountsService data, use it; otherwise refresh fallback
        let has_accounts_user = imp.accounts_user.borrow().is_some();
        if has_accounts_user {
            self.refresh_user_data();
        } else {
            let username = whoami::username();
            self.load_avatar_image(&username);
        }

        // Force a redraw
        self.queue_draw();
    }

    fn refresh_user_data(&self) {
        let imp = self.imp();
        let binding = imp.accounts_user.borrow();
        let Some(user) = binding.as_ref() else {
            return;
        };

        let username = user.user_name().unwrap_or_else(whoami::username);
        let real_name = user
            .real_name()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| username.clone());

        imp.avatar.set_text(Some(&real_name));

        if let Some(icon_path) = user.icon_file().filter(|s| !s.is_empty()) {
            let file = gio::File::for_path(&icon_path);
            let paintable = gtk4::IconPaintable::for_file(&file, imp.avatar.size(), 1);
            imp.avatar.set_custom_image(Some(&paintable));
        }

        imp.title.set_text(&real_name);
        imp.title_entry.set_text(&real_name);
        imp.subtitle.set_text(&username);

        let user_type = if user.system_account() {
            "Root User"
        } else {
            "Regular User"
        };
        imp.subtitle2.set_text(user_type);
    }

    fn on_edit_toggled(&self, active: bool) {
        let imp = self.imp();
        let name = if active { "entry" } else { "title" };
        imp.title_stack.set_visible_child_name(name);

        if active {
            let current_name = imp.title.text();
            imp.title_entry.set_text(&current_name);
            imp.title_entry.grab_focus_without_selecting();
            imp.title_entry.set_position(-1);
        }
    }

    fn save_name(&self, new_name: &str) {
        let imp = self.imp();
        let new_name = new_name.trim().to_string();

        if new_name.is_empty() {
            self.cancel_edit();
            return;
        }

        imp.title.set_text(&new_name);
        imp.title_button.set_active(false);

        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let binding = obj.imp().accounts_user.borrow();
                if let Some(user) = binding.as_ref() {
                    if let Err(err) = user.set_real_name(&new_name).await {
                        log::warn!("Failed to set real name: {err}");
                    }
                }
            }
        ));
    }

    fn cancel_edit(&self) {
        let imp = self.imp();
        imp.title_button.set_active(false);
    }

    fn open_avatar_file_chooser(&self) {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Select Avatar");
        dialog.set_accept_label(Some("_Open"));

        let filter = gtk4::FileFilter::new();
        filter.add_mime_type("image/*");
        dialog.set_default_filter(Some(&filter));

        let window = self.root().and_downcast::<gtk4::Window>();

        dialog.open(
            window.as_ref(),
            None::<&gio::Cancellable>,
            clone!(
                #[weak(rename_to = obj)]
                self,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            obj.set_avatar_from_path(&path);
                        }
                    }
                }
            ),
        );
    }

    fn set_avatar_from_path(&self, path: &PathBuf) {
        let imp = self.imp();
        imp.popover.popdown();

        let path_str = path.to_string_lossy().to_string();

        glib::MainContext::default().spawn_local(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let home = glib::home_dir();
                let face_path = home.join(".face");

                // Load and scale image to 96x96
                let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file(&path_str) else {
                    log::warn!("Failed to load image: {path_str}");
                    return;
                };

                let scaled = pixbuf.scale_simple(96, 96, gdk_pixbuf::InterpType::Bilinear);
                let Some(scaled) = scaled else {
                    log::warn!("Failed to scale image");
                    return;
                };

                if let Err(err) = scaled.savev(&face_path, "png", &[]) {
                    log::warn!("Failed to save avatar: {err}");
                    return;
                }

                // Update the avatar display
                let file = gio::File::for_path(&face_path);
                let paintable =
                    gtk4::IconPaintable::for_file(&file, obj.imp().avatar.size(), 1);
                obj.imp().avatar.set_custom_image(Some(&paintable));

                // Update AccountsService
                let binding = obj.imp().accounts_user.borrow();
                if let Some(user) = binding.as_ref() {
                    let face_str = face_path.to_string_lossy().to_string();
                    if let Err(err) = user.set_icon_file(&face_str).await {
                        log::warn!("Failed to set icon file: {err}");
                    }
                }
            }
        ));
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
