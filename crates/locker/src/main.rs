use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::*;

mod fingerprint;
mod lock_data;
mod locker_window;
mod pam;

use fingerprint::FingerprintManager;
use gtk4_session_lock::Instance as SessionLockInstance;
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

    /// Debug builds only: automatically unlock a real session lock after N seconds
    #[cfg(debug_assertions)]
    #[arg(long = "debug-auto-unlock-seconds", value_name = "SECONDS")]
    debug_auto_unlock_seconds: Option<u32>,
}

// Global state accessed by window callbacks
thread_local! {
    static SHOULD_LOCK: Cell<bool> = const { Cell::new(true) };
    static APP: RefCell<Option<libadwaita::Application>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<Vec<LockerWindow>> = const { RefCell::new(Vec::new()) };
    static SESSION_LOCK: RefCell<Option<SessionLockInstance>> = const { RefCell::new(None) };
    static LOCK_ACQUIRED: Cell<bool> = const { Cell::new(false) };
    static FINGERPRINT_INITIALIZED: Cell<bool> = const { Cell::new(false) };
    static HOLD_GUARD: RefCell<Option<gio::ApplicationHoldGuard>> = const { RefCell::new(None) };
    static DAEMON_NOTIFY_FD: Cell<i32> = const { Cell::new(-1) };
    static DAEMON_NOTIFIED: Cell<bool> = const { Cell::new(false) };
    static EXIT_STATUS: Cell<i32> = const { Cell::new(0) };
    #[cfg(debug_assertions)]
    static DEBUG_AUTO_UNLOCK_SECONDS: Cell<Option<u32>> = const { Cell::new(None) };
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

    if should_lock() {
        let lock = SESSION_LOCK.with(|cell| cell.borrow().clone());
        match lock {
            Some(lock) if lock.is_locked() => lock.unlock(),
            Some(_) => log::error!("Refusing to quit: the session lock is not acquired"),
            None => log::error!("Refusing to quit: no session-lock instance exists"),
        }
        return;
    }

    APP.with(|cell| {
        if let Some(ref app) = *cell.borrow() {
            app.quit();
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

    fn schedule(self: &Rc<Self>) {
        let time_obj = self.clone();

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
                WINDOWS.with(|windows| {
                    for win in windows.borrow().iter() {
                        win.set_date_time(&t, &d);
                    }
                });
                time_obj.schedule();
            },
        );
    }
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    SHOULD_LOCK.with(|c| c.set(!cli.debug_do_not_lock));
    #[cfg(debug_assertions)]
    DEBUG_AUTO_UNLOCK_SECONDS.with(|seconds| seconds.set(cli.debug_auto_unlock_seconds));

    if cli.daemonize {
        if let Err(error) = daemonize() {
            eprintln!("Failed to daemonize: {error}");
            std::process::exit(1);
        }
    }

    gtk4::init().expect("Failed to init GTK");
    let _ = libadwaita::init();

    if should_lock() && !gtk4_session_lock::is_supported() {
        eprintln!("The Wayland compositor does not support ext-session-lock-v1");
        notify_daemon(false);
        std::process::exit(1);
    }

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

    app.connect_shutdown(|_| {
        FingerprintManager::get_instance().release_device();
    });

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
        notify_daemon(false);
        std::process::exit(1);
    }

    if app.is_remote() {
        // Another instance is already running, notify a waiting daemon parent
        // and exit without disturbing the active lock.
        notify_daemon(true);
        return;
    }

    app.run_with_args::<&str>(&["swaysettings-locker"]);

    // If the application exited before reporting readiness, make a daemonizing
    // parent fail rather than leaving it blocked forever.
    notify_daemon(false);
    let status = EXIT_STATUS.with(Cell::get);
    if status != 0 {
        std::process::exit(status);
    }
}

fn init(app: &libadwaita::Application, settings: &gio::Settings) {
    WINDOWS.with(|windows| windows.borrow_mut().clear());
    LOCK_ACQUIRED.with(|acquired| acquired.set(false));

    let time_obj = TimeObj::new();
    time_obj.schedule();

    if should_lock() {
        init_session_lock(app, settings, &time_obj);
    } else {
        init_debug_windows(app, settings, &time_obj);
    }
}

