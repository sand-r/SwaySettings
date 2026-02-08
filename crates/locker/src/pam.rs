use std::ffi::CString;
use std::ptr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PamStatus {
    Error,
    AuthFailed,
    AuthSuccess,
}

/// Authenticate the current user's password via PAM, running in a background
/// thread so the GTK main loop stays responsive.
///
/// Calls `callback` on the main thread with the result.
pub fn check_password_async<F: FnOnce(PamStatus) + 'static>(password: &str, callback: F) {
    let password = password.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<PamStatus>();

    std::thread::spawn(move || {
        let status = pam_authenticate_credentials(
            "swaysettings-locker",
            &whoami::username(),
            &password,
        );
        let _ = tx.send(status);
    });

    let callback = std::cell::Cell::new(Some(callback));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(status) => {
                if let Some(cb) = callback.take() {
                    cb(status);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if let Some(cb) = callback.take() {
                    cb(PamStatus::Error);
                }
                glib::ControlFlow::Break
            }
        }
    });
}

fn pam_authenticate_credentials(service: &str, username: &str, password: &str) -> PamStatus {
    unsafe {
        let service_c = match CString::new(service) {
            Ok(val) => val,
            Err(_) => return PamStatus::Error,
        };
        let user_c = match CString::new(username) {
            Ok(val) => val,
            Err(_) => return PamStatus::Error,
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
            return PamStatus::Error;
        }

        let auth_status = pam_authenticate_raw(handle, 0);
        let _ = pam_setcred(handle, PAM_REFRESH_CRED);
        let _ = pam_end(handle, auth_status);

        if auth_status == PAM_SUCCESS {
            PamStatus::AuthSuccess
        } else {
            PamStatus::AuthFailed
        }
    }
}

// --- PAM FFI types and functions ---

#[repr(C)]
struct PamHandle {
    _opaque: [u8; 0],
}

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
            let m = &*msg_ptr;
            if m.msg_style == PAM_PROMPT_ECHO_OFF || m.msg_style == PAM_PROMPT_ECHO_ON {
                let resp_ptr = libc::strdup(data.password);
                (*responses.add(i as usize)).resp = resp_ptr;
            } else if m.msg_style == PAM_ERROR_MSG || m.msg_style == PAM_TEXT_INFO {
                // ignored
            }
        }
        PAM_SUCCESS
    }
}
