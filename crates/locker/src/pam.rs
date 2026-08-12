use std::ffi::{c_char, c_void, CString};
use std::ptr;

use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PamStatus {
    Error,
    AuthFailed,
    AuthSuccess,
}

/// Authenticate the current user's password via PAM on a background thread.
///
/// The password is owned by a `Zeroizing` allocation before it crosses the
/// thread boundary and is cleared when authentication finishes. PAM itself
/// owns the response copies made by the conversation callback.
pub fn check_password_async<F: FnOnce(PamStatus) + 'static>(
    password: Zeroizing<String>,
    callback: F,
) {
    let (tx, rx) = std::sync::mpsc::channel::<PamStatus>();

    std::thread::spawn(move || {
        let status = pam_authenticate_credentials(
            "swaysettings-locker",
            &whoami::username(),
            password.as_str(),
        );
        let _ = tx.send(status);
    });

    let callback = std::cell::Cell::new(Some(callback));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(status) => {
                if let Some(callback) = callback.take() {
                    callback(status);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if let Some(callback) = callback.take() {
                    callback(PamStatus::Error);
                }
                glib::ControlFlow::Break
            }
        }
    });
}

fn pam_authenticate_credentials(service: &str, username: &str, password: &str) -> PamStatus {
    unsafe {
        let service_c = match CString::new(service) {
            Ok(value) => value,
            Err(_) => return PamStatus::Error,
        };
        let user_c = match CString::new(username) {
            Ok(value) => value,
            Err(_) => return PamStatus::Error,
        };
        if password.as_bytes().contains(&0) {
            return PamStatus::AuthFailed;
        }

        // CString does not promise to clear its allocation on drop, so keep the
        // NUL-terminated password in an explicitly zeroizing byte buffer.
        let mut password_c = Zeroizing::new(Vec::with_capacity(password.len() + 1));
        password_c.extend_from_slice(password.as_bytes());
        password_c.push(0);

        let data = PamConvData {
            password: password_c.as_ptr().cast(),
            username: user_c.as_ptr(),
        };
        let conv = PamConv {
            conv: Some(pam_conversation),
            appdata_ptr: (&data as *const PamConvData).cast_mut().cast(),
        };
        let mut handle: *mut PamHandle = ptr::null_mut();

        let start = pam_start(service_c.as_ptr(), user_c.as_ptr(), &conv, &mut handle);
        if start != PAM_SUCCESS || handle.is_null() {
            return PamStatus::Error;
        }

        let auth_status = pam_authenticate_raw(handle, 0);
        if auth_status == PAM_SUCCESS {
            let _ = pam_setcred(handle, PAM_REFRESH_CRED);
        }
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
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: i32,
}

#[repr(C)]
struct PamConv {
    conv: Option<
        extern "C" fn(i32, *const *const PamMessage, *mut *mut PamResponse, *mut c_void) -> i32,
    >,
    appdata_ptr: *mut c_void,
}

#[repr(C)]
struct PamConvData {
    password: *const c_char,
    username: *const c_char,
}

const PAM_SUCCESS: i32 = 0;
const PAM_PROMPT_ECHO_OFF: i32 = 1;
const PAM_PROMPT_ECHO_ON: i32 = 2;
const PAM_ERROR_MSG: i32 = 3;
const PAM_TEXT_INFO: i32 = 4;
const PAM_CONV_ERR: i32 = 19;
const PAM_REFRESH_CRED: i32 = 0x10;

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service_name: *const c_char,
        user: *const c_char,
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
    msg: *const *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> i32 {
    unsafe {
        if resp.is_null() {
            return PAM_CONV_ERR;
        }
        // PAM requires the response pointer to remain NULL on every error.
        *resp = ptr::null_mut();
        if num_msg <= 0 || msg.is_null() || appdata_ptr.is_null() {
            return PAM_CONV_ERR;
        }

        let count = num_msg as usize;
        let responses =
            libc::calloc(count, std::mem::size_of::<PamResponse>()).cast::<PamResponse>();
        if responses.is_null() {
            return PAM_CONV_ERR;
        }

        let data = &*appdata_ptr.cast::<PamConvData>();
        for index in 0..count {
            let message = *msg.add(index);
            if message.is_null() {
                free_responses(responses, count);
                return PAM_CONV_ERR;
            }

            let answer = match (*message).msg_style {
                PAM_PROMPT_ECHO_OFF => data.password,
                // Echo-on prompts conventionally ask for the login name. Never
                // hand a password to a PAM module that may log this response.
                PAM_PROMPT_ECHO_ON => data.username,
                PAM_ERROR_MSG | PAM_TEXT_INFO => continue,
                _ => {
                    free_responses(responses, count);
                    return PAM_CONV_ERR;
                }
            };

            if answer.is_null() {
                free_responses(responses, count);
                return PAM_CONV_ERR;
            }
            let answer_copy = libc::strdup(answer);
            if answer_copy.is_null() {
                free_responses(responses, count);
                return PAM_CONV_ERR;
            }
            (*responses.add(index)).resp = answer_copy;
        }

        *resp = responses;
        PAM_SUCCESS
    }
}

unsafe fn free_responses(responses: *mut PamResponse, count: usize) {
    for index in 0..count {
        let answer = unsafe { (*responses.add(index)).resp };
        if !answer.is_null() {
            let length = unsafe { libc::strlen(answer) };
            unsafe {
                ptr::write_bytes(answer.cast::<u8>(), 0, length);
                libc::free(answer.cast());
            }
        }
    }
    unsafe { libc::free(responses.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn conversation_rejects_invalid_inputs_without_a_response() {
        let mut response = ptr::dangling_mut::<PamResponse>();
        assert_eq!(
            pam_conversation(0, ptr::null_mut(), &mut response, ptr::null_mut(),),
            PAM_CONV_ERR
        );
        assert!(response.is_null());
    }

    #[test]
    fn conversation_distinguishes_secret_and_username_prompts() {
        let password = CString::new("secret").expect("test password");
        let username = CString::new("sandor").expect("test username");
        let data = PamConvData {
            password: password.as_ptr(),
            username: username.as_ptr(),
        };
        let password_message = PamMessage {
            msg_style: PAM_PROMPT_ECHO_OFF,
            msg: ptr::null(),
        };
        let username_message = PamMessage {
            msg_style: PAM_PROMPT_ECHO_ON,
            msg: ptr::null(),
        };
        let messages = [
            &password_message as *const PamMessage,
            &username_message as *const PamMessage,
        ];
        let mut responses = ptr::null_mut();

        assert_eq!(
            pam_conversation(
                messages.len() as i32,
                messages.as_ptr(),
                &mut responses,
                (&data as *const PamConvData).cast_mut().cast(),
            ),
            PAM_SUCCESS
        );
        assert!(!responses.is_null());
        unsafe {
            assert_eq!(CStr::from_ptr((*responses).resp).to_bytes(), b"secret");
            assert_eq!(
                CStr::from_ptr((*responses.add(1)).resp).to_bytes(),
                b"sandor"
            );
            free_responses(responses, messages.len());
        }
    }
}
