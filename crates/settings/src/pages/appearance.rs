use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use glib::{self, clone, Variant, VariantTy};
use gtk4::prelude::*;
use libadwaita::prelude::*;

use swaysettings_core::constants::{SETTINGS_THEME_DARK, SETTINGS_THEME_LIGHT};
use swaysettings_core::functions;

const DEFAULT_THEME: &str = "Adwaita";

const TINY_WINDOW_HEIGHT: i32 = 64;
const TINY_WINDOW_WIDTH: i32 = 90;
const PREVIEW_HEIGHT: i32 = 140;
const PREVIEW_WIDTH: i32 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeStyle {
    Light,
    Dark,
}

impl ThemeStyle {
    fn label(self) -> &'static str {
        match self {
            ThemeStyle::Light => "Light",
            ThemeStyle::Dark => "Dark",
        }
    }

    fn gsettings_name(self) -> &'static str {
        match self {
            ThemeStyle::Light => "default",
            ThemeStyle::Dark => "prefer-dark",
        }
    }

    fn from_gsettings(value: &str) -> ThemeStyle {
        match value {
            "prefer-dark" => ThemeStyle::Dark,
            _ => ThemeStyle::Light,
        }
    }

    fn self_settings_key(self) -> &'static str {
        match self {
            ThemeStyle::Dark => SETTINGS_THEME_DARK,
            ThemeStyle::Light => SETTINGS_THEME_LIGHT,
        }
    }

    fn preview_class(self, front: bool) -> &'static str {
        match self {
            ThemeStyle::Light => {
                if front {
                    "light"
                } else {
                    "dark"
                }
            }
            ThemeStyle::Dark => "dark",
        }
    }
}

pub fn build_page() -> gtk4::Widget {
    let gnome_settings = gio::Settings::new("org.gnome.desktop.interface");
    let self_settings = gio::Settings::new("org.erikreider.swaysettings");

    let pref_page = libadwaita::PreferencesPage::new();

    // Style section (Light/Dark toggle)
    let style_group = libadwaita::PreferencesGroup::new();
    style_group.set_title("Style");

    let style_row = libadwaita::PreferencesRow::builder()
        .activatable(false)
        .selectable(false)
        .build();
    style_group.add(&style_row);

    let clamp = libadwaita::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    style_row.set_child(Some(&clamp));

    let style_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(24)
        .homogeneous(true)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    clamp.set_child(Some(&style_box));

    let preview_light = build_theme_preview(ThemeStyle::Light);
    let preview_dark = build_theme_preview(ThemeStyle::Dark);
    preview_dark.set_group(Some(&preview_light));

    let light_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    light_box.append(&preview_light);
    light_box.append(&gtk4::Label::new(Some(ThemeStyle::Light.label())));
    style_box.append(&light_box);

    let dark_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    dark_box.append(&preview_dark);
    dark_box.append(&gtk4::Label::new(Some(ThemeStyle::Dark.label())));
    style_box.append(&dark_box);

    // Set active from current settings
    set_style_from_settings(&gnome_settings, &preview_light, &preview_dark);

    // Connect toggle signals
    {
        let gnome_settings = gnome_settings.clone();
        let self_settings = self_settings.clone();
        preview_light.connect_toggled(clone!(
            #[weak]
            preview_light,
            move |_| {
                if preview_light.is_active() {
                    style_toggled(ThemeStyle::Light, &gnome_settings, &self_settings);
                }
            }
        ));
    }
    {
        let gnome_settings = gnome_settings.clone();
        let self_settings = self_settings.clone();
        preview_dark.connect_toggled(clone!(
            #[weak]
            preview_dark,
            move |_| {
                if preview_dark.is_active() {
                    style_toggled(ThemeStyle::Dark, &gnome_settings, &self_settings);
                }
            }
        ));
    }

    pref_page.add(&style_group);

    // Accent Color section
    let accent_group = Rc::new(RefCell::new(build_accent_widget(&gnome_settings)));
    pref_page.add(&*accent_group.borrow());

    // Waydock section (optional)
    if let Some(dock_widget) = build_dock_section() {
        if let Ok(dock_group) = dock_widget.downcast::<libadwaita::PreferencesGroup>() {
            pref_page.add(&dock_group);
        }
    }

    // GTK Options section
    let gtk_group = Rc::new(RefCell::new(build_gtk_options(
        &gnome_settings,
        &self_settings,
    )));
    pref_page.add(&*gtk_group.borrow());

    // Sync theme on startup
    sync_gtk_theme(&gnome_settings, &self_settings);

    // Listen for external changes
    {
        let gnome_settings_c = gnome_settings.clone();
        let self_settings_c = self_settings.clone();
        let pref_page_c = pref_page.clone();
        let accent_group_c = accent_group.clone();
        let gtk_group_c = gtk_group.clone();
        let preview_light_c = preview_light.clone();
        let preview_dark_c = preview_dark.clone();
        gnome_settings.connect_changed(None, move |settings, key| {
            match key {
                "color-scheme" => {
                    set_style_from_settings(settings, &preview_light_c, &preview_dark_c);
                }
                "accent-color" => {
                    let old = accent_group_c.borrow().clone();
                    pref_page_c.remove(&old);
                    let new = build_accent_widget(settings);
                    // Insert after style group (use add, it appends)
                    pref_page_c.add(&new);
                    *accent_group_c.borrow_mut() = new;
                }
                "gtk-theme" | "icon-theme" | "cursor-theme" | "enable-animations"
                | "overlay-scrolling" => {
                    let old = gtk_group_c.borrow().clone();
                    pref_page_c.remove(&old);
                    let new = build_gtk_options(&gnome_settings_c, &self_settings_c);
                    pref_page_c.add(&new);
                    *gtk_group_c.borrow_mut() = new;
                }
                _ => {}
            }
        });
    }
    {
        let gnome_settings_c = gnome_settings.clone();
        let self_settings_c = self_settings.clone();
        let pref_page_c = pref_page.clone();
        let gtk_group_c = gtk_group.clone();
        self_settings.connect_changed(None, move |_, key| {
            match key {
                SETTINGS_THEME_DARK | SETTINGS_THEME_LIGHT => {
                    let old = gtk_group_c.borrow().clone();
                    pref_page_c.remove(&old);
                    let new = build_gtk_options(&gnome_settings_c, &self_settings_c);
                    pref_page_c.add(&new);
                    *gtk_group_c.borrow_mut() = new;
                }
                _ => {}
            }
        });
    }

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&pref_page)
        .build();

    scrolled.upcast()
}

