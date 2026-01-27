use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::pages::{self, PageType};

#[derive(Clone)]
struct SettingsItem {
    icon: &'static str,
    page: PageType,
    disabled: bool,
    group: usize,
}

pub struct WindowState {
    pub window: libadwaita::ApplicationWindow,
    listbox: gtk4::ListBox,
}

impl WindowState {
    pub fn navigate_to_page(&self, internal_name: &str) {
        let mut row = self.listbox.first_child();
        while let Some(widget) = row {
            if let Some(list_row) = widget.downcast_ref::<gtk4::ListBoxRow>() {
                if let Some(name) = unsafe { list_row.data::<String>("internal-name").map(|p| p.as_ref().clone()) } {
                    if name == internal_name {
                        self.listbox.select_row(Some(list_row));
                        list_row.emit_activate();
                        return;
                    }
                }
            }
            row = widget.next_sibling();
        }
    }
}

pub fn build_window(app: &libadwaita::Application, settings: &gio::Settings) -> WindowState {
    let window = libadwaita::ApplicationWindow::new(app);

    let width = settings.int("window-width");
    let height = settings.int("window-height");
    if width > 0 {
        window.set_default_width(width);
    }
    if height > 0 {
        window.set_default_height(height);
    }
    window.set_width_request(500);
    window.set_height_request(300);

    let settings_clone = settings.clone();
    window.connect_close_request(move |win| {
        settings_clone.set_int("window-width", win.width());
        settings_clone.set_int("window-height", win.height());
        glib::Propagation::Proceed
    });

    let split_view = libadwaita::NavigationSplitView::new();
    split_view.set_show_content(true);
    window.set_content(Some(&split_view));

    let condition = libadwaita::BreakpointCondition::new_length(
        libadwaita::BreakpointConditionLengthType::MaxWidth,
        650.0,
        libadwaita::LengthUnit::Sp,
    );
    let breakpoint = libadwaita::Breakpoint::new(condition);
    breakpoint.add_setter(&split_view, "collapsed", &true.to_value());
    window.add_breakpoint(breakpoint);

    let sidebar_listbox = gtk4::ListBox::new();
    sidebar_listbox.add_css_class("navigation-sidebar");
    sidebar_listbox.set_hexpand(true);
    sidebar_listbox.set_vexpand(true);
    sidebar_listbox.set_activate_on_single_click(true);
    sidebar_listbox.set_selection_mode(gtk4::SelectionMode::Single);

    let toolbar_view = libadwaita::ToolbarView::new();
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_child(Some(&sidebar_listbox));
    toolbar_view.set_content(Some(&scrolled));

    let nav_page = libadwaita::NavigationPage::new(&toolbar_view, "Settings");
    split_view.set_sidebar(Some(&nav_page));

    let content_toolbar = libadwaita::ToolbarView::new();
    let content_page = libadwaita::NavigationPage::new(&content_toolbar, "");
    split_view.set_content(Some(&content_page));

    let current_page_name: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let items = build_items();

    sidebar_listbox.set_header_func(move |row, before| {
        let before = match before {
            Some(row) => row,
            None => return,
        };
        let row_group = unsafe { row.data::<usize>("group").map(|p| *p.as_ref()).unwrap_or(0) };
        let before_group = unsafe { before.data::<usize>("group").map(|p| *p.as_ref()).unwrap_or(0) };
        if row_group != before_group {
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_margin_start(12);
            sep.set_margin_end(12);
            row.set_header(Some(&sep));
        }
    });

    for item in &items {
        let row = build_row(item);
        unsafe {
            row.set_data("page", item.page);
            row.set_data("internal-name", item.page.internal_name().to_string());
            row.set_data("group", item.group);
        }
        if item.disabled {
            row.set_sensitive(false);
        }
        sidebar_listbox.append(&row);
    }

    let current_page_name_clone = current_page_name.clone();
    sidebar_listbox.connect_row_activated(move |_, row| {
        let internal_name = unsafe {
            row.data::<String>("internal-name")
                .map(|p| p.as_ref().clone())
                .unwrap_or_default()
        };
        if current_page_name_clone.borrow().as_deref() == Some(&internal_name) {
            split_view.set_show_content(true);
            return;
        }
        *current_page_name_clone.borrow_mut() = Some(internal_name.clone());

        let page = unsafe { row.data::<PageType>("page").map(|p| *p.as_ref()) };
        if let Some(page) = page {
            let widget = pages::create_page(page);
            content_toolbar.set_content(Some(&widget));
            content_page.set_title(page.name());
            split_view.set_show_content(true);
        }
    });

    WindowState { window, listbox: sidebar_listbox }
}

fn build_items() -> Vec<SettingsItem> {
    let mut items = Vec::new();
    let mut group = 0;

    items.push(SettingsItem {
        icon: "org.gnome.Settings-users-symbolic",
        page: PageType::Users,
        disabled: false,
        group,
    });

    group += 1;
    items.extend([
        SettingsItem {
            icon: "org.gnome.Settings-about-symbolic",
            page: PageType::AboutPc,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "org.gnome.Settings-bluetooth-symbolic",
            page: PageType::Bluetooth,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "org.gnome.Settings-sound-symbolic",
            page: PageType::Sound,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "org.gnome.Settings-power-symbolic",
            page: PageType::Power,
            disabled: false,
            group,
        },
    ]);

    group += 1;
    items.extend([
        SettingsItem {
            icon: "preferences-desktop-wallpaper",
            page: PageType::Wallpaper,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "org.gnome.Settings-appearance-symbolic",
            page: PageType::Appearance,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "system-run-symbolic",
            page: PageType::StartupApps,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "org.gnome.Settings-applications-symbolic",
            page: PageType::DefaultApps,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "camera-photo-symbolic",
            page: PageType::Screenshot,
            disabled: false,
            group,
        },
    ]);

    group += 1;
    items.extend([
        SettingsItem {
            icon: "org.gnome.Settings-keyboard-symbolic",
            page: PageType::Keyboard,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "org.gnome.Settings-mouse-symbolic",
            page: PageType::Mouse,
            disabled: false,
            group,
        },
        SettingsItem {
            icon: "input-touchpad-symbolic",
            page: PageType::Trackpad,
            disabled: false,
            group,
        },
    ]);

    items
}

fn build_row(item: &SettingsItem) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    let action_row = libadwaita::ActionRow::new();
    action_row.set_title(item.page.name());
    action_row.add_prefix(&gtk4::Image::from_icon_name(item.icon));
    row.set_child(Some(&action_row));
    row
}
