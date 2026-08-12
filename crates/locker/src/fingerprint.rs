use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;

const MAX_CLAIM_RETRIES: u32 = 10;
const FPRINT_DBUS_TIMEOUT_MS: i32 = 3_000;

type StatusChangedCallback = dyn Fn(&str, bool);

/// Callbacks for fingerprint events.
pub struct FingerprintCallbacks {
    pub on_status_changed: Box<StatusChangedCallback>,
    pub on_auth_success: Box<dyn Fn()>,
    pub on_availability_changed: Box<dyn Fn(bool)>,
}

/// Manages fingerprint authentication via the fprintd D-Bus interface.
///
/// Every D-Bus operation is asynchronous so a slow or wedged sensor can never
/// freeze the password UI. A generation counter makes completions from an old
/// device connection harmless after reinitialization or release.
pub struct FingerprintManager {
    manager_proxy: RefCell<Option<gio::DBusProxy>>,
    device_proxy: RefCell<Option<gio::DBusProxy>>,
    available: Cell<bool>,
    verifying: Cell<bool>,
    claimed: Cell<bool>,
    suspended: Cell<bool>,
    initializing: Cell<bool>,
    claiming: Cell<bool>,
    verify_starting: Cell<bool>,
    generation: Cell<u64>,
    verify_epoch: Cell<u64>,
    claim_retry_count: Cell<u32>,
    verify_signal_handler_id: RefCell<Option<glib::SignalHandlerId>>,
    callbacks: RefCell<Option<FingerprintCallbacks>>,
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<FingerprintManager>>> = const { RefCell::new(None) };
}

