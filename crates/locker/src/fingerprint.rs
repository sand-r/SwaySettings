use std::cell::{Cell, RefCell};

use gio::prelude::*;

const MAX_CLAIM_RETRIES: u32 = 10;

/// Callbacks for fingerprint events.
pub struct FingerprintCallbacks {
    pub on_status_changed: Box<dyn Fn(&str, bool)>,
    pub on_auth_success: Box<dyn Fn()>,
    pub on_availability_changed: Box<dyn Fn(bool)>,
}

/// Manages fingerprint authentication via fprintd D-Bus interface.
pub struct FingerprintManager {
    manager_proxy: RefCell<Option<gio::DBusProxy>>,
    device_proxy: RefCell<Option<gio::DBusProxy>>,
    available: Cell<bool>,
    verifying: Cell<bool>,
    claimed: Cell<bool>,
    suspended: Cell<bool>,
    claim_retry_count: Cell<u32>,
    verify_signal_handler_id: RefCell<Option<glib::SignalHandlerId>>,
    callbacks: RefCell<Option<FingerprintCallbacks>>,
}

thread_local! {
    static INSTANCE: RefCell<Option<std::rc::Rc<FingerprintManager>>> = RefCell::new(None);
}

impl FingerprintManager {
    pub fn get_instance() -> std::rc::Rc<FingerprintManager> {
        INSTANCE.with(|cell| {
            let mut opt = cell.borrow_mut();
            if opt.is_none() {
                *opt = Some(std::rc::Rc::new(FingerprintManager {
                    manager_proxy: RefCell::new(None),
                    device_proxy: RefCell::new(None),
                    available: Cell::new(false),
                    verifying: Cell::new(false),
                    claimed: Cell::new(false),
                    suspended: Cell::new(false),
                    claim_retry_count: Cell::new(0),
                    verify_signal_handler_id: RefCell::new(None),
                    callbacks: RefCell::new(None),
                }));
            }
            opt.as_ref().unwrap().clone()
        })
    }

    pub fn set_callbacks(&self, callbacks: FingerprintCallbacks) {
        *self.callbacks.borrow_mut() = Some(callbacks);
    }

    pub fn available(&self) -> bool {
        self.available.get()
    }

    pub fn verifying(&self) -> bool {
        self.verifying.get()
    }

    pub fn suspended(&self) -> bool {
        self.suspended.get()
    }

    pub fn set_suspended(&self, suspended: bool) {
        self.suspended.set(suspended);
        if suspended {
            self.stop_verify();
        } else if self.available.get() && self.claimed.get() {
            self.start_verify();
        }
    }

    fn emit_status_changed(&self, status: &str, is_error: bool) {
        if let Some(ref cbs) = *self.callbacks.borrow() {
            (cbs.on_status_changed)(status, is_error);
        }
    }

    fn emit_auth_success(&self) {
        if let Some(ref cbs) = *self.callbacks.borrow() {
            (cbs.on_auth_success)();
        }
    }

    fn emit_availability_changed(&self, available: bool) {
        if let Some(ref cbs) = *self.callbacks.borrow() {
            (cbs.on_availability_changed)(available);
        }
    }

    pub fn init(&self) {
        log::debug!("Fingerprint: starting init");
        self.claim_retry_count.set(0);

        let proxy = match gio::DBusProxy::for_bus_sync(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "net.reactivated.Fprint",
            "/net/reactivated/Fprint/Manager",
            "net.reactivated.Fprint.Manager",
            None::<&gio::Cancellable>,
        ) {
            Ok(proxy) => proxy,
            Err(e) => {
                log::debug!("Fingerprint: could not connect to fprintd manager: {}", e);
                self.available.set(false);
                return;
            }
        };
        *self.manager_proxy.borrow_mut() = Some(proxy.clone());
        log::debug!("Fingerprint: connected to fprintd manager");

        // Get default device
        let device_path = match proxy.call_sync(
            "GetDefaultDevice",
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        ) {
            Ok(result) => {
                let path: (String,) = result.get().unwrap_or_default();
                path.0
            }
            Err(e) => {
                log::debug!("Fingerprint: error getting device: {}", e);
                self.available.set(false);
                return;
            }
        };

        log::debug!("Fingerprint: device path = {}", device_path);
        if device_path.is_empty() {
            log::debug!("Fingerprint: no device found");
            self.available.set(false);
            return;
        }

        match gio::DBusProxy::for_bus_sync(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "net.reactivated.Fprint",
            &device_path,
            "net.reactivated.Fprint.Device",
            None::<&gio::Cancellable>,
        ) {
            Ok(device) => {
                log::debug!("Fingerprint: connected to device");
                *self.device_proxy.borrow_mut() = Some(device);
                self.claim_with_retry();
            }
            Err(e) => {
                log::debug!("Fingerprint: error connecting to device: {}", e);
                self.available.set(false);
            }
        }
    }

