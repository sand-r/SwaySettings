mod imp {
    use std::cell::{OnceCell, RefCell};
    use std::collections::HashMap;

    use glib::prelude::*;
    use glib::subclass::InitializingObject;
    use gtk4::subclass::prelude::*;
    use gtk4::CompositeTemplate;
    use libadwaita::subclass::prelude::*;

    use crate::sidebar_row::SidebarRow;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/SettingsWindow.ui")]
    pub struct SettingsWindowImpl {
        #[template_child]
        pub split_view: TemplateChild<libadwaita::NavigationSplitView>,
        #[template_child]
        pub sidebar_listbox: TemplateChild<gtk4::ListBox>,
        #[template_child]
        pub content_page: TemplateChild<libadwaita::NavigationPage>,
        #[template_child]
        pub content_toolbar: TemplateChild<libadwaita::ToolbarView>,

        pub settings: OnceCell<gio::Settings>,
        pub current_page_name: RefCell<Option<String>>,
        pub page_cache: RefCell<HashMap<crate::pages::PageType, gtk4::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsWindowImpl {
        const NAME: &'static str = "SwaySettingsWindow";
        type Type = super::SettingsWindow;
        type ParentType = libadwaita::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            SidebarRow::ensure_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SettingsWindowImpl {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for SettingsWindowImpl {}
    impl WindowImpl for SettingsWindowImpl {}
    impl ApplicationWindowImpl for SettingsWindowImpl {}
    impl AdwApplicationWindowImpl for SettingsWindowImpl {}
}

use gio::prelude::*;
use glib::Object;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use libadwaita::prelude::*;

use crate::pages::{self, PageType};
use crate::sidebar_row::SidebarRow;

glib::wrapper! {
    pub struct SettingsWindow(ObjectSubclass<imp::SettingsWindowImpl>)
        @extends libadwaita::ApplicationWindow, gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk4::Accessible, gtk4::Buildable,
                    gtk4::ConstraintTarget, gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl SettingsWindow {
    pub fn new(app: &libadwaita::Application, settings: &gio::Settings) -> Self {
        let window: Self = Object::builder().property("application", app).build();

        window.imp().settings.set(settings.clone()).unwrap();
        window.restore_window_size(settings);
        window.setup_close_handler(settings);
        window.setup_breakpoint();

        window
    }

    fn setup(&self) {
        self.setup_sidebar();
        self.setup_row_activation();
        self.select_first_page();
    }

    fn select_first_page(&self) {
        let imp = self.imp();
        if let Some(first_child) = imp.sidebar_listbox.first_child() {
            if let Some(first_row) = first_child.downcast_ref::<SidebarRow>() {
                imp.sidebar_listbox.select_row(Some(first_row));
                first_row.emit_activate();
            }
        }
    }

    fn restore_window_size(&self, settings: &gio::Settings) {
        let width = settings.int("window-width");
        let height = settings.int("window-height");
        if width > 0 {
            self.set_default_width(width);
        }
        if height > 0 {
            self.set_default_height(height);
        }
    }

    fn setup_close_handler(&self, settings: &gio::Settings) {
        let settings_clone = settings.clone();
        self.connect_close_request(move |win| {
            let _ = settings_clone.set_int("window-width", win.width());
            let _ = settings_clone.set_int("window-height", win.height());
            glib::Propagation::Proceed
        });
    }

    fn setup_breakpoint(&self) {
        let condition = libadwaita::BreakpointCondition::new_length(
            libadwaita::BreakpointConditionLengthType::MaxWidth,
            650.0,
            libadwaita::LengthUnit::Sp,
        );
        let breakpoint = libadwaita::Breakpoint::new(condition);
        breakpoint.add_setter(&*self.imp().split_view, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(breakpoint);
    }

    fn setup_sidebar(&self) {
        let imp = self.imp();

        imp.sidebar_listbox.set_activate_on_single_click(true);

        imp.sidebar_listbox.set_header_func(|row, before| {
            let Some(before) = before else {
                return;
            };

            let row = row.downcast_ref::<SidebarRow>().unwrap();
            let before = before.downcast_ref::<SidebarRow>().unwrap();

            if row.group() != before.group() {
                let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
                sep.set_margin_start(12);
                sep.set_margin_end(12);
                row.set_header(Some(&sep));
            }
        });

        let items = Self::build_items();
        for (page, icon, group) in items {
            let row = SidebarRow::new(page, icon, group);
            imp.sidebar_listbox.append(&row);
        }
    }

    fn setup_row_activation(&self) {
        let imp = self.imp();
        let split_view = imp.split_view.clone();
        let content_toolbar = imp.content_toolbar.clone();
        let content_page = imp.content_page.clone();
        let current_page_name = imp.current_page_name.clone();
        let page_cache = imp.page_cache.clone();

        imp.sidebar_listbox.connect_row_activated(move |_, row| {
            let row = row.downcast_ref::<SidebarRow>().unwrap();
            let page_type = row.page_type();
            let internal_name = page_type.internal_name().to_string();

            if current_page_name.borrow().as_deref() == Some(&internal_name) {
                split_view.set_show_content(true);
                return;
            }
            *current_page_name.borrow_mut() = Some(internal_name);

            let widget = page_cache
                .borrow_mut()
                .entry(page_type)
                .or_insert_with(|| pages::create_page(page_type))
                .clone();
            content_toolbar.set_content(Some(&widget));
            content_page.set_title(page_type.name());
            split_view.set_show_content(true);
        });
    }

    fn build_items() -> Vec<(PageType, &'static str, u32)> {
        vec![
            // Group 0: Users
            (PageType::Users, "org.gnome.Settings-users-symbolic", 0),
            // Group 1: System
            (PageType::AboutPc, "org.gnome.Settings-about-symbolic", 1),
            (
                PageType::Bluetooth,
                "org.gnome.Settings-bluetooth-symbolic",
                1,
            ),
            (PageType::Sound, "org.gnome.Settings-sound-symbolic", 1),
            (PageType::Power, "org.gnome.Settings-power-symbolic", 1),
            // Group 2: Desktop
            (PageType::Wallpaper, "preferences-desktop-wallpaper", 2),
            (
                PageType::Appearance,
                "org.gnome.Settings-appearance-symbolic",
                2,
            ),
            (PageType::StartupApps, "system-run-symbolic", 2),
            (
                PageType::DefaultApps,
                "org.gnome.Settings-applications-symbolic",
                2,
            ),
            (PageType::Screenshot, "camera-photo-symbolic", 2),
            // Group 3: Input
            (
                PageType::Keyboard,
                "org.gnome.Settings-keyboard-symbolic",
                3,
            ),
            (PageType::Mouse, "org.gnome.Settings-mouse-symbolic", 3),
            (PageType::Trackpad, "input-touchpad-symbolic", 3),
        ]
    }

    pub fn navigate_to_page(&self, internal_name: &str) {
        let imp = self.imp();
        let mut row = imp.sidebar_listbox.first_child();
        while let Some(widget) = row {
            if let Some(sidebar_row) = widget.downcast_ref::<SidebarRow>() {
                if sidebar_row.page_type().internal_name() == internal_name {
                    imp.sidebar_listbox.select_row(Some(sidebar_row));
                    sidebar_row.emit_activate();
                    return;
                }
            }
            row = widget.next_sibling();
        }
    }
}
