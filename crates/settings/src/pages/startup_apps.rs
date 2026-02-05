use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gio::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

#[derive(Clone, Debug)]
struct StartupAppEntry {
    path: PathBuf,
    name: String,
    command: String,
    icon: Option<String>,
}

#[derive(Clone, Debug)]
struct AppChoice {
    id: String,
    name: String,
    command: String,
    icon: Option<String>,
}

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/StartupAppsContent.ui")]
    pub struct StartupAppsContent {
        #[template_child(id = "stack")]
        pub stack: TemplateChild<gtk4::Stack>,
        #[template_child(id = "apps_group")]
        pub apps_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child(id = "placeholder_add_button")]
        pub placeholder_add_button: TemplateChild<gtk4::Button>,
        #[template_child(id = "add_row")]
        pub add_row: TemplateChild<libadwaita::ActionRow>,

        pub rows: RefCell<Vec<libadwaita::ActionRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StartupAppsContent {
        const NAME: &'static str = "SwaySettingsStartupAppsContent";
        type Type = super::StartupAppsContent;
        type ParentType = libadwaita::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for StartupAppsContent {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for StartupAppsContent {}
    impl BinImpl for StartupAppsContent {}
}

glib::wrapper! {
    pub struct StartupAppsContent(ObjectSubclass<imp::StartupAppsContent>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

pub fn build_page() -> gtk4::Widget {
    StartupAppsContent::new().upcast()
}

impl StartupAppsContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        let imp = self.imp();

        imp.placeholder_add_button.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.open_app_chooser();
            }
        ));

        imp.add_row.connect_activated(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.open_app_chooser();
            }
        ));

        self.refresh_rows();
    }

    fn refresh_rows(&self) {
        let apps = load_startup_apps();
        let imp = self.imp();

        let existing_rows = std::mem::take(&mut *imp.rows.borrow_mut());
        for row in existing_rows {
            imp.apps_group.remove(&row);
        }

        for app in &apps {
            let row = self.create_startup_row(app);
            imp.apps_group.add(&row);
            imp.rows.borrow_mut().push(row);
        }

        imp.stack.set_visible_child_name(if apps.is_empty() {
            "placeholder"
        } else {
            "page"
        });
    }

    fn create_startup_row(&self, app: &StartupAppEntry) -> libadwaita::ActionRow {
        let row = libadwaita::ActionRow::builder()
            .title(&app.name)
            .subtitle(&app.command)
            .activatable(false)
            .build();

        let icon = gtk4::Image::new();
        icon.set_pixel_size(32);
        if let Some(icon_name) = app.icon.as_deref() {
            if Path::new(icon_name).exists() {
                icon.set_from_file(Some(icon_name));
            } else {
                icon.set_icon_name(Some(icon_name));
            }
        } else {
            icon.set_icon_name(Some("application-x-executable-symbolic"));
        }
        row.add_prefix(&icon);

        let remove_button = gtk4::Button::builder()
            .icon_name("edit-delete-symbolic")
            .tooltip_text("Remove from startup")
            .valign(gtk4::Align::Center)
            .build();
        remove_button.add_css_class("flat");

        let path = app.path.clone();
        remove_button.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                if let Err(err) = std::fs::remove_file(&path) {
                    log::warn!("Failed to remove startup app '{}': {err}", path.display());
                }
                this.refresh_rows();
            }
        ));

        row.add_suffix(&remove_button);
        row
    }

    fn open_app_chooser(&self) {
        let mut choices = list_available_apps();
        if choices.is_empty() {
            return;
        }

        choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let dialog = gtk4::Dialog::builder()
            .title("Choose Application")
            .modal(true)
            .default_width(560)
            .default_height(520)
            .build();

        if let Some(window) = self
            .root()
            .and_then(|root| root.downcast::<gtk4::Window>().ok())
        {
            dialog.set_transient_for(Some(&window));
        }

        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        dialog.add_button("Add", gtk4::ResponseType::Accept);
        dialog.set_default_response(gtk4::ResponseType::Accept);

        let content_area = dialog.content_area();

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vexpand(true)
            .min_content_height(360)
            .build();

        let list = gtk4::ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        list.add_css_class("boxed-list");

        for choice in &choices {
            let row = libadwaita::ActionRow::builder()
                .title(&choice.name)
                .subtitle(&choice.command)
                .activatable(true)
                .build();

            let icon = gtk4::Image::new();
            icon.set_pixel_size(20);
            if let Some(icon_name) = choice.icon.as_deref() {
                if Path::new(icon_name).exists() {
                    icon.set_from_file(Some(icon_name));
                } else {
                    icon.set_icon_name(Some(icon_name));
                }
            } else {
                icon.set_icon_name(Some("application-x-executable-symbolic"));
            }

            row.add_prefix(&icon);
            list.append(&row);
        }

        if let Some(row) = list.row_at_index(0) {
            list.select_row(Some(&row));
        }

        let dialog_for_activate = dialog.clone();
        list.connect_row_activated(move |_, _| {
            dialog_for_activate.response(gtk4::ResponseType::Accept);
        });

        scrolled.set_child(Some(&list));
        content_area.append(&scrolled);

        dialog.connect_response(clone!(
            #[weak(rename_to = this)]
            self,
            #[strong]
            list,
            #[strong]
            choices,
            move |dialog, response| {
                if response == gtk4::ResponseType::Accept {
                    let selected_index = list.selected_row().map(|row| row.index() as usize);
                    if let Some(index) = selected_index {
                        if let Some(choice) = choices.get(index) {
                            if let Err(err) = create_startup_entry(choice) {
                                log::warn!(
                                    "Failed to create startup entry for '{}': {err}",
                                    choice.name
                                );
                            }
                            this.refresh_rows();
                        }
                    }
                }

                dialog.close();
            }
        ));

        dialog.present();
    }
}

