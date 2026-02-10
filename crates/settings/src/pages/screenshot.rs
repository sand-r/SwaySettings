use gio::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use swaysettings_core::constants::{
    SETTINGS_SCREENSHOT_EDIT_CMD, SETTINGS_SCREENSHOT_EXIT_ON_SAVE, SETTINGS_SCREENSHOT_SAVE_DEST,
};

pub fn build_page() -> gtk4::Widget {
    let settings = gio::Settings::new("org.erikreider.swaysettings");

    let pref_group = libadwaita::PreferencesGroup::new();
    pref_group.set_title("Screenshot preferences");

    pref_group.add(&build_exit_on_save_row(&settings));
    pref_group.add(&build_save_destination_row(&settings));
    pref_group.add(&build_edit_command_row(&settings));

    let pref_page = libadwaita::PreferencesPage::new();
    pref_page.add(&pref_group);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&pref_page)
        .build();

    scrolled.upcast()
}

fn build_exit_on_save_row(settings: &gio::Settings) -> libadwaita::SwitchRow {
    let row = libadwaita::SwitchRow::builder()
        .title("Exit On Save")
        .build();

    settings
        .bind(SETTINGS_SCREENSHOT_EXIT_ON_SAVE, &row, "active")
        .build();

    row.add_suffix(&build_reset_button(settings, SETTINGS_SCREENSHOT_EXIT_ON_SAVE));
    row
}

fn build_save_destination_row(settings: &gio::Settings) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::builder()
        .title("Default Save Location")
        .subtitle_selectable(true)
        .build();

    settings
        .bind(SETTINGS_SCREENSHOT_SAVE_DEST, &row, "subtitle")
        .build();

    let button = gtk4::Button::from_icon_name("search-folder-symbolic");
    button.set_valign(gtk4::Align::Center);
    button.add_css_class("flat");
    button.set_tooltip_text(Some("Select the default save location"));

    let settings_clone = settings.clone();
    button.connect_clicked(clone!(
        #[weak]
        row,
        move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_modal(true);

            let settings = settings_clone.clone();
            dialog.select_folder(
                row.root()
                    .and_then(|r| r.downcast::<gtk4::Window>().ok())
                    .as_ref(),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let _ = settings.set_string(
                                SETTINGS_SCREENSHOT_SAVE_DEST,
                                &path.to_string_lossy(),
                            );
                        }
                    }
                },
            );
        }
    ));

    row.add_suffix(&button);
    row.add_suffix(&build_reset_button(settings, SETTINGS_SCREENSHOT_SAVE_DEST));
    row
}

fn build_edit_command_row(settings: &gio::Settings) -> libadwaita::EntryRow {
    let row = libadwaita::EntryRow::builder()
        .title("Edit Command. Ex: \"swappy -f -\"")
        .build();

    settings
        .bind(SETTINGS_SCREENSHOT_EDIT_CMD, &row, "text")
        .build();

    row.add_suffix(&build_reset_button(settings, SETTINGS_SCREENSHOT_EDIT_CMD));
    row
}

fn build_reset_button(settings: &gio::Settings, key: &'static str) -> gtk4::Button {
    let button = gtk4::Button::from_icon_name("arrow-circular-top-right-symbolic");
    button.set_valign(gtk4::Align::Center);
    button.add_css_class("flat");
    button.set_tooltip_text(Some("Reset to default"));

    let settings_click = settings.clone();
    button.connect_clicked(move |_| {
        settings_click.reset(key);
    });

    // Show only when value differs from default
    let update_visibility = {
        let button = button.downgrade();
        let settings = settings.clone();
        move || {
            if let Some(button) = button.upgrade() {
                let current = settings.value(key);
                let default = settings.default_value(key);
                button.set_visible(default.map_or(true, |d| d != current));
            }
        }
    };

    update_visibility();

    let update_vis = update_visibility.clone();
    settings.connect_changed(Some(key), move |_, _| {
        update_vis();
    });

    button
}
