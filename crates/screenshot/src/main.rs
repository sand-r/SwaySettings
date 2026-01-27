use std::cell::RefCell;
use std::process::Command;
use std::io::Write;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;

use swaysettings_core::constants;

#[derive(Clone)]
struct AppState {
    windows: Rc<RefCell<Vec<SelectionWindow>>>,
    settings: gio::Settings,
    app: libadwaita::Application,
}

#[derive(Clone)]
struct SelectionWindow {
    window: libadwaita::ApplicationWindow,
    drawing: gtk4::DrawingArea,
    monitor: gdk4::Monitor,
    state: Rc<RefCell<SelectionState>>, // shared for draw/gesture
}

#[derive(Default)]
struct SelectionState {
    start_x: f64,
    start_y: f64,
    offset_x: f64,
    offset_y: f64,
    active: bool,
}

fn main() {
    env_logger::init();

    gtk4::init().expect("Failed to init GTK");
    let _ = libadwaita::init();

    swaysettings_core::resources::init_resources();
    swaysettings_core::resources::load_css(
        "/org/erikreider/swaysettings/style/screenshot.css",
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let settings = gio::Settings::new("org.erikreider.swaysettings");

    let app = libadwaita::Application::new(
        Some("org.erikreider.swaysettings-screenshot"),
        gio::ApplicationFlags::FLAGS_NONE,
    );

    let state = AppState {
        windows: Rc::new(RefCell::new(Vec::new())),
        settings,
        app: app.clone(),
    };

    let state_clone = state.clone();
    app.connect_activate(move |app| {
        init_windows(app, &state_clone);
    });

    app.run();
}

fn init_windows(app: &libadwaita::Application, state: &AppState) {
    let display = match gdk4::Display::default() {
        Some(display) => display,
        None => return,
    };

    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk4::Monitor>() {
            let win = build_selection_window(app, &monitor, state.clone());
            win.window.present();
            state.windows.borrow_mut().push(win);
        }
    }
}

fn build_selection_window(
    app: &libadwaita::Application,
    monitor: &gdk4::Monitor,
    state: AppState,
) -> SelectionWindow {
    let window = libadwaita::ApplicationWindow::new(app);
    window.add_css_class("screenshot-window");

    window.set_decorated(false);
    window.set_resizable(false);
    window.set_default_size(monitor.geometry().width(), monitor.geometry().height());
    window.fullscreen();

    let drawing = gtk4::DrawingArea::new();
    drawing.set_hexpand(true);
    drawing.set_vexpand(true);

    let selection_state = Rc::new(RefCell::new(SelectionState::default()));
    let state_for_draw = selection_state.clone();
    let monitor_geo = monitor.geometry();

    drawing.set_draw_func(move |_, ctx, width, height| {
        draw_overlay(ctx, width, height, &state_for_draw.borrow(), &monitor_geo);
    });

    let drag = gtk4::GestureDrag::new();
    let state_for_drag = selection_state.clone();
    let monitor_clone = monitor.clone();

    drag.connect_drag_begin(move |_, x, y| {
        let mut selection = state_for_drag.borrow_mut();
        selection.start_x = x + monitor_clone.geometry().x() as f64;
        selection.start_y = y + monitor_clone.geometry().y() as f64;
        selection.offset_x = 0.0;
        selection.offset_y = 0.0;
        selection.active = true;
    });

    let state_for_update = selection_state.clone();
    drag.connect_drag_update(move |_, x, y| {
        let mut selection = state_for_update.borrow_mut();
        selection.offset_x = x;
        selection.offset_y = y;
        gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(0), || {});
    });

    let state_for_end = selection_state.clone();
    let state_end_clone = state.clone();
    let window_clone = window.clone();
    drag.connect_drag_end(move |_, offset_x, offset_y| {
        let mut selection = state_for_end.borrow_mut();
        selection.offset_x = offset_x;
        selection.offset_y = offset_y;
        if selection.offset_x.abs() > 5.0 && selection.offset_y.abs() > 5.0 {
            let rect = gdk4::Rectangle::new(
                selection.start_x as i32,
                selection.start_y as i32,
                selection.offset_x as i32,
                selection.offset_y as i32,
            );
            selection.active = false;
            for win in state_end_clone.windows.borrow().iter() {
                win.window.hide();
            }
            if let Some(texture) = grim_screenshot_rect(&rect) {
                show_preview(&state_end_clone, &texture);
            } else {
                state_end_clone.app.quit();
            }
        } else {
            selection.active = false;
            window_clone.queue_draw();
        }
    });

    drawing.add_controller(drag);
    window.set_child(Some(&drawing));

    SelectionWindow {
        window,
        drawing,
        monitor: monitor.clone(),
        state: selection_state,
    }
}

