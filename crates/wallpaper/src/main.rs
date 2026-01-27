use std::cell::RefCell;
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::prelude::*;

use swaysettings_core::{constants, utils, Config, ScaleMode};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Image path
    #[arg(short = 'i', long = "image")]
    image: Option<String>,

    /// Image scaling mode
    #[arg(short = 'm', long = "mode")]
    mode: Option<String>,

    /// Background color (#rrggbb)
    #[arg(short = 'c', long = "color")]
    color: Option<String>,

    /// List scaling modes
    #[arg(short = 'l', long = "list-modes")]
    list_modes: bool,

    /// Debug: disable layer shell
    #[arg(long = "no-layer-shell")]
    no_layer_shell: bool,
}

struct WallpaperWindow {
    window: gtk4::Window,
    overlay: gtk4::Overlay,
    current: gtk4::Picture,
    previous: gtk4::Picture,
    animation: libadwaita::TimedAnimation,
    config: RefCell<Config>,
}

impl WallpaperWindow {
    fn new(app: &libadwaita::Application, monitor: &gdk4::Monitor, _use_layer_shell: bool) -> Self {
        let window = gtk4::Window::new();
        window.set_application(Some(app));
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_focusable(false);

        let overlay = gtk4::Overlay::new();
        let previous = gtk4::Picture::new();
        let current = gtk4::Picture::new();

        previous.set_content_fit(gtk4::ContentFit::Cover);
        current.set_content_fit(gtk4::ContentFit::Cover);

        overlay.add_overlay(&previous);
        overlay.add_overlay(&current);
        window.set_child(Some(&overlay));

        window.set_default_size(monitor.geometry().width(), monitor.geometry().height());
        window.fullscreen();

        let current_clone = current.clone();
        let previous_clone = previous.clone();
        let animation_target = libadwaita::CallbackAnimationTarget::new(move |value| {
            current_clone.set_opacity(value as f64);
            previous_clone.set_opacity(1.0 - value as f64);
        });
        let animation = libadwaita::TimedAnimation::new(&window, 0.0, 1.0, 500, animation_target);

        Self {
            window,
            overlay,
            current,
            previous,
            animation,
            config: RefCell::new(Config::default()),
        }
    }

    fn set_config(&self, config: Config) {
        let mut stored = self.config.borrow_mut();
        if stored.cmp(&config) {
            return;
        }
        let old_texture = self.current.paintable();
        if let Some(old) = old_texture {
            self.previous.set_paintable(Some(&old));
            self.previous.set_opacity(1.0);
        }

        if config.path.is_empty() {
            let rgba = config.get_color();
            let texture = texture_from_rgba(&rgba);
            self.current.set_paintable(Some(&texture));
        } else if let Ok(texture) = gdk4::Texture::from_file(&gio::File::for_path(&config.path)) {
            self.current.set_paintable(Some(&texture));
            self.current.set_content_fit(config.scale_mode.to_content_fit());
        }

        *stored = config;
        self.animation.play();
    }
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();
    if cli.list_modes {
        println!("Available scaling modes: fill, stretch, fit, center");
        return;
    }

    gtk4::init().expect("Failed to init GTK");
    let _ = libadwaita::init();

    let settings = gio::Settings::new("org.erikreider.swaysettings");

    let mut config = Config::default();
    if let Some(path) = cli.image.clone() {
        config.path = path;
    } else if let Some(path) = utils::get_wallpaper_gschema(&settings) {
        config.path = path;
    } else {
        config.path = Config::default_path().to_string_lossy().to_string();
    }

    if let Some(color) = cli.color.clone() {
        config.color = color;
    }

    config.scale_mode = if let Some(mode) = cli.mode.as_deref() {
        ScaleMode::parse_mode(Some(mode))
    } else {
        utils::get_scale_mode_gschema(&settings)
    };

    let app = libadwaita::Application::new(
        Some("org.erikreider.swaysettings-wallpaper"),
        gio::ApplicationFlags::FLAGS_NONE,
    );

    let config_state = Rc::new(RefCell::new(config));
    let windows: Rc<RefCell<Vec<WallpaperWindow>>> = Rc::new(RefCell::new(Vec::new()));

    let windows_action = windows.clone();
    let config_action = config_state.clone();
    let action = gio::SimpleAction::new(constants::WALLPAPER_ACTION_NAME, Some(&glib::VariantType::new(constants::WALLPAPER_ACTION_FORMAT).unwrap()));
    action.connect_activate(move |_, param| {
        if let Some(param) = param {
            if let Some((path, mode, color)) = param.get::<(String, i32, String)>() {
                let mut cfg = config_action.borrow_mut();
                cfg.path = path;
                cfg.scale_mode = ScaleMode::try_from(mode).unwrap_or(ScaleMode::Fill);
                cfg.color = color;
                for window in windows_action.borrow().iter() {
                    window.set_config(cfg.clone());
                }
            }
        }
    });
    app.add_action(&action);

    let windows_activate = windows.clone();
    let config_activate = config_state.clone();
    let no_layer_shell = cli.no_layer_shell;

    app.connect_activate(move |app| {
        if let Some(display) = gdk4::Display::default() {
            let monitors = display.monitors();
            let use_layer_shell = !no_layer_shell;

            for i in 0..monitors.n_items() {
                if let Some(monitor) = monitors.item(i).and_downcast::<gdk4::Monitor>() {
                    let win = WallpaperWindow::new(app, &monitor, use_layer_shell);
                    win.window.present();
                    win.set_config(config_activate.borrow().clone());
                    windows_activate.borrow_mut().push(win);
                }
            }
        }
    });

    if let Err(err) = app.register(None::<&gio::Cancellable>) {
        eprintln!("Failed to register application: {err}");
    }

    if app.is_remote() {
        let cfg = config_state.borrow();
        let variant = glib::Variant::from((cfg.path.as_str(), cfg.scale_mode as i32, cfg.color.as_str()));
        app.activate_action(constants::WALLPAPER_ACTION_NAME, Some(&variant));
        return;
    }

    app.run();
}

fn texture_from_rgba(color: &gdk4::RGBA) -> gdk4::Texture {
    let pixel = [
        (color.red() * 255.0) as u8,
        (color.green() * 255.0) as u8,
        (color.blue() * 255.0) as u8,
        (color.alpha() * 255.0) as u8,
    ];
    let bytes = glib::Bytes::from(&pixel);
    let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        1,
        1,
        4,
    );
    gdk4::Texture::for_pixbuf(&pixbuf)
}