fn get_color_scheme(settings: &gio::Settings) -> ThemeStyle {
    let value = settings.string("color-scheme");
    ThemeStyle::from_gsettings(&value)
}

fn get_theme_for_style(
    gnome_settings: &gio::Settings,
    self_settings: &gio::Settings,
) -> Option<String> {
    let style = get_color_scheme(gnome_settings);
    let key = style.self_settings_key();
    functions::get_gsetting(self_settings, key, VariantTy::STRING)
        .and_then(|v| v.get::<String>())
}

fn set_self_theme(
    gnome_settings: &gio::Settings,
    self_settings: &gio::Settings,
    theme: &str,
) {
    let style = get_color_scheme(gnome_settings);
    let key = style.self_settings_key();
    functions::set_gsetting(self_settings, key, &Variant::from(theme));
}

fn sync_gtk_theme(gnome_settings: &gio::Settings, self_settings: &gio::Settings) {
    let theme = get_theme_for_style(gnome_settings, self_settings);
    let applied = functions::get_gsetting(gnome_settings, "gtk-theme", VariantTy::STRING)
        .and_then(|v| v.get::<String>());

    let theme = match theme {
        Some(ref t) if !t.is_empty() => {
            if applied.as_deref() == Some(t) {
                return;
            }
            t.clone()
        }
        _ => {
            let default = DEFAULT_THEME.to_string();
            set_self_theme(gnome_settings, self_settings, &default);
            default
        }
    };

    set_gtk_value(gnome_settings, "gtk-theme", &Variant::from(theme.as_str()), true);
}

fn set_style_from_settings(
    settings: &gio::Settings,
    preview_light: &gtk4::ToggleButton,
    preview_dark: &gtk4::ToggleButton,
) {
    let is_dark = get_color_scheme(settings) == ThemeStyle::Dark;
    if is_dark {
        preview_dark.set_active(true);
    } else {
        preview_light.set_active(true);
    }
}