    fn reinitialize_device(&self) {
        log::debug!("Fingerprint: reinitializing device after suspend");

        // Clean up old state
        self.disconnect_verify_signal();

        if let Some(ref device) = *self.device_proxy.borrow() {
            if self.claimed.get() {
                let _ = device.call_sync(
                    "Release",
                    None,
                    gio::DBusCallFlags::NONE,
                    -1,
                    None::<&gio::Cancellable>,
                );
            }
        }

        self.claimed.set(false);
        self.verifying.set(false);
        *self.device_proxy.borrow_mut() = None;

        // Re-connect to device
        let manager = self.manager_proxy.borrow();
        let manager = match manager.as_ref() {
            Some(m) => m,
            None => {
                self.available.set(false);
                self.emit_availability_changed(false);
                return;
            }
        };

        let device_path = match manager.call_sync(
            "GetDefaultDevice",
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        ) {
            Ok(result) => {
                let path: (String,) = result.get().unwrap_or_default();
                path.0
            }
            Err(e) => {
                log::debug!("Fingerprint: reinit failed: {}", e);
                self.available.set(false);
                self.emit_availability_changed(false);
                return;
            }
        };

        if device_path.is_empty() {
            self.available.set(false);
            self.emit_availability_changed(false);
            return;
        }

        match gio::DBusProxy::for_bus_sync(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "net.reactivated.Fprint",
            &device_path,
            "net.reactivated.Fprint.Device",
            None::<&gio::Cancellable>,
        ) {
            Ok(device) => {
                *self.device_proxy.borrow_mut() = Some(device);
                self.claim_retry_count.set(0);
                self.claim_with_retry();
            }
            Err(e) => {
                log::debug!("Fingerprint: reinit failed: {}", e);
                self.available.set(false);
                self.emit_availability_changed(false);
            }
        }
    }

    fn claim_with_retry(&self) {
        if self.claim_device() {
            self.available.set(true);
            log::debug!("Fingerprint: device claimed, available = true");
            self.emit_availability_changed(true);
            self.start_verify();
        } else if self.claim_retry_count.get() < MAX_CLAIM_RETRIES {
            let count = self.claim_retry_count.get() + 1;
            self.claim_retry_count.set(count);
            let delay = 500 * count;
            log::debug!(
                "Fingerprint: claim failed, retry {} in {}ms",
                count,
                delay
            );
            let mgr = Self::get_instance();
            glib::timeout_add_local_once(std::time::Duration::from_millis(delay as u64), move || {
                mgr.claim_with_retry();
            });
        } else {
            log::debug!(
                "Fingerprint: claim failed after {} retries",
                self.claim_retry_count.get()
            );
            self.available.set(false);
            self.emit_availability_changed(false);
        }
    }

    fn claim_device(&self) -> bool {
        let device = self.device_proxy.borrow();
        let device = match device.as_ref() {
            Some(d) => d,
            None => {
                log::debug!("Fingerprint: claim_device - proxy is null");
                return false;
            }
        };

        // Try to release stale claim first
        let _ = device.call_sync(
            "Release",
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        );

        // Claim for current user (empty string = current user)
        match device.call_sync(
            "Claim",
            Some(&("",).to_variant()),
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        ) {
            Ok(_) => {
                self.claimed.set(true);
                log::debug!("Fingerprint: device claimed successfully");
                self.connect_verify_signal();
                true
            }
            Err(e) => {
                log::debug!("Fingerprint: claim error: {}", e);
                if e.to_string().contains("NoEnrolledPrints") {
                    self.emit_status_changed("No fingerprints", false);
                }
                false
            }
        }
    }