fn init_session_lock(
    app: &libadwaita::Application,
    settings: &gio::Settings,
    time_obj: &Rc<TimeObj>,
) {
    let lock = SessionLockInstance::new();

    lock.connect_locked(|_| {
        log::info!("Wayland session lock acquired");
        LOCK_ACQUIRED.with(|acquired| acquired.set(true));
        schedule_debug_auto_unlock();
        maybe_notify_daemon_ready();
    });

    let app_failed = app.clone();
    lock.connect_failed(move |_| {
        fail_lock(
            &app_failed,
            "The compositor refused the Wayland session lock",
        );
    });

    let app_unlocked = app.clone();
    lock.connect_unlocked(move |_| {
        log::info!("Wayland session unlocked");
        FingerprintManager::get_instance().release_device();
        LOCK_ACQUIRED.with(|acquired| acquired.set(false));
        WINDOWS.with(|windows| windows.borrow_mut().clear());
        SESSION_LOCK.with(|cell| cell.replace(None));
        HOLD_GUARD.with(|cell| cell.replace(None));
        app_unlocked.quit();
    });

    let app_monitor = app.clone();
    let settings_monitor = settings.clone();
    let time_monitor = time_obj.clone();
    lock.connect_monitor(move |lock, monitor| {
        log::info!("Compositor requested a session-lock surface for a monitor");
        let window = create_lock_window(&app_monitor, &settings_monitor, monitor, &time_monitor);

        // This is the only operation that presents a window in secure mode.
        // It gives the window an ext-session-lock surface and lets the
        // compositor size and map it for the supplied output.
        lock.assign_window_to_monitor(&window, monitor);
    });

    SESSION_LOCK.with(|cell| cell.replace(Some(lock.clone())));

    if !lock.lock() {
        fail_lock(app, "Failed to start the Wayland session-lock request");
    }
}

fn init_debug_windows(
    app: &libadwaita::Application,
    settings: &gio::Settings,
    time_obj: &Rc<TimeObj>,
) {
    let display = match gdk4::Display::default() {
        Some(display) => display,
        None => {
            fail_lock(app, "No display found");
            return;
        }
    };

    let monitors = display.monitors();
    if monitors.n_items() == 0 {
        fail_lock(app, "No monitors found");
        return;
    }

    for i in 0..monitors.n_items() {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk4::Monitor>() {
            let window = create_lock_window(app, settings, &monitor, time_obj);
            configure_debug_window(&window, app);
            window.present();
        }
    }

    let app_hotplug = app.clone();
    let settings_hotplug = settings.clone();
    let time_hotplug = time_obj.clone();
    monitors.connect_items_changed(move |monitors, position, removed, added| {
        for _ in 0..removed {
            let window = WINDOWS.with(|windows| windows.borrow().get(position as usize).cloned());
            if let Some(window) = window {
                window.destroy();
            }
        }

        for i in 0..added {
            let index = position + i;
            if let Some(monitor) = monitors.item(index).and_downcast::<gdk4::Monitor>() {
                let window =
                    create_lock_window(&app_hotplug, &settings_hotplug, &monitor, &time_hotplug);
                configure_debug_window(&window, &app_hotplug);
                window.present();
            }
        }
    });
}

fn configure_debug_window(window: &LockerWindow, app: &libadwaita::Application) {
    let app = app.clone();
    window.connect_close_request(move |_| {
        app.quit();
        glib::Propagation::Proceed
    });
}