impl FingerprintManager {
    pub fn get_instance() -> Rc<FingerprintManager> {
        INSTANCE.with(|cell| {
            cell.borrow_mut()
                .get_or_insert_with(|| {
                    Rc::new(FingerprintManager {
                        manager_proxy: RefCell::new(None),
                        device_proxy: RefCell::new(None),
                        available: Cell::new(false),
                        verifying: Cell::new(false),
                        claimed: Cell::new(false),
                        suspended: Cell::new(false),
                        initializing: Cell::new(false),
                        claiming: Cell::new(false),
                        verify_starting: Cell::new(false),
                        generation: Cell::new(0),
                        verify_epoch: Cell::new(0),
                        claim_retry_count: Cell::new(0),
                        verify_signal_handler_id: RefCell::new(None),
                        callbacks: RefCell::new(None),
                    })
                })
                .clone()
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

    pub fn set_suspended(self: &Rc<Self>, suspended: bool) {
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

    fn set_available(&self, available: bool) {
        if self.available.replace(available) != available {
            self.emit_availability_changed(available);
        }
    }

    fn emit_availability_changed(&self, available: bool) {
        if let Some(ref cbs) = *self.callbacks.borrow() {
            (cbs.on_availability_changed)(available);
        }
    }

    fn next_generation(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    fn is_current_generation(&self, generation: u64) -> bool {
        self.generation.get() == generation
    }

    fn next_verify_epoch(&self) -> u64 {
        let epoch = self.verify_epoch.get().wrapping_add(1);
        self.verify_epoch.set(epoch);
        epoch
    }

    pub fn init(self: &Rc<Self>) {
        if self.initializing.replace(true) {
            return;
        }

        // Starting a new generation invalidates any operation guards inherited
        // from an earlier attempt, even if a future caller reaches init without
        // first going through release_device().
        self.claiming.set(false);
        self.verify_starting.set(false);
        log::debug!("Fingerprint: starting asynchronous init");
        self.claim_retry_count.set(0);
        let generation = self.next_generation();
        let manager = self.clone();
        glib::MainContext::default().spawn_local(async move {
            manager.initialize(generation).await;
        });
    }

    async fn initialize(self: &Rc<Self>, generation: u64) {
        let manager_proxy = match gio::DBusProxy::for_bus_future(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "net.reactivated.Fprint",
            "/net/reactivated/Fprint/Manager",
            "net.reactivated.Fprint.Manager",
        )
        .await
        {
            Ok(proxy) => proxy,
            Err(error) => {
                self.finish_initialization_error(
                    generation,
                    &format!("could not connect to fprintd manager: {error}"),
                );
                return;
            }
        };

        if !self.is_current_generation(generation) {
            return;
        }
        self.manager_proxy.replace(Some(manager_proxy.clone()));

        let device_path = match Self::get_default_device(&manager_proxy).await {
            Ok(path) if !path.is_empty() => path,
            Ok(_) => {
                self.finish_initialization_error(generation, "no fingerprint device found");
                return;
            }
            Err(error) => {
                self.finish_initialization_error(
                    generation,
                    &format!("could not get fingerprint device: {error}"),
                );
                return;
            }
        };

        let device = match Self::device_proxy(&device_path).await {
            Ok(device) => device,
            Err(error) => {
                self.finish_initialization_error(
                    generation,
                    &format!("could not connect to fingerprint device: {error}"),
                );
                return;
            }
        };

        if !self.is_current_generation(generation) {
            return;
        }

        log::debug!("Fingerprint: connected to device at {device_path}");
        self.device_proxy.replace(Some(device));
        self.initializing.set(false);
        self.claim_with_retry(generation);
    }

    fn finish_initialization_error(&self, generation: u64, message: &str) {
        if !self.is_current_generation(generation) {
            return;
        }
        log::debug!("Fingerprint: {message}");
        self.initializing.set(false);
        self.set_available(false);
    }

    async fn get_default_device(manager: &gio::DBusProxy) -> Result<String, glib::Error> {
        let result = manager
            .call_future(
                "GetDefaultDevice",
                None,
                gio::DBusCallFlags::NONE,
                FPRINT_DBUS_TIMEOUT_MS,
            )
            .await?;
        Ok(result
            .get::<(String,)>()
            .map(|path| path.0)
            .unwrap_or_default())
    }

    async fn device_proxy(device_path: &str) -> Result<gio::DBusProxy, glib::Error> {
        gio::DBusProxy::for_bus_future(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "net.reactivated.Fprint",
            device_path,
            "net.reactivated.Fprint.Device",
        )
        .await
    }

    fn reinitialize_device(self: &Rc<Self>) {
        log::debug!("Fingerprint: asynchronously reinitializing device after suspend");

        let generation = self.next_generation();
        self.next_verify_epoch();
        self.initializing.set(true);
        self.claiming.set(false);
        self.claim_retry_count.set(0);
        self.verifying.set(false);
        self.verify_starting.set(false);
        self.claimed.set(false);
        self.set_available(false);

        let old_device = self.take_device_proxy();
        if let Some(device) = old_device {
            Self::release_proxy_best_effort(device);
        }

        let manager_proxy = self.manager_proxy.borrow().clone();
        let manager = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let Some(manager_proxy) = manager_proxy else {
                manager.initializing.set(false);
                manager.init();
                return;
            };

            let device_path = match Self::get_default_device(&manager_proxy).await {
                Ok(path) if !path.is_empty() => path,
                Ok(_) => {
                    manager.finish_initialization_error(generation, "no fingerprint device found");
                    return;
                }
                Err(error) => {
                    manager.finish_initialization_error(
                        generation,
                        &format!("device reinitialization failed: {error}"),
                    );
                    return;
                }
            };

            let device = match Self::device_proxy(&device_path).await {
                Ok(device) => device,
                Err(error) => {
                    manager.finish_initialization_error(
                        generation,
                        &format!("device reinitialization failed: {error}"),
                    );
                    return;
                }
            };

            if !manager.is_current_generation(generation) {
                return;
            }
            manager.device_proxy.replace(Some(device));
            manager.initializing.set(false);
            manager.claim_with_retry(generation);
        });
    }

    fn claim_with_retry(self: &Rc<Self>, generation: u64) {
        if !self.is_current_generation(generation) || self.claiming.replace(true) {
            return;
        }

        let Some(device) = self.device_proxy.borrow().clone() else {
            self.claiming.set(false);
            return;
        };

        let manager = self.clone();
        glib::MainContext::default().spawn_local(async move {
            // Clear a stale claim left by an interrupted verification attempt.
            let _ = device
                .call_future(
                    "Release",
                    None,
                    gio::DBusCallFlags::NONE,
                    FPRINT_DBUS_TIMEOUT_MS,
                )
                .await;

            if !manager.is_current_generation(generation) {
                return;
            }

            let claim_result = device
                .call_future(
                    "Claim",
                    Some(&("",).to_variant()),
                    gio::DBusCallFlags::NONE,
                    FPRINT_DBUS_TIMEOUT_MS,
                )
                .await;

            if !manager.is_current_generation(generation) {
                return;
            }
            manager.claiming.set(false);

            match claim_result {
                Ok(_) => {
                    manager.claimed.set(true);
                    manager.claim_retry_count.set(0);
                    manager.connect_verify_signal();
                    manager.set_available(true);
                    log::debug!("Fingerprint: device claimed successfully");
                    manager.start_verify();
                }
                Err(error) => manager.schedule_claim_retry(generation, &error),
            }
        });
    }

    fn schedule_claim_retry(self: &Rc<Self>, generation: u64, error: &glib::Error) {
        log::debug!("Fingerprint: claim error: {error}");
        if error.to_string().contains("NoEnrolledPrints") {
            self.emit_status_changed("No fingerprints", false);
        }

        let retry = self.claim_retry_count.get() + 1;
        if retry > MAX_CLAIM_RETRIES {
            log::debug!("Fingerprint: claim failed after {MAX_CLAIM_RETRIES} retries");
            self.set_available(false);
            return;
        }

        self.claim_retry_count.set(retry);
        let delay_ms = 500 * retry;
        log::debug!("Fingerprint: claim retry {retry} in {delay_ms}ms");
        let manager = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(delay_ms.into()), move || {
            manager.claim_with_retry(generation);
        });
    }

    fn connect_verify_signal(self: &Rc<Self>) {
        self.disconnect_verify_signal();
        let Some(device) = self.device_proxy.borrow().clone() else {
            return;
        };

        let manager = self.clone();
        let handler_id = device.connect_local("g-signal", false, move |values| {
            // values: [proxy, sender_name, signal_name, parameters]
            let signal_name = values.get(2).and_then(|value| value.get::<&str>().ok())?;
            if signal_name != "VerifyStatus" {
                return None;
            }
            let Some(parameters) = values
                .get(3)
                .and_then(|value| value.get::<glib::Variant>().ok())
            else {
                log::warn!("Fingerprint: malformed VerifyStatus signal");
                return None;
            };
            manager.on_verify_status(&parameters);
            None
        });
        self.verify_signal_handler_id.replace(Some(handler_id));
    }

    fn disconnect_verify_signal(&self) {
        let handler_id = self.verify_signal_handler_id.borrow_mut().take();
        let device = self.device_proxy.borrow().clone();
        if let (Some(handler_id), Some(device)) = (handler_id, device) {
            device.disconnect(handler_id);
        }
    }

    fn take_device_proxy(&self) -> Option<gio::DBusProxy> {
        self.disconnect_verify_signal();
        self.device_proxy.borrow_mut().take()
    }

    pub fn release_device(self: &Rc<Self>) {
        log::debug!("Fingerprint: scheduling best-effort device release");
        self.next_generation();
        self.next_verify_epoch();
        self.initializing.set(false);
        self.claiming.set(false);
        self.verify_starting.set(false);
        self.verifying.set(false);
        self.claimed.set(false);
        self.set_available(false);

        if let Some(device) = self.take_device_proxy() {
            Self::release_proxy_best_effort(device);
        }
        let _ = self.manager_proxy.borrow_mut().take();
    }

    fn release_proxy_best_effort(device: gio::DBusProxy) {
        glib::MainContext::default().spawn_local(async move {
            let _ = device
                .call_future(
                    "VerifyStop",
                    None,
                    gio::DBusCallFlags::NONE,
                    FPRINT_DBUS_TIMEOUT_MS,
                )
                .await;
            let _ = device
                .call_future(
                    "Release",
                    None,
                    gio::DBusCallFlags::NONE,
                    FPRINT_DBUS_TIMEOUT_MS,
                )
                .await;
        });
    }

    pub fn start_verify(self: &Rc<Self>) {
        if self.verifying.get()
            || self.suspended.get()
            || !self.claimed.get()
            || self.verify_starting.replace(true)
        {
            return;
        }

        let Some(device) = self.device_proxy.borrow().clone() else {
            self.verify_starting.set(false);
            return;
        };
        let generation = self.generation.get();
        let epoch = self.next_verify_epoch();
        let manager = self.clone();

        glib::MainContext::default().spawn_local(async move {
            let result = device
                .call_future(
                    "VerifyStart",
                    Some(&("any",).to_variant()),
                    gio::DBusCallFlags::NONE,
                    FPRINT_DBUS_TIMEOUT_MS,
                )
                .await;

            let current =
                manager.is_current_generation(generation) && manager.verify_epoch.get() == epoch;
            if !current {
                if result.is_ok() {
                    let _ = device
                        .call_future(
                            "VerifyStop",
                            None,
                            gio::DBusCallFlags::NONE,
                            FPRINT_DBUS_TIMEOUT_MS,
                        )
                        .await;
                }
                return;
            }

            manager.verify_starting.set(false);
            match result {
                Ok(_) if !manager.suspended.get() && manager.claimed.get() => {
                    manager.verifying.set(true);
                    manager.emit_status_changed("Touch sensor", false);
                }
                Ok(_) => {
                    let _ = device
                        .call_future(
                            "VerifyStop",
                            None,
                            gio::DBusCallFlags::NONE,
                            FPRINT_DBUS_TIMEOUT_MS,
                        )
                        .await;
                }
                Err(error) => {
                    log::debug!("Could not start fingerprint verification: {error}");
                }
            }
        });
    }

    pub fn stop_verify(self: &Rc<Self>) {
        let had_operation = self.verifying.get() || self.verify_starting.get();
        self.next_verify_epoch();
        self.verify_starting.set(false);
        self.verifying.set(false);
        if !had_operation {
            return;
        }

        if let Some(device) = self.device_proxy.borrow().clone() {
            glib::MainContext::default().spawn_local(async move {
                let _ = device
                    .call_future(
                        "VerifyStop",
                        None,
                        gio::DBusCallFlags::NONE,
                        FPRINT_DBUS_TIMEOUT_MS,
                    )
                    .await;
            });
        }
    }

    fn on_verify_status(self: &Rc<Self>, parameters: &glib::Variant) {
        if self.suspended.get() {
            return;
        }

        let (status, done): (String, bool) = match parameters.get() {
            Some(value) => value,
            None => {
                log::warn!("Fingerprint: invalid VerifyStatus parameters");
                return;
            }
        };

        log::debug!("Fingerprint status: {status}, done: {done}");
        match status.as_str() {
            "verify-match" => {
                self.emit_status_changed("Fingerprint matched", false);
                self.emit_auth_success();
                return;
            }
            "verify-no-match" => self.emit_status_changed("Not recognized", true),
            "verify-retry-scan" => self.emit_status_changed("Try again", false),
            "verify-swipe-too-short" => self.emit_status_changed("Swipe too short", false),
            "verify-finger-not-centered" => self.emit_status_changed("Center your finger", false),
            "verify-remove-and-retry" => self.emit_status_changed("Retry", false),
            "verify-disconnected" => {
                self.verify_starting.set(false);
                self.verifying.set(false);
                self.claimed.set(false);
                self.set_available(false);
                self.emit_status_changed("Disconnected", true);
                return;
            }
            "verify-unknown-error" => {
                self.emit_status_changed("Reconnecting...", false);
                self.verifying.set(false);
                let manager = self.clone();
                glib::timeout_add_local_once(Duration::from_millis(1_000), move || {
                    if !manager.suspended.get() {
                        manager.reinitialize_device();
                    }
                });
                return;
            }
            _ => self.emit_status_changed(&status, false),
        }

        if done {
            self.stop_verify();
            let manager = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(1_500), move || {
                if !manager.suspended.get() {
                    manager.start_verify();
                }
            });
        }
    }
}
