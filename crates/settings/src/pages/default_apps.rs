use std::cell::RefCell;

use gio::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

#[derive(Clone, Debug)]
struct DefaultAppData {
    category_name: &'static str,
    mime_type: &'static str,
    extra_types: &'static [&'static str],
}

const DEFAULT_APPS: &[DefaultAppData] = &[
    DefaultAppData {
        category_name: "Web",
        mime_type: "x-scheme-handler/http",
        extra_types: &[
            "text/html",
            "application/xhtml+xml",
            "x-scheme-handler/https",
        ],
    },
    DefaultAppData {
        category_name: "Mail",
        mime_type: "x-scheme-handler/mailto",
        extra_types: &[],
    },
    DefaultAppData {
        category_name: "Calendar",
        mime_type: "text/calendar",
        extra_types: &[],
    },
    DefaultAppData {
        category_name: "Music",
        mime_type: "audio/x-vorbis+ogg",
        extra_types: &["audio/*"],
    },
    DefaultAppData {
        category_name: "Video",
        mime_type: "video/x-ogm+ogg",
        extra_types: &["video/*"],
    },
    DefaultAppData {
        category_name: "Photos",
        mime_type: "image/jpeg",
        extra_types: &["image/*"],
    },
];

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/DefaultAppsContent.ui")]
    pub struct DefaultAppsContent {
        #[template_child(id = "stack")]
        pub stack: TemplateChild<gtk4::Stack>,
        #[template_child(id = "apps_group")]
        pub apps_group: TemplateChild<libadwaita::PreferencesGroup>,

        pub rows: RefCell<Vec<libadwaita::PreferencesRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DefaultAppsContent {
        const NAME: &'static str = "SwaySettingsDefaultAppsContent";
        type Type = super::DefaultAppsContent;
        type ParentType = libadwaita::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DefaultAppsContent {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().refresh_rows();
        }
    }

    impl WidgetImpl for DefaultAppsContent {}
    impl BinImpl for DefaultAppsContent {}
}

glib::wrapper! {
    pub struct DefaultAppsContent(ObjectSubclass<imp::DefaultAppsContent>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

pub fn build_page() -> gtk4::Widget {
    DefaultAppsContent::new().upcast()
}

impl DefaultAppsContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn refresh_rows(&self) {
        let imp = self.imp();
        let mut has_choices = false;

        let existing_rows = std::mem::take(&mut *imp.rows.borrow_mut());
        for row in existing_rows {
            imp.apps_group.remove(&row);
        }

        for def_app in DEFAULT_APPS.iter().cloned() {
            let row = create_app_row(def_app, &mut has_choices);
            imp.apps_group.add(&row);
            imp.rows.borrow_mut().push(row.upcast());
        }

        imp.stack
            .set_visible_child_name(if has_choices { "page" } else { "placeholder" });
    }
}

fn create_app_row(def_app: DefaultAppData, has_choices: &mut bool) -> libadwaita::PreferencesRow {
    let mut apps = gio::AppInfo::all_for_type(def_app.mime_type);
    apps.sort_by(|a, b| {
        a.display_name()
            .to_string()
            .to_lowercase()
            .cmp(&b.display_name().to_string().to_lowercase())
    });

    if apps.is_empty() {
        return libadwaita::ActionRow::builder()
            .title(def_app.category_name)
            .subtitle("No compatible applications found")
            .activatable(false)
            .build()
            .upcast();
    }

    *has_choices = true;

    let model = gio::ListStore::new::<gtk4::StringObject>();
    for app in &apps {
        model.append(&gtk4::StringObject::new(&app.display_name()));
    }

    let default_id = gio::AppInfo::default_for_type(def_app.mime_type, false)
        .and_then(|app| app.id().map(|id| id.to_string()));

    let default_index = default_id
        .as_ref()
        .and_then(|default_id| {
            apps.iter().position(|app| {
                app.id()
                    .map(|id| id.as_str() == default_id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(0);

    let row = libadwaita::ComboRow::builder()
        .title(def_app.category_name)
        .build();

    row.set_model(Some(&model));
    row.set_selected(default_index as u32);

    row.connect_selected_notify(move |row| {
        let index = row.selected() as usize;
        let Some(app) = apps.get(index) else {
            return;
        };

        set_default_app(&def_app, app);
    });

    row.upcast()
}

fn set_default_app(def_app: &DefaultAppData, selected_app: &gio::AppInfo) {
    set_default_for_mime(def_app.mime_type, selected_app);

    if !def_app.extra_types.is_empty() {
        for mime in selected_app.supported_types() {
            if def_app
                .extra_types
                .iter()
                .any(|pattern| mime_type_matches_pattern(pattern, mime.as_str()))
            {
                set_default_for_mime(mime.as_str(), selected_app);
            }
        }
    }
}

fn set_default_for_mime(mime_type: &str, selected_app: &gio::AppInfo) {
    if let Err(err) = selected_app.set_as_default_for_type(mime_type) {
        log::warn!(
            "Could not set '{}' as default for '{}': {err}",
            selected_app.display_name(),
            mime_type
        );
    }
}

fn mime_type_matches_pattern(pattern: &str, mime: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return mime.starts_with(prefix) && mime.as_bytes().get(prefix.len()) == Some(&b'/');
    }

    pattern == mime
}
