use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use swaysettings_core::constants::{
    SETTINGS_USER_WALLPAPERS, SETTINGS_WALLPAPER_PATH, SETTINGS_WALLPAPER_SCALING_MODE,
    WALLPAPER_ACTION_NAME,
};
use swaysettings_core::functions;
use swaysettings_core::utils::{self, ScaleMode};

const PREVIEW_IMAGE_WIDTH: i32 = 384;
const PREVIEW_IMAGE_HEIGHT: i32 = 216; // 16:9
const THUMB_WIDTH: i32 = 144;
const THUMB_HEIGHT: i32 = THUMB_WIDTH * 3 / 4; // 108, 4:3 aspect
const SPACING: i32 = 12;
const BATCH_SIZE: usize = 20;

#[derive(Clone, Debug)]
struct Wallpaper {
    path: String,
}

pub fn build_page() -> gtk4::Widget {
    let settings = gio::Settings::new("org.erikreider.swaysettings");

    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    content_box.set_margin_top(24);
    content_box.set_margin_bottom(24);
    content_box.set_margin_start(24);
    content_box.set_margin_end(24);

    // Preview image
    let (preview_widget, preview) = build_preview_image(&settings);
    content_box.append(&preview_widget);

    // Scale mode combo in a PreferencesGroup
    let scale_group = build_scale_mode_row(&settings, &preview);
    content_box.append(&scale_group);

    // Wallpaper grid sections
    let wallpaper_box = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    content_box.append(&wallpaper_box);

    let load_queue: Rc<RefCell<VecDeque<gtk4::Picture>>> = Rc::new(RefCell::new(VecDeque::new()));
    let user_flow_box: Rc<RefCell<Option<gtk4::FlowBox>>> = Rc::new(RefCell::new(None));
    let sys_flow_box: Rc<RefCell<Option<gtk4::FlowBox>>> = Rc::new(RefCell::new(None));

    let rebuild = {
        let settings = settings.clone();
        let wallpaper_box = wallpaper_box.clone();
        let load_queue = load_queue.clone();
        let user_flow_box = user_flow_box.clone();
        let sys_flow_box = sys_flow_box.clone();
        let preview = preview.clone();
        move || {
            while let Some(child) = wallpaper_box.first_child() {
                wallpaper_box.remove(&child);
            }
            load_queue.borrow_mut().clear();

            let current_path = utils::get_wallpaper_gschema(&settings);

            // User wallpapers
            let user_wps = get_user_wallpapers(&settings);
            let (user_section, ufb) = build_wallpaper_section(
                "User Wallpapers",
                &user_wps,
                &current_path,
                true,
                &settings,
                &load_queue,
                &preview,
            );
            wallpaper_box.append(&user_section);
            *user_flow_box.borrow_mut() = ufb;

            // System wallpapers
            let sys_wps = get_system_wallpapers();
            let (sys_section, sfb) = build_wallpaper_section(
                "System Wallpapers",
                &sys_wps,
                &current_path,
                false,
                &settings,
                &load_queue,
                &preview,
            );
            wallpaper_box.append(&sys_section);
            *sys_flow_box.borrow_mut() = sfb;

            load_batched_images(load_queue.clone());
            refresh_preview(&preview, &settings);
        }
    };

    rebuild();

    {
        let rebuild = rebuild.clone();
        settings.connect_changed(Some(SETTINGS_USER_WALLPAPERS), move |_, _| {
            rebuild();
        });
    }
    {
        let rebuild = rebuild.clone();
        settings.connect_changed(Some(SETTINGS_WALLPAPER_SCALING_MODE), move |_, _| {
            rebuild();
        });
    }
    {
        let rebuild = rebuild.clone();
        settings.connect_changed(Some(SETTINGS_WALLPAPER_PATH), move |_, _| {
            rebuild();
        });
    }

    let clamp = libadwaita::Clamp::builder()
        .maximum_size(800)
        .tightening_threshold(600)
        .child(&content_box)
        .build();

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();

    scrolled.upcast()
}