    fn connect_verify_signal(&self) {
        let device = self.device_proxy.borrow();
        let device = match device.as_ref() {
            Some(d) => d,
            None => return,
        };

        let mgr = Self::get_instance();
        let handler_id = device.connect_local("g-signal", false, move |values| {
            // values: [proxy, sender_name, signal_name, parameters]
            let signal_name = values[2].get::<&str>().unwrap_or("");
            if signal_name != "VerifyStatus" {
                return None;
            }
            let params = values[3].get::<glib::Variant>().unwrap();
            mgr.on_verify_status(&params);
            None
        });
        *self.verify_signal_handler_id.borrow_mut() = Some(handler_id);
        log::debug!("Fingerprint: signal handler connected");
    }

    fn disconnect_verify_signal(&self) {
        if let Some(handler_id) = self.verify_signal_handler_id.borrow_mut().take() {
            if let Some(ref device) = *self.device_proxy.borrow() {
                device.disconnect(handler_id);
            }
        }
    }

    pub fn release_device(&self) {
        log::debug!("Fingerprint: release_device called");
        self.stop_verify();

        if self.claimed.get() {
            self.disconnect_verify_signal();
            if let Some(ref device) = *self.device_proxy.borrow() {
                let _ = device.call_sync(
                    "Release",
                    None,
                    gio::DBusCallFlags::NONE,
                    -1,
                    None::<&gio::Cancellable>,
                );
            }
            self.claimed.set(false);
        }
    }

    pub fn start_verify(&self) {
        if self.verifying.get() || self.suspended.get() || !self.claimed.get() {
            return;
        }

        let device = self.device_proxy.borrow();
        let device = match device.as_ref() {
            Some(d) => d,
            None => return,
        };

        match device.call_sync(
            "VerifyStart",
            Some(&("any",).to_variant()),
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        ) {
            Ok(_) => {
                self.verifying.set(true);
                self.emit_status_changed("Touch sensor", false);
            }
            Err(e) => {
                log::debug!("Could not start fingerprint verification: {}", e);
            }
        }
    }

    pub fn stop_verify(&self) {
        if !self.verifying.get() {
            return;
        }

        let device = self.device_proxy.borrow();
        if let Some(ref device) = *device {
            let _ = device.call_sync(
                "VerifyStop",
                None,
                gio::DBusCallFlags::NONE,
                -1,
                None::<&gio::Cancellable>,
            );
        }
        self.verifying.set(false);
    }

    fn on_verify_status(&self, parameters: &glib::Variant) {
        if self.suspended.get() {
            return;
        }

        let (status, done): (String, bool) = match parameters.get() {
            Some(v) => v,
            None => return,
        };

        log::debug!("Fingerprint status: {}, done: {}", status, done);

        match status.as_str() {
            "verify-match" => {
                self.emit_status_changed("Fingerprint matched", false);
                self.emit_auth_success();
                return;
            }
            "verify-no-match" => {
                self.emit_status_changed("Not recognized", true);
            }
            "verify-retry-scan" => {
                self.emit_status_changed("Try again", false);
            }
            "verify-swipe-too-short" => {
                self.emit_status_changed("Swipe too short", false);
            }
            "verify-finger-not-centered" => {
                self.emit_status_changed("Center your finger", false);
            }
            "verify-remove-and-retry" => {
                self.emit_status_changed("Retry", false);
            }
            "verify-disconnected" => {
                self.emit_status_changed("Disconnected", true);
                self.available.set(false);
                self.emit_availability_changed(false);
                return;
            }
            "verify-unknown-error" => {
                self.emit_status_changed("Reconnecting...", false);
                self.verifying.set(false);
                let mgr = Self::get_instance();
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(1000),
                    move || {
                        if !mgr.suspended.get() {
                            mgr.reinitialize_device();
                        }
                    },
                );
                return;
            }
            _ => {
                self.emit_status_changed(&status, false);
            }
        }

        if done {
            self.stop_verify();
            let delay = if status == "verify-unknown-error" {
                2000
            } else {
                1500
            };
            let mgr = Self::get_instance();
            glib::timeout_add_local_once(
                std::time::Duration::from_millis(delay),
                move || {
                    if !mgr.suspended.get() && mgr.available.get() && mgr.claimed.get() {
                        mgr.start_verify();
                    }
                },
            );
        }
    }
}
