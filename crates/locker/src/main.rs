use std::cell::{Cell, RefCell};
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::*;

mod fingerprint;
mod lock_data;
mod locker_window;
mod pam;

use fingerprint::FingerprintManager;
use locker_window::LockerWindow;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Detach from terminal
    #[arg(long)]
    daemonize: bool,

    /// Debug: don't use session lock, opens in a regular window
    #[arg(long = "debug-do-not-lock")]
    debug_do_not_lock: bool,
}

// Global state accessed by window callbacks
thread_local! {
    static SHOULD_LOCK: Cell<bool> = Cell::new(true);
    static APP: RefCell<Option<libadwaita::Application>> = RefCell::new(None);
    static WINDOWS: RefCell<Vec<LockerWindow>> = RefCell::new(Vec::new());
    static FINGERPRINT_INITIALIZED: Cell<bool> = Cell::new(false);
    static PARENT_PID: Cell<libc::pid_t> = Cell::new(-1);
    static HOLD_GUARD: RefCell<Option<gio::ApplicationHoldGuard>> = RefCell::new(None);
}

pub fn should_lock() -> bool {
    SHOULD_LOCK.with(|c| c.get())
}

pub fn fingerprint_initialized() -> bool {
    FINGERPRINT_INITIALIZED.with(|c| c.get())
}

pub fn set_fingerprint_initialized(val: bool) {
    FINGERPRINT_INITIALIZED.with(|c| c.set(val));
}

pub fn do_unlock() {
    FingerprintManager::get_instance().release_device();
    APP.with(|cell| {
        if let Some(ref app) = *cell.borrow() {
            if should_lock() {
                // In session-lock mode, just quit (session lock will be unlocked)
                app.quit();
            } else {
                // Debug mode
                app.quit();
            }
        }
    });
}

/// Time tracker that fires callbacks at minute boundaries, like the Vala `TimeObj`.
struct TimeObj {
    time: RefCell<String>,
    date: RefCell<String>,
}

impl TimeObj {
    fn new() -> Rc<Self> {
        let obj = Rc::new(Self {
            time: RefCell::new(String::new()),
            date: RefCell::new(String::new()),
        });
        obj.update();
        obj
    }

    fn update(&self) {
        let now = chrono::Local::now();
        *self.time.borrow_mut() = now.format("%H:%M").to_string();
        *self.date.borrow_mut() = now.format("%d %b").to_string();
    }

    fn time(&self) -> String {
        self.time.borrow().clone()
    }

    fn date(&self) -> String {
        self.date.borrow().clone()
    }

    fn schedule(self: &Rc<Self>, windows: &Rc<RefCell<Vec<LockerWindow>>>) {
        let time_obj = self.clone();
        let windows = windows.clone();

        // Compute ms until the next minute boundary
        let now = chrono::Local::now();
        let sec = now.format("%S").to_string().parse::<u32>().unwrap_or(0);
        let usec = now.timestamp_subsec_micros();
        let delay_ms = (60 - sec) * 1000 - usec / 1000;

        glib::timeout_add_local_once(
            std::time::Duration::from_millis(delay_ms as u64),
            move || {
                time_obj.update();
                let t = time_obj.time();
                let d = time_obj.date();
                for win in windows.borrow().iter() {
                    win.set_date_time(&t, &d);
                }
                time_obj.schedule(&windows);
            },
        );
    }
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    SHOULD_LOCK.with(|c| c.set(!cli.debug_do_not_lock));

    if cli.daemonize {
        daemonize();
    }

    gtk4::init().expect("Failed to init GTK");
    let _ = libadwaita::init();

    swaysettings_core::resources::init_resources();
    swaysettings_core::resources::load_css(
        "/org/erikreider/swaysettings/style/locker.css",
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );

    // Add icon resource path
    if let Some(display) = gdk4::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        theme.add_resource_path("/org/erikreider/swaysettings/icons");
    }

    let settings = gio::Settings::new("org.erikreider.swaysettings");

    let app = libadwaita::Application::new(
        Some("org.erikreider.swaysettings-locker"),
        gio::ApplicationFlags::FLAGS_NONE,
    );

    APP.with(|cell| cell.replace(Some(app.clone())));

    let activated = Rc::new(Cell::new(false));
    let settings_clone = settings.clone();

    app.connect_activate(move |app| {
        if activated.get() {
            return;
        }
        activated.set(true);
        HOLD_GUARD.with(|cell| cell.replace(Some(app.hold())));
        init(app, &settings_clone);
    });

    // Register and check for remote instance
    if let Err(e) = app.register(None::<&gio::Cancellable>) {
        log::error!("Failed to register application: {}", e);
        std::process::exit(1);
    }

    if app.is_remote() {
        // Another instance is already running, signal daemon and exit
        signal_daemon();
        return;
    }

    app.run_with_args::<&str>(&[]);
}

