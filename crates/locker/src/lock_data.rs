use std::cell::{Cell, RefCell};

/// Shared state for the lock screen, equivalent to `LockData` in the Vala implementation.
/// Shared across all lock windows via a thread-local singleton.
pub struct LockData {
    pwd_buffer: gtk4::PasswordEntryBuffer,
    show_password: Cell<bool>,
    messages: RefCell<Vec<String>>,
    errors: RefCell<Vec<String>>,
}

thread_local! {
    static INSTANCE: LockData = LockData {
        pwd_buffer: gtk4::PasswordEntryBuffer::new(),
        show_password: Cell::new(false),
        messages: RefCell::new(Vec::new()),
        errors: RefCell::new(Vec::new()),
    };
}

impl LockData {
    pub fn get() -> &'static LockData {
        // SAFETY: thread_local! ensures single-threaded access, and the reference
        // is valid for the lifetime of the thread (which is the main thread).
        INSTANCE.with(|data| unsafe { &*(data as *const LockData) })
    }

    pub fn pwd_buffer(&self) -> &gtk4::PasswordEntryBuffer {
        &self.pwd_buffer
    }

    pub fn show_password(&self) -> bool {
        self.show_password.get()
    }

    pub fn toggle_show_password(&self) {
        self.show_password.set(!self.show_password.get());
    }

    pub fn messages(&self) -> Vec<String> {
        self.messages.borrow().clone()
    }

    pub fn errors(&self) -> Vec<String> {
        self.errors.borrow().clone()
    }

    pub fn add_message(&self, msg: &str) {
        self.messages.borrow_mut().push(msg.to_string());
    }

    #[allow(dead_code)]
    pub fn add_error(&self, err: &str) {
        self.errors.borrow_mut().push(err.to_string());
    }

    pub fn clear_messages(&self) {
        self.messages.borrow_mut().clear();
        self.errors.borrow_mut().clear();
    }
}