fn load_startup_apps() -> Vec<StartupAppEntry> {
    let mut apps = Vec::new();
    let autostart_dir = autostart_dir();

    if let Err(err) = std::fs::create_dir_all(&autostart_dir) {
        log::warn!(
            "Failed to create autostart directory '{}': {err}",
            autostart_dir.display()
        );
        return apps;
    }

    let Ok(read_dir) = std::fs::read_dir(&autostart_dir) else {
        return apps;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
            continue;
        }

        if let Some(app) = parse_startup_desktop_file(&path) {
            apps.push(app);
        }
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn parse_startup_desktop_file(path: &Path) -> Option<StartupAppEntry> {
    let key_file = glib::KeyFile::new();
    if key_file
        .load_from_file(path, glib::KeyFileFlags::NONE)
        .is_err()
    {
        return None;
    }

    let enabled = key_file
        .boolean("Desktop Entry", "X-GNOME-Autostart-enabled")
        .unwrap_or(true);
    if !enabled {
        return None;
    }

    let name = key_file
        .string("Desktop Entry", "Name")
        .ok()
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })?;

    let command = key_file
        .string("Desktop Entry", "Exec")
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_default();

    let icon = key_file
        .string("Desktop Entry", "Icon")
        .ok()
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty());

    Some(StartupAppEntry {
        path: path.to_path_buf(),
        name,
        command,
        icon,
    })
}

fn list_available_apps() -> Vec<AppChoice> {
    let mut choices = Vec::new();

    for app in gio::AppInfo::all() {
        if !app.should_show() {
            continue;
        }

        let name = app.display_name().to_string();
        if name.trim().is_empty() {
            continue;
        }

        let command = app
            .commandline()
            .map(|value| value.to_string_lossy().to_string())
            .filter(|value: &String| !value.trim().is_empty())
            .or_else(|| Some(app.executable().to_string_lossy().to_string()))
            .unwrap_or_default();

        if command.trim().is_empty() {
            continue;
        }

        let id = app
            .id()
            .map(|value| value.to_string())
            .unwrap_or_else(|| sanitize_filename(&name));

        let icon = app
            .icon()
            .and_then(|icon| icon.to_string())
            .map(|value| value.to_string())
            .filter(|value| !value.trim().is_empty());

        choices.push(AppChoice {
            id,
            name,
            command,
            icon,
        });
    }

    choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    choices.dedup_by(|a, b| a.id == b.id);
    choices
}

fn create_startup_entry(choice: &AppChoice) -> anyhow::Result<()> {
    let autostart_dir = autostart_dir();
    std::fs::create_dir_all(&autostart_dir)?;

    let mut file_name = if choice.id.trim().is_empty() {
        sanitize_filename(&choice.name)
    } else {
        choice.id.clone()
    };

    if !file_name.ends_with(".desktop") {
        file_name.push_str(".desktop");
    }

    let file_path = autostart_dir.join(file_name);

    let key_file = glib::KeyFile::new();
    key_file.set_string("Desktop Entry", "Type", "Application");
    key_file.set_string("Desktop Entry", "Version", "1.0");
    key_file.set_string("Desktop Entry", "Name", &choice.name);
    key_file.set_string("Desktop Entry", "Exec", &choice.command);
    if let Some(icon) = choice.icon.as_deref() {
        key_file.set_string("Desktop Entry", "Icon", icon);
    }
    key_file.set_boolean("Desktop Entry", "X-GNOME-Autostart-enabled", true);

    key_file.save_to_file(file_path)?;
    Ok(())
}

fn autostart_dir() -> PathBuf {
    PathBuf::from(glib::user_config_dir()).join("autostart")
}

fn sanitize_filename(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else if !output.ends_with('-') {
            output.push('-');
        }
    }

    output.trim_matches('-').to_string()
}