fn style_toggled(
    style: ThemeStyle,
    gnome_settings: &gio::Settings,
    self_settings: &gio::Settings,
) {
    set_gtk_value(
        gnome_settings,
        "color-scheme",
        &Variant::from(style.gsettings_name()),
        false,
    );
    sync_gtk_theme(gnome_settings, self_settings);
}

fn build_theme_preview(style: ThemeStyle) -> gtk4::ToggleButton {
    let button = gtk4::ToggleButton::new();
    button.set_width_request(PREVIEW_WIDTH);
    button.set_height_request(PREVIEW_HEIGHT);
    button.set_has_frame(false);
    button.set_halign(gtk4::Align::Center);
    button.set_overflow(gtk4::Overflow::Hidden);
    button.add_css_class("theme-preview-item");

    let overlay = gtk4::Overlay::new();
    button.set_child(Some(&overlay));

    let background = gtk4::Picture::new();
    background.set_content_fit(gtk4::ContentFit::Fill);
    let layout = gtk4::CenterLayout::new();
    background.set_layout_manager(Some(layout));
    overlay.set_child(Some(&background));

    // Load wallpaper background
    draw_preview_background(&background);

    let fixed = gtk4::Fixed::new();
    overlay.add_overlay(&fixed);

    // Add fake floating windows
    fixed.put(&build_tiny_window(style, false), 50.0, 25.0);
    fixed.put(&build_tiny_window(style, true), 20.0, 45.0);

    button
}

fn draw_preview_background(picture: &gtk4::Picture) {
    let mut cache_path = glib::user_cache_dir();
    cache_path.push("wallpaper");
    let cache_path_str = cache_path.to_string_lossy().to_string();

    let weak = glib::SendWeakRef::from(picture.downgrade());
    let context = glib::MainContext::default();

    std::thread::spawn(move || {
        let texture = match functions::generate_thumbnail(&cache_path_str, false, 256) {
            Ok(thumb_path) => gdk4::Texture::from_filename(&thumb_path).ok(),
            Err(_) => gdk4::Texture::from_filename(&cache_path_str).ok(),
        };

        if let Some(tex) = texture {
            let (scaled_w, scaled_h) = functions::calc_scaled_size(
                tex.width() as f32,
                tex.height() as f32,
                PREVIEW_WIDTH as f32,
                PREVIEW_HEIGHT as f32,
            );

            let scaled_texture = {
                let data_size = (tex.width() as usize) * (tex.height() as usize) * 4;
                let mut data = vec![0u8; data_size];
                let stride = (tex.width() as usize) * 4;
                tex.download(&mut data, stride);
                let pixels = glib::Bytes::from_owned(data);
                let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
                    &pixels,
                    gdk_pixbuf::Colorspace::Rgb,
                    true,
                    8,
                    tex.width(),
                    tex.height(),
                    tex.width() * 4,
                );
                pixbuf
                    .scale_simple(
                        scaled_w as i32,
                        scaled_h as i32,
                        gdk_pixbuf::InterpType::Bilinear,
                    )
                    .and_then(|scaled| {
                        let w = scaled.width();
                        let h = scaled.height();
                        let fmt = if scaled.has_alpha() {
                            gdk4::MemoryFormat::R8g8b8a8
                        } else {
                            gdk4::MemoryFormat::R8g8b8
                        };
                        let bytes = scaled.read_pixel_bytes();
                        let stride = scaled.rowstride() as usize;
                        Some(
                            gdk4::MemoryTexture::new(w, h, fmt, &bytes, stride)
                                .upcast::<gdk4::Texture>(),
                        )
                    })
            };

            context.invoke(move || {
                if let Some(picture) = weak.upgrade() {
                    match scaled_texture {
                        Some(ref tex) => picture.set_paintable(Some(tex)),
                        None => picture.set_paintable(None::<&gdk4::Texture>),
                    }
                }
            });
        }
    });
}

fn build_tiny_window(style: ThemeStyle, front: bool) -> gtk4::Widget {
    let window = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    window.add_css_class("window");
    window.add_css_class(style.preview_class(front));
    if front {
        window.add_css_class("front");
    } else {
        window.add_css_class("back");
    }

    let header_bar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    header_bar.add_css_class("header-bar");
    header_bar.add_css_class(style.preview_class(front));
    window.append(&header_bar);

    window.set_size_request(TINY_WINDOW_WIDTH, TINY_WINDOW_HEIGHT);
    window.upcast()
}

