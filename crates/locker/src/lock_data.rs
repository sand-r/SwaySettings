use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Shared state for the lock screen, equivalent to `LockData` in the Vala implementation.
/// Shared across all lock windows via a thread-local singleton.
pub struct LockData {
    pwd_buffer: gtk4::PasswordEntryBuffer,
    show_password: Cell<bool>,
    auth_in_progress: Cell<bool>,
    messages: RefCell<Vec<String>>,
    errors: RefCell<Vec<String>>,
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<LockData>>> = const { RefCell::new(None) };
}

impl LockData {
    pub fn get() -> Rc<LockData> {
        INSTANCE.with(|cell| {
            cell.borrow_mut()
                .get_or_insert_with(|| {
                    Rc::new(LockData {
                        pwd_buffer: gtk4::PasswordEntryBuffer::new(),
                        show_password: Cell::new(false),
                        auth_in_progress: Cell::new(false),
                        messages: RefCell::new(Vec::new()),
                        errors: RefCell::new(Vec::new()),
                    })
                })
                .clone()
        })
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

    /// Atomically starts a process-wide password check. Every monitor shares
    /// this state, so only one PAM conversation can run at a time.
    pub fn try_begin_auth(&self) -> bool {
        !self.auth_in_progress.replace(true)
    }

    pub fn finish_auth(&self) {
        self.auth_in_progress.set(false);
    }

    pub fn auth_in_progress(&self) -> bool {
        self.auth_in_progress.get()
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