fn build_preview_image(settings: &gio::Settings) -> (gtk4::Widget, gtk4::Picture) {
    let picture = gtk4::Picture::new();
    picture.set_content_fit(gtk4::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.add_css_class("thumbnail-image");

    // Wrap in a fixed-size box so the picture doesn't expand beyond intended dimensions
    let frame = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    frame.set_halign(gtk4::Align::Center);
    frame.set_valign(gtk4::Align::Start);
    frame.set_size_request(PREVIEW_IMAGE_WIDTH, PREVIEW_IMAGE_HEIGHT);
    frame.set_overflow(gtk4::Overflow::Hidden);
    frame.append(&picture);

    refresh_preview(&picture, settings);

    (frame.upcast(), picture)
}

fn refresh_preview(picture: &gtk4::Picture, settings: &gio::Settings) {
    let wallpaper_path = utils::Config::default_path().to_path_buf();
    load_image_async(picture, &wallpaper_path.to_string_lossy(), true);
}

fn build_scale_mode_row(
    settings: &gio::Settings,
    preview: &gtk4::Picture,
) -> libadwaita::PreferencesGroup {
    let modes = [ScaleMode::Fill, ScaleMode::Stretch, ScaleMode::Fit, ScaleMode::Center];

    let pref_group = libadwaita::PreferencesGroup::new();
    let combo_row = libadwaita::ComboRow::builder()
        .title("Scaling Mode")
        .build();

    let model = gio::ListStore::new::<gtk4::StringObject>();
    let current = utils::get_scale_mode_gschema(settings);
    let mut selected_idx = 0u32;
    for (i, mode) in modes.iter().enumerate() {
        model.append(&gtk4::StringObject::new(mode.to_title()));
        if *mode == current {
            selected_idx = i as u32;
        }
    }

    combo_row.set_model(Some(&model));
    combo_row.set_selected(selected_idx);

    let settings_clone = settings.clone();
    let preview_clone = preview.clone();
    combo_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if idx < modes.len() {
            let mode = modes[idx];
            let _ = functions::set_gsetting(
                &settings_clone,
                SETTINGS_WALLPAPER_SCALING_MODE,
                &glib::Variant::from(mode as i32),
            );
            preview_clone.set_content_fit(mode.to_content_fit());

            if let Some(app) = utils::wallpaper_application() {
                let wp_path = utils::get_wallpaper_gschema(&settings_clone)
                    .unwrap_or_default();
                let variant = glib::Variant::from((wp_path.as_str(), mode as i32, "#FFFFFF"));
                app.activate_action(WALLPAPER_ACTION_NAME, Some(&variant));
            }
        }
    });

    pref_group.add(&combo_row);
    pref_group
}