// Accent Color section
fn build_accent_widget(settings: &gio::Settings) -> libadwaita::PreferencesGroup {
    let group = libadwaita::PreferencesGroup::new();
    group.set_title("Accent Color");

    let row = libadwaita::PreferencesRow::builder()
        .activatable(false)
        .build();
    group.add(&row);

    let accent_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .halign(gtk4::Align::Center)
        .build();
    row.set_child(Some(&accent_box));

    let accent_colors = [
        ("blue", 0),
        ("teal", 1),
        ("green", 2),
        ("yellow", 3),
        ("orange", 4),
        ("red", 5),
        ("pink", 6),
        ("purple", 7),
        ("slate", 8),
    ];

    let current_accent = settings.string("accent-color").to_string();
    let mut prev_button: Option<gtk4::ToggleButton> = None;

    for (nick, _value) in &accent_colors {
        let button = gtk4::ToggleButton::new();
        button.add_css_class("accent-button");
        button.add_css_class(nick);

        if *nick == current_accent {
            button.set_active(true);
        }

        let settings_clone = settings.clone();
        let nick_str = nick.to_string();
        button.connect_toggled(move |btn| {
            if btn.is_active() {
                functions::set_gsetting(
                    &settings_clone,
                    "accent-color",
                    &Variant::from(nick_str.as_str()),
                );
            }
        });

        if let Some(ref prev) = prev_button {
            button.set_group(Some(prev));
        }
        prev_button = Some(button.clone());

        accent_box.append(&button);
    }

    group
}

// Waydock section
fn build_dock_section() -> Option<gtk4::Widget> {
    let schema_source = gio::SettingsSchemaSource::default()?;
    let dock_schema = schema_source.lookup("org.erikreider.waydock", true)?;
    let dock_settings = gio::Settings::new_full(&dock_schema, None::<&gio::SettingsBackend>, None);

    let group = libadwaita::PreferencesGroup::new();
    group.set_title("Waydock");
    let mut has_rows = false;

    // Position combo
    if dock_schema.has_key("position") {
        if let Some(position_row) = build_dock_position_row(&dock_settings, &dock_schema) {
            group.add(&position_row);
            has_rows = true;
        }
    }

    // Minimized switch
    if dock_schema.has_key("minimized") {
        let minimized_row = libadwaita::SwitchRow::builder()
            .title("Minimized")
            .build();
        dock_settings
            .bind("minimized", &minimized_row, "active")
            .build();
        group.add(&minimized_row);
        has_rows = true;
    }

    if has_rows {
        Some(group.upcast())
    } else {
        None
    }
}

fn build_dock_position_row(
    dock_settings: &gio::Settings,
    dock_schema: &gio::SettingsSchema,
) -> Option<libadwaita::ComboRow> {
    let key = dock_schema.key("position");
    let range = key.range();

    // range is (sv) where s="enum" and v is the string array of values
    if range.type_().as_str() != "(sv)" {
        return None;
    }

    let range_type = range.child_value(0);
    if range_type.get::<String>().as_deref() != Some("enum") {
        return None;
    }

    let variant = range.child_value(1).as_variant()?;
    let values = variant.get::<Vec<String>>()?;
    if values.is_empty() {
        return None;
    }

    let selected = dock_settings.string("position").to_string();

    let combo_row = libadwaita::ComboRow::builder()
        .title("Position")
        .build();

    let model = gio::ListStore::new::<gtk4::StringObject>();
    let mut selected_idx = 0u32;
    for (i, val) in values.iter().enumerate() {
        model.append(&gtk4::StringObject::new(val));
        if *val == selected {
            selected_idx = i as u32;
        }
    }
    combo_row.set_model(Some(&model));
    combo_row.set_selected(selected_idx);

    let dock_settings_clone = dock_settings.clone();
    combo_row.connect_selected_notify(move |row| {
        let idx = row.selected();
        if let Some(item) = model.item(idx) {
            if let Some(string_obj) = item.downcast_ref::<gtk4::StringObject>() {
                let _ = dock_settings_clone.set_string("position", &string_obj.string());
            }
        }
    });

    Some(combo_row)
}