fn init(app: &libadwaita::Application, settings: &gio::Settings) {
    let display = match gdk4::Display::default() {
        Some(display) => display,
        None => {
            log::error!("No display found");
            return;
        }
    };

    let monitors = display.monitors();
    let windows: Rc<RefCell<Vec<LockerWindow>>> = Rc::new(RefCell::new(Vec::new()));

    // Create windows for all monitors
    for i in 0..monitors.n_items() {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk4::Monitor>() {
            let win = LockerWindow::new(app, &monitor);
            win.load_content(settings);
            windows.borrow_mut().push(win);
        }
    }

    // Set up time updates
    let time_obj = TimeObj::new();
    let t = time_obj.time();
    let d = time_obj.date();
    for win in windows.borrow().iter() {
        win.set_date_time(&t, &d);
    }
    time_obj.schedule(&windows);

    // Setup fingerprint UI on first window
    if let Some(first_win) = windows.borrow().first() {
        first_win.setup_fingerprint_ui();

        // Connect to app shutdown for cleanup
        let app_clone = app.clone();
        app_clone.connect_shutdown(|_| {
            FingerprintManager::get_instance().release_device();
        });
    }

    // Monitor hotplug
    let app_for_hotplug = app.clone();
    let settings_for_hotplug = settings.clone();
    let windows_for_hotplug = windows.clone();
    monitors.connect_items_changed(move |monitors, position, removed, added| {
        let mut wins = windows_for_hotplug.borrow_mut();

        // Remove windows for disconnected monitors
        for _ in 0..removed {
            if (position as usize) < wins.len() {
                let win = wins.remove(position as usize);
                win.close();
            }
        }

        // Add windows for new monitors
        for i in 0..added {
            let idx = position + i;
            if let Some(monitor) = monitors.item(idx).and_downcast::<gdk4::Monitor>() {
                let win = LockerWindow::new(&app_for_hotplug, &monitor);
                win.load_content(&settings_for_hotplug);
                wins.insert(idx as usize, win.clone());
                if should_lock() {
                    // In session lock mode, present is handled by session lock
                    // For now, just present
                    win.present();
                } else {
                    win.present();
                }
            }
        }
    });

    if should_lock() {
        // Session lock mode - present all windows
        locked(&windows);
        for win in windows.borrow().iter() {
            win.present();
        }
    } else {
        // Debug mode - regular windows
        locked(&windows);
        let app_for_close = app.clone();
        for win in windows.borrow().iter() {
            let app_close = app_for_close.clone();
            win.connect_close_request(move |_| {
                app_close.quit();
                glib::Propagation::Proceed
            });
            win.present();
        }
    }

    // Store windows globally
    WINDOWS.with(|cell| {
        *cell.borrow_mut() = windows.borrow().clone();
    });
}

fn locked(windows: &Rc<RefCell<Vec<LockerWindow>>>) {
    // Signal daemon once all windows are mapped
    let n_items = windows.borrow().len();
    if n_items == 0 {
        signal_daemon();
        return;
    }

    let count = Rc::new(Cell::new(n_items as i32));

    for win in windows.borrow().iter() {
        if win.is_mapped() && win.is_realized() {
            let prev = count.get();
            count.set(prev - 1);
            continue;
        }

        let count_clone = count.clone();
        win.connect_map(move |_win| {
            let prev = count_clone.get();
            count_clone.set(prev - 1);
            if prev - 1 <= 0 {
                signal_daemon();
            }
        });
    }

    // If all were already mapped
    if count.get() <= 0 {
        signal_daemon();
        return;
    }

    // Fallback timeout
    glib::timeout_add_seconds_local_once(1, || {
        signal_daemon();
    });
}

// --- Daemonization logic ---

fn daemonize() {
    unsafe {
        let parent_pid = libc::getpid();
        PARENT_PID.with(|c| c.set(parent_pid));

        // Set up USR2 signal handler
        libc::signal(libc::SIGUSR2, sig_handler as *const () as libc::sighandler_t);

        match libc::fork() {
            -1 => {
                eprintln!("Fork PID error");
                std::process::exit(1);
            }
            0 => {
                // Child continues
            }
            _child_pid => {
                // Parent waits for USR2
                let mut sig_set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut sig_set);
                libc::sigaddset(&mut sig_set, libc::SIGUSR2);
                libc::sigprocmask(libc::SIG_BLOCK, &sig_set, std::ptr::null_mut());
                let mut sig: i32 = 0;
                if libc::sigwait(&sig_set, &mut sig) != 0 {
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
        }

        if libc::setsid() < 0 {
            eprintln!("setsid error");
            std::process::exit(1);
        }

        match libc::fork() {
            -1 => {
                eprintln!("Fork 2 PID error");
                std::process::exit(1);
            }
            0 => {
                // Grandchild continues as daemon
            }
            _ => {
                std::process::exit(0);
            }
        }
    }
}

extern "C" fn sig_handler(_sig: i32) {
    // Signal received (USR2)
}

fn signal_daemon() {
    let parent_pid = PARENT_PID.with(|c| c.get());
    if parent_pid > 0 {
        unsafe {
            libc::kill(parent_pid, libc::SIGUSR2);
        }
    }
}