fn build_wallpaper_section(
    title: &str,
    wallpapers: &[Wallpaper],
    current_path: &Option<String>,
    is_user: bool,
    settings: &gio::Settings,
    load_queue: &Rc<RefCell<VecDeque<gtk4::Picture>>>,
    preview: &gtk4::Picture,
) -> (gtk4::Box, Option<gtk4::FlowBox>) {
    let section = gtk4::Box::new(gtk4::Orientation::Vertical, SPACING);

    // Header: title + optional add button
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    let title_label = gtk4::Label::new(Some(title));
    title_label.add_css_class("title-4");
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_hexpand(true);
    header.append(&title_label);

    if is_user {
        let add_button = gtk4::Button::builder()
            .label("+ Add Picture…")
            .build();
        add_button.add_css_class("flat");
        let settings_add = settings.clone();
        add_button.connect_clicked(move |btn| {
            add_user_wallpaper(btn, &settings_add);
        });
        header.append(&add_button);
    }

    section.append(&header);

    if wallpapers.is_empty() {
        // For user wallpapers: just show the header with add button, no big placeholder
        // For system wallpapers: show a small notice
        if !is_user {
            let notice = gtk4::Label::new(Some("No wallpapers found"));
            notice.add_css_class("dim-label");
            notice.set_margin_top(12);
            notice.set_margin_bottom(12);
            section.append(&notice);
        }
        return (section, None);
    }

    let flow_box = gtk4::FlowBox::builder()
        .max_children_per_line(8)
        .min_children_per_line(1)
        .homogeneous(true)
        .halign(gtk4::Align::Center)
        .activate_on_single_click(true)
        .selection_mode(gtk4::SelectionMode::Single)
        .row_spacing(SPACING as u32)
        .column_spacing(SPACING as u32)
        .build();

    let settings_select = settings.clone();
    let preview_select = preview.clone();
    flow_box.connect_child_activated(move |_, child| {
        if let Some(overlay) = child.child() {
            if let Some(picture) = overlay.first_child() {
                if let Some(picture) = picture.downcast_ref::<gtk4::Picture>() {
                    if let Some(path) = picture.tooltip_text() {
                        functions::set_wallpaper(&path, &settings_select);
                        refresh_preview(&preview_select, &settings_select);
                    }
                }
            }
        }
    });

    section.append(&flow_box);

    for wp in wallpapers {
        let overlay = gtk4::Overlay::new();
        overlay.set_size_request(THUMB_WIDTH, THUMB_HEIGHT);
        overlay.set_overflow(gtk4::Overflow::Hidden);

        let picture = gtk4::Picture::new();
        picture.set_content_fit(gtk4::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.set_tooltip_text(Some(&wp.path));
        picture.add_css_class("thumbnail-image");

        overlay.set_child(Some(&picture));

        if is_user {
            let remove_btn = gtk4::Button::from_icon_name("window-close-symbolic");
            remove_btn.set_has_frame(false);
            remove_btn.add_css_class("circular");
            remove_btn.add_css_class("img-remove-button");
            remove_btn.set_halign(gtk4::Align::End);
            remove_btn.set_valign(gtk4::Align::Start);
            let settings_remove = settings.clone();
            let wp_path_remove = wp.path.clone();
            remove_btn.connect_clicked(move |_| {
                remove_user_wallpaper(&settings_remove, &wp_path_remove);
            });
            overlay.add_overlay(&remove_btn);
        }

        let f_child = gtk4::FlowBoxChild::new();
        f_child.set_child(Some(&overlay));
        f_child.set_hexpand(false);
        f_child.set_vexpand(false);
        f_child.add_css_class("background-flowbox-child");

        flow_box.append(&f_child);

        if current_path.as_deref() == Some(&wp.path) {
            flow_box.select_child(&f_child);
        }

        load_queue.borrow_mut().push_back(picture);
    }

    (section, Some(flow_box))
}

fn load_batched_images(queue: Rc<RefCell<VecDeque<gtk4::Picture>>>) {
    glib::idle_add_local(move || {
        let mut batch = Vec::new();
        {
            let mut q = queue.borrow_mut();
            for _ in 0..BATCH_SIZE {
                match q.pop_front() {
                    Some(pic) => batch.push(pic),
                    None => break,
                }
            }
        }

        if batch.is_empty() {
            return glib::ControlFlow::Break;
        }

        for picture in batch {
            if let Some(path) = picture.tooltip_text() {
                load_image_async(&picture, &path, false);
            }
        }

        glib::ControlFlow::Continue
    });
}

fn load_image_async(picture: &gtk4::Picture, path: &str, full_size: bool) {
    let path = path.to_string();
    let weak = glib::SendWeakRef::from(picture.downgrade());
    let context = glib::MainContext::default();

    std::thread::spawn(move || {
        let texture = load_texture(&path, full_size);
        context.invoke(move || {
            if let Some(picture) = weak.upgrade() {
                match texture {
                    Some(tex) => picture.set_paintable(Some(&tex)),
                    None => picture.set_paintable(None::<&gdk4::Texture>),
                }
            }
        });
    });
}

fn load_texture(path: &str, full_size: bool) -> Option<gdk4::Texture> {
    if !full_size {
        if let Ok(thumb_path) = functions::generate_thumbnail(path, false, 256) {
            if let Ok(tex) = gdk4::Texture::from_filename(&thumb_path) {
                return Some(tex);
            }
        }
    }

    gdk4::Texture::from_filename(path).ok()
}

fn add_user_wallpaper(button: &gtk4::Button, settings: &gio::Settings) {
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Select Image");

    let filter = gtk4::FileFilter::new();
    filter.add_mime_type("image/*");
    dialog.set_default_filter(Some(&filter));

    let settings = settings.clone();
    dialog.open(
        button
            .root()
            .and_then(|r| r.downcast::<gtk4::Window>().ok())
            .as_ref(),
        None::<&gio::Cancellable>,
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().to_string();
                    let mut paths = get_user_wallpaper_paths(&settings);
                    if !paths.contains(&path_str) {
                        paths.push(path_str);
                        let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
                        let _ = settings.set_strv(SETTINGS_USER_WALLPAPERS, refs);
                    }
                }
            }
        },
    );
}