// GTK Options section
fn build_gtk_options(
    gnome_settings: &gio::Settings,
    self_settings: &gio::Settings,
) -> libadwaita::PreferencesGroup {
    let group = libadwaita::PreferencesGroup::new();
    group.set_title("GTK Options");

    group.add(&build_theme_combo_row(
        "GTK3 Theme",
        "gtk-theme",
        "themes",
        gnome_settings,
        self_settings,
    ));
    group.add(&build_theme_combo_row(
        "Icon Theme",
        "icon-theme",
        "icons",
        gnome_settings,
        self_settings,
    ));
    group.add(&build_theme_combo_row(
        "Cursor Theme",
        "cursor-theme",
        "icons",
        gnome_settings,
        self_settings,
    ));
    group.add(&build_toggle_row("Animations", "enable-animations", gnome_settings));
    group.add(&build_toggle_row(
        "Overlay Scrolling",
        "overlay-scrolling",
        gnome_settings,
    ));

    group
}

fn build_toggle_row(
    title: &str,
    setting_name: &str,
    settings: &gio::Settings,
) -> libadwaita::SwitchRow {
    let row = libadwaita::SwitchRow::builder().title(title).build();

    if let Some(schema) = settings.settings_schema() {
        if schema.has_key(setting_name) {
            let key_type = schema.key(setting_name).value_type();
            if key_type.as_ref().as_str() == "b" {
                let value = settings.boolean(setting_name);
                row.set_active(value);

                let settings_clone = settings.clone();
                let setting_name = setting_name.to_string();
                row.connect_active_notify(move |row| {
                    set_gtk_value(
                        &settings_clone,
                        &setting_name,
                        &Variant::from(row.is_active()),
                        true,
                    );
                });

                row.set_activatable(true);
                return row;
            }
        }
    }

    row.set_sensitive(false);
    row
}

fn build_theme_combo_row(
    title: &str,
    setting_name: &str,
    folder_name: &str,
    gnome_settings: &gio::Settings,
    self_settings: &gio::Settings,
) -> libadwaita::ComboRow {
    let combo_row = libadwaita::ComboRow::builder().title(title).build();

    let current_theme = if let Some(schema) = gnome_settings.settings_schema() {
        if schema.has_key(setting_name) {
            Some(gnome_settings.string(setting_name).to_string())
        } else {
            None
        }
    } else {
        None
    };

    let themes = discover_themes(setting_name, folder_name);

    if current_theme.is_none() || themes.is_empty() {
        combo_row.set_sensitive(false);
        return combo_row;
    }

    let current_theme = current_theme.unwrap();
    let model = gio::ListStore::new::<gtk4::StringObject>();
    let mut selected_idx = 0u32;

    for (i, theme) in themes.iter().enumerate() {
        model.append(&gtk4::StringObject::new(theme));
        if *theme == current_theme {
            selected_idx = i as u32;
        }
    }

    combo_row.set_model(Some(&model));
    let expression =
        gtk4::PropertyExpression::new(gtk4::StringObject::static_type(), None::<gtk4::Expression>, "string");
    combo_row.set_expression(Some(&expression));
    combo_row.set_selected(selected_idx);

    let gnome_settings = gnome_settings.clone();
    let self_settings = self_settings.clone();
    let setting_name = setting_name.to_string();
    combo_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(theme) = themes.get(idx) {
            set_self_theme(&gnome_settings, &self_settings, theme);
            set_gtk_value(
                &gnome_settings,
                &setting_name,
                &Variant::from(theme.as_str()),
                true,
            );
        }
    });

    combo_row
}

fn set_gtk_value(
    settings: &gio::Settings,
    setting_name: &str,
    value: &Variant,
    write_file: bool,
) {
    let theme_value = functions::set_gsetting(settings, setting_name, value);
    let theme_value = match theme_value {
        Some(v) if write_file => v,
        _ => return,
    };

    let looking_for = match setting_name {
        "gtk-theme" => "gtk-theme-name",
        "icon-theme" => "gtk-icon-theme-name",
        "cursor-theme" => "gtk-cursor-theme-name",
        "enable-animations" => "gtk-enable-animations",
        "overlay-scrolling" => "gtk-overlay-scrolling",
        _ => return,
    };

    let cfg_dir = glib::user_config_dir();
    let paths = [
        cfg_dir.join("gtk-2.0").join("settings.ini"),
        cfg_dir.join("gtk-3.0").join("settings.ini"),
        cfg_dir.join("gtk-4.0").join("settings.ini"),
    ];

    for path in &paths {
        write_gtk_config(path, looking_for, &theme_value);
    }
}