fn draw_overlay(
    ctx: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    state: &SelectionState,
    monitor_geo: &gdk4::Rectangle,
) {
    ctx.set_source_rgba(0.0, 0.0, 0.0, 0.5);
    ctx.rectangle(0.0, 0.0, width as f64, height as f64);
    ctx.fill().unwrap();

    if !state.active {
        return;
    }

    let x = state.start_x - monitor_geo.x() as f64;
    let y = state.start_y - monitor_geo.y() as f64;
    ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    ctx.set_operator(gtk4::cairo::Operator::Clear);
    ctx.rectangle(x, y, state.offset_x, state.offset_y);
    ctx.fill().unwrap();
    ctx.set_operator(gtk4::cairo::Operator::Over);

    ctx.set_source_rgba(1.0, 1.0, 1.0, 0.6);
    ctx.set_line_width(2.0);
    ctx.rectangle(x, y, state.offset_x, state.offset_y);
    ctx.stroke().unwrap();
}

fn grim_screenshot_rect(rect: &gdk4::Rectangle) -> Option<gdk4::Texture> {
    let geometry = format!("{},{} {}x{}", rect.x(), rect.y(), rect.width(), rect.height());
    let output = Command::new("grim")
        .arg("-t")
        .arg("png")
        .arg("-g")
        .arg(geometry)
        .arg("-")
        .output()
        .ok()?;

    if !output.status.success() {
        eprintln!("grim failed");
        return None;
    }

    let bytes = glib::Bytes::from(&output.stdout);
    gdk4::Texture::from_bytes(&bytes).ok()
}

fn show_preview(state: &AppState, texture: &gdk4::Texture) {
    let preview = libadwaita::Window::new();
    preview.set_title(Some("Screenshot"));
    preview.set_default_size(800, 600);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let picture = gtk4::Picture::new();
    picture.set_paintable(Some(texture));
    picture.set_content_fit(gtk4::ContentFit::Contain);
    vbox.append(&picture);

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let save_btn = gtk4::Button::with_label("Save");
    let save_as_btn = gtk4::Button::with_label("Save As");
    let copy_btn = gtk4::Button::with_label("Copy");
    let edit_btn = gtk4::Button::with_label("Edit");
    let close_btn = gtk4::Button::with_label("Close");

    buttons.append(&copy_btn);
    buttons.append(&save_as_btn);
    buttons.append(&save_btn);
    buttons.append(&edit_btn);
    buttons.append(&close_btn);
    vbox.append(&buttons);

    preview.set_child(Some(&vbox));

    let settings = state.settings.clone();
    let app = state.app.clone();
    let preview_clone = preview.clone();
    let texture_clone = texture.clone();

    save_btn.connect_clicked(move |_| {
        save_to_default(&settings, &texture_clone, &preview_clone, true);
    });

    let settings_save_as = state.settings.clone();
    let preview_save_as = preview.clone();
    let texture_save_as = texture.clone();
    save_as_btn.connect_clicked(move |_| {
        save_as_dialog(&settings_save_as, &texture_save_as, &preview_save_as);
    });

    let texture_copy = texture.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some(display) = gdk4::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_texture(&texture_copy);
        }
    });

    let settings_edit = state.settings.clone();
    let texture_edit = texture.clone();
    edit_btn.connect_clicked(move |_| {
        edit_screenshot(&settings_edit, &texture_edit);
    });

    let preview_close = preview.clone();
    close_btn.connect_clicked(move |_| {
        preview_close.close();
        app.quit();
    });

    preview.present();
}