fn remove_user_wallpaper(settings: &gio::Settings, path: &str) {
    let paths = get_user_wallpaper_paths(settings);
    let filtered: Vec<&str> = paths
        .iter()
        .filter(|p| p.as_str() != path)
        .map(|s| s.as_str())
        .collect();
    let _ = settings.set_strv(SETTINGS_USER_WALLPAPERS, filtered);
}

fn get_user_wallpaper_paths(settings: &gio::Settings) -> Vec<String> {
    settings
        .strv(SETTINGS_USER_WALLPAPERS)
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn get_user_wallpapers(settings: &gio::Settings) -> Vec<Wallpaper> {
    get_user_wallpaper_paths(settings)
        .into_iter()
        .map(|path| Wallpaper { path })
        .collect()
}

fn get_system_wallpapers() -> Vec<Wallpaper> {
    let mut dirs_to_scan: Vec<PathBuf> = vec![
        PathBuf::from("/usr/share/backgrounds"),
        PathBuf::from("/usr/share/wallpapers"),
        PathBuf::from("/usr/local/share/wallpapers"),
        PathBuf::from("/usr/local/share/backgrounds"),
    ];

    let supported_formats: Vec<String> = {
        let mut formats = vec!["jpg".to_string(), "jpeg".to_string(), "png".to_string()];
        for fmt in gdk_pixbuf::Pixbuf::formats() {
            for ext in fmt.extensions() {
                let ext_str = ext.to_string();
                if !formats.contains(&ext_str) {
                    formats.push(ext_str);
                }
            }
        }
        formats
    };

    let mut wallpapers = Vec::new();
    let mut i = 0;

    while i < dirs_to_scan.len() {
        let dir_path = dirs_to_scan[i].clone();
        i += 1;

        let _ = functions::walk_through_dir(&dir_path, |info, _dir| {
            let file_type = info.file_type();
            let name = info.name();

            match file_type {
                gio::FileType::Regular => {
                    if info.is_hidden() || info.is_backup() || info.is_symlink() {
                        return;
                    }
                    let name_str = name.to_string_lossy();
                    if let Some(ext) = Path::new(name_str.as_ref()).extension() {
                        let ext_lower = ext.to_string_lossy().to_lowercase();
                        if supported_formats.contains(&ext_lower) {
                            let wp_path = dir_path.join(name_str.as_ref());
                            wallpapers.push(Wallpaper {
                                path: wp_path.to_string_lossy().to_string(),
                            });
                        }
                    }
                }
                gio::FileType::Directory => {
                    dirs_to_scan.push(dir_path.join(name.to_string_lossy().as_ref()));
                }
                _ => {}
            }
        });
    }

    wallpapers
}