fn write_gtk_config(path: &Path, looking_for: &str, theme_value: &str) {
    if !path.exists() {
        return;
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(err) => {
            log::warn!("Could not read {}: {err}", path.display());
            return;
        }
    };

    let mut new_contents = String::new();
    let mut changed = false;

    for line in contents.lines() {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() == 2 && parts[0] == looking_for {
            let new_line = format!("{}={}", parts[0], theme_value);
            if line != new_line {
                new_contents.push_str(&new_line);
                changed = true;
            } else {
                new_contents.push_str(line);
            }
        } else {
            new_contents.push_str(line);
        }
        new_contents.push('\n');
    }

    if !changed {
        log::debug!("Skipped writing config: {}", path.display());
        return;
    }

    if let Err(err) = std::fs::write(path, &new_contents) {
        log::warn!("Theme writing error for {}: {err}", path.display());
    }
}

fn discover_themes(setting_name: &str, folder_name: &str) -> Vec<String> {
    let mut search_dirs: Vec<PathBuf> = glib::system_data_dirs()
        .iter()
        .map(|d| PathBuf::from(d).join(folder_name))
        .collect();

    search_dirs.push(PathBuf::from(glib::user_data_dir()).join(folder_name));

    // Also check ~/.themes or ~/.icons
    if let Some(home) = glib::home_dir().to_str() {
        search_dirs.push(PathBuf::from(format!("{home}/.{folder_name}")));
    }

    let mut themes = Vec::new();

    for dir_path in &mut search_dirs {
        functions::extract_symlink(dir_path);
        if !dir_path.is_dir() {
            continue;
        }

        let entries = match std::fs::read_dir(dir_path.as_path()) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let mut child_path = entry.path();
            functions::extract_symlink(&mut child_path);

            if !child_path.is_dir() {
                continue;
            }

            // Skip flatpak export dirs
            let child_str = child_path.to_string_lossy();
            if child_str.contains("/flatpak/exports/share/") {
                continue;
            }

            let name = match child_path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };

            match folder_name {
                "themes" => {
                    if is_valid_gtk_theme(&child_path) && !themes.contains(&name) {
                        themes.push(name);
                    }
                }
                "icons" => {
                    if is_valid_icon_theme(setting_name, &child_path) && !themes.contains(&name) {
                        themes.push(name);
                    }
                }
                _ => {}
            }
        }
    }

    themes.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    themes
}

fn is_valid_gtk_theme(folder_path: &Path) -> bool {
    // Check for gtk-3.0/gtk.css
    let gtk3_css = folder_path.join("gtk-3.0").join("gtk.css");
    if gtk3_css.exists() {
        return true;
    }

    // Check for gtk-3.<minor_version>/gtk.css
    let minor = gtk4::minor_version();
    let even_minor = if minor % 2 != 0 { minor + 1 } else { minor };
    let versioned_css = folder_path
        .join(format!("gtk-3.{even_minor}"))
        .join("gtk.css");
    versioned_css.exists()
}

fn is_valid_icon_theme(setting_name: &str, folder_path: &Path) -> bool {
    match setting_name {
        "cursor-theme" => {
            let cursors_dir = folder_path.join("cursors");
            cursors_dir.is_dir()
        }
        "icon-theme" => {
            let index_file = folder_path.join("index.theme");
            if !index_file.is_file() {
                return false;
            }

            // Check if the theme has size directories (scalable, symbolic, NxN)
            let entries = match std::fs::read_dir(folder_path) {
                Ok(e) => e,
                Err(_) => return false,
            };

            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name == "scalable" || name == "symbolic" {
                        return true;
                    }
                    // Check for NxN pattern
                    let parts: Vec<&str> = name.split('x').collect();
                    if parts.len() == 2 {
                        if parts[0].parse::<u32>().is_ok() && parts[1].parse::<u32>().is_ok() {
                            return true;
                        }
                    }
                }
            }
            false
        }
        _ => false,
    }
}