fn screenshot_filename() -> String {
    let now = chrono::Local::now();
    format!("Screenshot from {}.png", now.format("%Y-%m-%d %H-%M-%S"))
}

fn initial_folder(settings: &gio::Settings) -> Option<gio::File> {
    let variant = settings.value(constants::SETTINGS_SCREENSHOT_SAVE_DEST);
    let path = variant.get::<String>()?;
    let mut expanded = path.clone();
    if expanded.starts_with("~/") {
        expanded = format!("{}{}", glib::home_dir().to_string_lossy(), &expanded[1..]);
    }
    let file = gio::File::for_path(expanded);
    if file.query_exists(None::<&gio::Cancellable>) {
        Some(file)
    } else {
        None
    }
}

fn save_to_default(
    settings: &gio::Settings,
    texture: &gdk4::Texture,
    preview: &impl IsA<gtk4::Window>,
    close_if_needed: bool,
) {
    let folder = initial_folder(settings);
    let name = screenshot_filename();
    let file_path = if let Some(folder) = folder {
        folder.path().map(|p| p.join(&name))
    } else {
        Some(std::path::PathBuf::from(name))
    };

    if let Some(path) = file_path {
        if let Err(err) = texture.save_to_png(path.to_string_lossy().as_ref()) {
            eprintln!("Failed to save screenshot: {err}");
        }
    }

    let exit_on_save = settings.boolean(constants::SETTINGS_SCREENSHOT_EXIT_ON_SAVE);
    if exit_on_save && close_if_needed {
        preview.close();
    }
}

fn save_as_dialog(settings: &gio::Settings, texture: &gdk4::Texture, preview: &impl IsA<gtk4::Window>) {
    let dialog = gtk4::FileDialog::new();
    dialog.set_modal(true);
    dialog.set_title("Save Screenshot");
    if let Some(folder) = initial_folder(settings) {
        dialog.set_initial_folder(Some(&folder));
    }
    dialog.set_initial_name(Some(&screenshot_filename()));

    let texture = texture.clone();
    let preview = preview.clone();
    dialog.save(Some(&preview), None::<&gio::Cancellable>, move |res| {
        match res {
            Ok(file) => {
                if let Some(path) = file.path() {
                    if let Err(err) = texture.save_to_png(path.to_string_lossy().as_ref()) {
                        eprintln!("Failed to save screenshot: {err}");
                    }
                }
            }
            Err(err) => {
                eprintln!("Save dialog error: {err}");
            }
        }
    });
}

fn edit_screenshot(settings: &gio::Settings, texture: &gdk4::Texture) {
    let cmd_variant = settings.value(constants::SETTINGS_SCREENSHOT_EDIT_CMD);
    let cmd = match cmd_variant.get::<String>() {
        Some(cmd) => cmd,
        None => return,
    };

    let bytes = texture.save_to_png_bytes();

    let cmd = if cmd.starts_with("~/") {
        format!("{}{}", glib::home_dir().to_string_lossy(), &cmd[1..])
    } else {
        cmd
    };

    let argv = shell_words::split(&cmd).unwrap_or_else(|_| vec![cmd.clone()]);
    let mut command = Command::new(&argv[0]);
    if argv.len() > 1 {
        command.args(&argv[1..]);
    }
    if let Ok(mut child) = command.stdin(std::process::Stdio::piped()).spawn() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(bytes.as_ref());
        }
    }
}
