use std::cell::RefCell;
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use glib::clone;
use gtk4::prelude::*;

mod locker_window;
use locker_window::LockerWindow;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Detach from terminal (no-op in this rewrite)
    #[arg(long)]
    daemonize: bool,

    /// Debug: don't use session lock (still a layer-shell overlay)
    #[arg(long = "debug-do-not-lock")]
    debug_do_not_lock: bool,
}

struct LockWindow {
    window: LockerWindow,
    entry: gtk4::Entry,
}

fn main() {
    env_logger::init();

    let _cli = Cli::parse();

    gtk4::init().expect("Failed to init GTK");
    let _ = libadwaita::init();

    swaysettings_core::resources::init_resources();
    swaysettings_core::resources::load_css(
        "/org/erikreider/swaysettings/style/locker.css",
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let app = libadwaita::Application::new(
        Some("org.erikreider.swaysettings-locker"),
        gio::ApplicationFlags::FLAGS_NONE,
    );

    app.connect_activate(move |app| {
        let app = app.clone();
        let display = match gdk4::Display::default() {
            Some(display) => display,
            None => return,
        };
        let monitors = display.monitors();
        let windows: Rc<RefCell<Vec<LockWindow>>> = Rc::new(RefCell::new(Vec::new()));

        for i in 0..monitors.n_items() {
            if let Some(monitor) = monitors.item(i).and_downcast::<gdk4::Monitor>() {
                let win = build_lock_window(&app, &monitor);
                win.window.present();
                windows.borrow_mut().push(win);
            }
        }

        let windows_clone = windows.clone();
        for lock_win in windows_clone.borrow().iter() {
            let windows_for_cb = windows_clone.clone();
            let app_for_cb = app.clone();
            lock_win.entry.connect_activate(move |entry| {
                let pwd = entry.text().to_string();
                if authenticate(&pwd) {
                    for win in windows_for_cb.borrow().iter() {
                        win.window.close();
                    }
                    app_for_cb.quit();
                } else {
                    entry.set_text("");
                    entry.add_css_class("error");
                }
            });
        }
    });

    app.run();
}

fn build_lock_window(app: &libadwaita::Application, monitor: &gdk4::Monitor) -> LockWindow {
    let window = LockerWindow::new(app);
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_focusable(true);

    window.set_default_size(monitor.geometry().width(), monitor.geometry().height());
    window.fullscreen();

    let entry = window.entry();
    entry.grab_focus();

    let time_label = window.time_label();
    let date_label = window.date_label();

    update_time_labels(&time_label, &date_label);
    glib::timeout_add_seconds_local(
        60,
        clone!(
            #[strong]
            time_label,
            #[strong]
            date_label,
            move || {
                update_time_labels(&time_label, &date_label);
                glib::ControlFlow::Continue
            }
        ),
    );

    LockWindow { window, entry }
}

fn update_time_labels(time_label: &gtk4::Label, date_label: &gtk4::Label) {
    let now = chrono::Local::now();
    time_label.set_text(&now.format("%H:%M").to_string());
    date_label.set_text(&now.format("%d %b").to_string());
}

fn authenticate(password: &str) -> bool {
    pam_authenticate_credentials("swaysettings-locker", &whoami::username(), password)
}

fn pam_authenticate_credentials(service: &str, username: &str, password: &str) -> bool {
    use std::ffi::CString;
    use std::ptr;

    unsafe {
        let service_c = match CString::new(service) {
            Ok(val) => val,
            Err(_) => return false,
        };
        let user_c = match CString::new(username) {
            Ok(val) => val,
            Err(_) => return false,
        };

        let mut handle: *mut PamHandle = ptr::null_mut();
        let pw = CString::new(password).unwrap_or_default();
        let data = PamConvData {
            password: pw.as_ptr(),
        };
        let conv = PamConv {
            conv: Some(pam_conversation),
            appdata_ptr: &data as *const PamConvData as *mut _,
        };

        let start = pam_start(service_c.as_ptr(), user_c.as_ptr(), &conv, &mut handle);
        if start != PAM_SUCCESS {
            return false;
        }

        let auth_status = pam_authenticate_raw(handle, 0);
        let _ = pam_setcred(handle, PAM_REFRESH_CRED);
        let _ = pam_end(handle, auth_status);

        auth_status == PAM_SUCCESS
    }
}

#[repr(C)]
struct PamHandle;

#[repr(C)]
struct PamMessage {
    msg_style: i32,
    msg: *const i8,
}

#[repr(C)]
struct PamResponse {
    resp: *mut i8,
    resp_retcode: i32,
}

#[repr(C)]
struct PamConv {
    conv: Option<
        extern "C" fn(
            i32,
            *mut *const PamMessage,
            *mut *mut PamResponse,
            *mut std::ffi::c_void,
        ) -> i32,
    >,
    appdata_ptr: *mut std::ffi::c_void,
}

#[repr(C)]
struct PamConvData {
    password: *const i8,
}

const PAM_SUCCESS: i32 = 0;
const PAM_PROMPT_ECHO_OFF: i32 = 1;
const PAM_PROMPT_ECHO_ON: i32 = 2;
const PAM_ERROR_MSG: i32 = 3;
const PAM_TEXT_INFO: i32 = 4;
const PAM_REFRESH_CRED: i32 = 0x10;

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service_name: *const i8,
        user: *const i8,
        conv: *const PamConv,
        pamh: *mut *mut PamHandle,
    ) -> i32;
    #[link_name = "pam_authenticate"]
    fn pam_authenticate_raw(pamh: *mut PamHandle, flags: i32) -> i32;
    fn pam_setcred(pamh: *mut PamHandle, flags: i32) -> i32;
    fn pam_end(pamh: *mut PamHandle, pam_status: i32) -> i32;
}

extern "C" fn pam_conversation(
    num_msg: i32,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut std::ffi::c_void,
) -> i32 {
    unsafe {
        if num_msg <= 0 {
            return PAM_SUCCESS;
        }
        let responses =
            libc::calloc(num_msg as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if responses.is_null() {
            return PAM_SUCCESS;
        }
        *resp = responses;

        let data = &*(appdata_ptr as *const PamConvData);
        for i in 0..num_msg {
            let msg_ptr = *msg.add(i as usize);
            if msg_ptr.is_null() {
                continue;
            }
            let msg = &*msg_ptr;
            if msg.msg_style == PAM_PROMPT_ECHO_OFF || msg.msg_style == PAM_PROMPT_ECHO_ON {
                let resp_ptr = libc::strdup(data.password);
                (*responses.add(i as usize)).resp = resp_ptr;
            } else if msg.msg_style == PAM_ERROR_MSG || msg.msg_style == PAM_TEXT_INFO {
                // ignore
            }
        }
        PAM_SUCCESS
    }
}