fn create_lock_window(
    app: &libadwaita::Application,
    settings: &gio::Settings,
    monitor: &gdk4::Monitor,
    time_obj: &TimeObj,
) -> LockerWindow {
    let window = LockerWindow::new(app, monitor);
    window.load_content(settings);
    window.set_date_time(&time_obj.time(), &time_obj.date());

    let first = WINDOWS.with(|windows| windows.borrow().is_empty());
    if first {
        window.setup_fingerprint_ui();
    }

    let window_id = window.as_ptr() as usize;
    window.connect_destroy(move |_| {
        let (empty, new_first) = WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            let was_first = windows
                .first()
                .is_some_and(|window| window.as_ptr() as usize == window_id);
            windows.retain(|window| window.as_ptr() as usize != window_id);
            let new_first = was_first.then(|| windows.first().cloned()).flatten();
            (windows.is_empty(), new_first)
        });

        if empty {
            FingerprintManager::get_instance().release_device();
            set_fingerprint_initialized(false);
        } else if let Some(window) = new_first {
            window.setup_fingerprint_ui();
        }
    });

    window.connect_map(|_| {
        log::info!("Locker window mapped (secure={})", should_lock());
        maybe_notify_daemon_ready();
    });

    WINDOWS.with(|windows| windows.borrow_mut().push(window.clone()));
    window
}

fn maybe_notify_daemon_ready() {
    if DAEMON_NOTIFIED.with(Cell::get) {
        return;
    }

    if should_lock() && !LOCK_ACQUIRED.with(Cell::get) {
        return;
    }

    let expected = gdk4::Display::default()
        .map(|display| display.monitors().n_items() as usize)
        .unwrap_or(0);
    let mapped = WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .filter(|window| window.is_mapped() && window.is_realized())
            .count()
    });

    if expected > 0 && mapped >= expected {
        notify_daemon(true);
    }
}

fn fail_lock(app: &libadwaita::Application, message: &str) {
    if EXIT_STATUS.with(Cell::get) != 0 {
        return;
    }

    log::error!("{message}");
    EXIT_STATUS.with(|status| status.set(1));
    FingerprintManager::get_instance().release_device();
    notify_daemon(false);
    app.quit();
}

#[cfg(debug_assertions)]
fn schedule_debug_auto_unlock() {
    let seconds = DEBUG_AUTO_UNLOCK_SECONDS.with(Cell::take);
    let Some(seconds) = seconds else {
        return;
    };

    eprintln!(
        "WARNING: debug recovery is armed; the session will automatically unlock in {seconds} seconds"
    );
    glib::timeout_add_seconds_local_once(seconds, || {
        eprintln!("Debug recovery timer expired; unlocking the session");
        do_unlock();
    });
}

#[cfg(not(debug_assertions))]
fn schedule_debug_auto_unlock() {}

// --- Daemonization logic ---

fn daemonize() -> io::Result<()> {
    let mut pipe_fds = [-1; 2];

    unsafe {
        if libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }

        match libc::fork() {
            -1 => {
                libc::close(pipe_fds[0]);
                libc::close(pipe_fds[1]);
                return Err(io::Error::last_os_error());
            }
            0 => {
                // Daemon child: keep only the status writer.
                libc::close(pipe_fds[0]);
                DAEMON_NOTIFY_FD.with(|fd| fd.set(pipe_fds[1]));
            }
            _ => {
                // Original parent: wait until the grandchild reports that all
                // lock surfaces are mapped, or until every writer closes.
                libc::close(pipe_fds[1]);
                let mut status = 0u8;
                let read = loop {
                    let result =
                        libc::read(pipe_fds[0], &mut status as *mut u8 as *mut libc::c_void, 1);
                    if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    break result;
                };
                libc::close(pipe_fds[0]);
                libc::_exit(if read == 1 && status == 1 { 0 } else { 1 });
            }
        }

        if libc::setsid() < 0 {
            notify_daemon(false);
            return Err(io::Error::last_os_error());
        }

        match libc::fork() {
            -1 => {
                notify_daemon(false);
                Err(io::Error::last_os_error())
            }
            0 => Ok(()),
            _ => {
                // Intermediate child. The grandchild retains its own copy of
                // the status writer.
                libc::close(pipe_fds[1]);
                libc::_exit(0);
            }
        }
    }
}

fn notify_daemon(success: bool) {
    if DAEMON_NOTIFIED.with(|notified| notified.replace(true)) {
        return;
    }

    DAEMON_NOTIFY_FD.with(|fd| {
        let raw_fd = fd.replace(-1);
        if raw_fd < 0 {
            return;
        }

        let status = u8::from(success);
        unsafe {
            let _ = libc::write(raw_fd, &status as *const u8 as *const libc::c_void, 1);
            libc::close(raw_fd);
        }
    });
}
