use std::cell::{Cell, RefCell};

use gio::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;

use crate::fingerprint::{FingerprintCallbacks, FingerprintManager};
use crate::lock_data::LockData;
use crate::pam::{self, PamStatus};

const PASSWORD_SHOW_ICON_NAME: &str = "eye-open-negative-filled-symbolic";
const PASSWORD_HIDE_ICON_NAME: &str = "eye-not-looking-symbolic";
const DEFAULT_PLACEHOLDER: &str = "Enter Password";

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/LockerWindow.ui")]
pub struct LockerWindowImpl {
    #[template_child(id = "picture")]
    pub picture: TemplateChild<gtk4::Picture>,
    #[template_child(id = "revealer")]
    pub revealer: TemplateChild<gtk4::Revealer>,
    #[template_child(id = "time_label")]
    pub time_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "date_label")]
    pub date_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "avatar")]
    pub avatar: TemplateChild<libadwaita::Avatar>,
    #[template_child(id = "real_name")]
    pub real_name: TemplateChild<gtk4::Label>,
    #[template_child(id = "entry")]
    pub entry: TemplateChild<gtk4::Entry>,
    #[template_child(id = "status_revealer")]
    pub status_revealer: TemplateChild<gtk4::Revealer>,
    #[template_child(id = "status")]
    pub status: TemplateChild<gtk4::Box>,

    pub monitor: RefCell<Option<gdk4::Monitor>>,
    pub loaded_user_data: Cell<bool>,
    pub busy_guard: RefCell<Option<gio::ApplicationBusyGuard>>,
}

#[glib::object_subclass]
impl ObjectSubclass for LockerWindowImpl {
    const NAME: &'static str = "LockerWindow";
    type Type = LockerWindow;
    type ParentType = libadwaita::ApplicationWindow;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for LockerWindowImpl {}
impl WidgetImpl for LockerWindowImpl {}
impl WindowImpl for LockerWindowImpl {}
impl ApplicationWindowImpl for LockerWindowImpl {}
impl AdwApplicationWindowImpl for LockerWindowImpl {}

glib::wrapper! {
    pub struct LockerWindow(ObjectSubclass<LockerWindowImpl>)
        @extends libadwaita::ApplicationWindow, gtk4::ApplicationWindow, gtk4::Window, gtk4::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk4::Accessible, gtk4::Buildable,
                    gtk4::ConstraintTarget, gtk4::Native, gtk4::Root, gtk4::ShortcutManager;
}

impl LockerWindow {
    pub fn new(app: &libadwaita::Application, monitor: &gdk4::Monitor) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .property("css-name", "lockerwindow")
            .build();

        window.imp().monitor.replace(Some(monitor.clone()));
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_focusable(true);
        window.set_default_size(monitor.geometry().width(), monitor.geometry().height());
        window.fullscreen();

        window.setup_focus_handling();
        window.setup_password_entry();
        window.setup_map_handler();

        window
    }

    pub fn monitor(&self) -> Option<gdk4::Monitor> {
        self.imp().monitor.borrow().clone()
    }

    pub fn entry(&self) -> gtk4::Entry {
        self.imp().entry.get()
    }

    pub fn time_label(&self) -> gtk4::Label {
        self.imp().time_label.get()
    }

    pub fn date_label(&self) -> gtk4::Label {
        self.imp().date_label.get()
    }

    fn setup_focus_handling(&self) {
        let should_lock = crate::should_lock();
        self.connect_notify(Some("is-active"), move |win, _| {
            let active = if !should_lock { true } else { win.is_active() };
            win.imp().revealer.set_reveal_child(active);
            let entry = win.imp().entry.get();
            entry.grab_focus_without_selecting();
            entry.set_position(-1);
            if active {
                win.add_css_class("focused");
                win.check_fingerprint_on_focus();
            } else {
                win.remove_css_class("focused");
            }
        });
    }

    fn setup_map_handler(&self) {
        self.connect_map(|win| {
            win.add_css_class("locked");
        });
    }

    fn setup_password_entry(&self) {
        let entry = self.imp().entry.get();
        let lock_data = LockData::get();

        // Bind shared password buffer
        entry.set_buffer(lock_data.pwd_buffer());

        // Set initial password visibility
        self.set_password_visibility();

        // Toggle icon click
        entry.connect_icon_release(clone!(
            #[weak(rename_to = win)]
            self,
            move |_entry, pos| {
                if pos == gtk4::EntryIconPosition::Secondary {
                    LockData::get().toggle_show_password();
                    win.set_password_visibility();
                }
            }
        ));

        // Activate → password check
        entry.connect_activate(clone!(
            #[weak(rename_to = win)]
            self,
            move |_entry| {
                win.password_check();
            }
        ));

        // Suspend fingerprint when typing
        entry.connect_changed(|entry| {
            let lock_data = LockData::get();
            if lock_data.pwd_buffer().length() > 0 {
                let fprint = FingerprintManager::get_instance();
                fprint.set_suspended(true);
                entry.set_icon_from_paintable(
                    gtk4::EntryIconPosition::Primary,
                    None::<&gdk4::Paintable>,
                );
            }
        });
    }

    fn set_password_visibility(&self) {
        let entry = self.imp().entry.get();
        let show = LockData::get().show_password();
        entry.set_visibility(show);
        entry.set_icon_from_icon_name(
            gtk4::EntryIconPosition::Secondary,
            Some(if show {
                PASSWORD_HIDE_ICON_NAME
            } else {
                PASSWORD_SHOW_ICON_NAME
            }),
        );
    }

    pub fn set_date_time(&self, time: &str, date: &str) {
        self.imp().time_label.set_text(time);
        self.imp().date_label.set_text(date);
    }

    pub fn load_content(&self, settings: &gio::Settings) {
        self.load_wallpaper(settings);
        self.load_user_data();
    }

    fn load_wallpaper(&self, settings: &gio::Settings) {
        let path = swaysettings_core::utils::get_wallpaper_gschema(settings);
        let path = match path {
            Some(p) if !p.is_empty() => p,
            _ => {
                log::warn!("No wallpaper path in GSettings");
                return;
            }
        };

        let file = gio::File::for_path(&path);
        if !file.query_exists(None::<&gio::Cancellable>) {
            log::error!("Wallpaper doesn't exist: {}", path);
            return;
        }

        match gdk4::Texture::from_file(&file) {
            Ok(texture) => {
                let monitor = self.imp().monitor.borrow();
                if let Some(ref monitor) = *monitor {
                    let geo = monitor.geometry();
                    if let Some((scaled, _w, _h)) =
                        swaysettings_core::functions::gdk_texture_scale(
                            &texture,
                            texture.width() as u32,
                            texture.height() as u32,
                            geo.width(),
                            geo.height(),
                            gsk4::ScalingFilter::Trilinear,
                        )
                    {
                        self.imp().picture.set_paintable(Some(&scaled));
                    } else {
                        self.imp().picture.set_paintable(Some(&texture));
                    }
                } else {
                    self.imp().picture.set_paintable(Some(&texture));
                }
            }
            Err(e) => {
                log::error!("Getting background error: {}", e);
                self.imp()
                    .picture
                    .set_paintable(None::<&gdk4::Paintable>);
            }
        }

        let scale = swaysettings_core::utils::get_scale_mode_gschema(settings);
        self.imp().picture.set_content_fit(scale.to_content_fit());
    }

    fn load_user_data(&self) {
        let win = self.clone();
        glib::MainContext::default().spawn_local(async move {
            match swaysettings_core::AccountsServiceUser::for_current_user().await {
                Ok(user) => {
                    win.set_user_data(&user);
                    win.imp().loaded_user_data.set(true);

                    let win_weak = win.downgrade();
                    user.connect_changed(move || {
                        if let Some(win) = win_weak.upgrade() {
                            if win.imp().loaded_user_data.get() {
                                let win2 = win.clone();
                                glib::MainContext::default().spawn_local(async move {
                                    if let Ok(u) =
                                        swaysettings_core::AccountsServiceUser::for_current_user()
                                            .await
                                    {
                                        win2.set_user_data(&u);
                                    }
                                });
                            }
                        }
                    });

                    // Keep user alive for the signal handler
                    std::mem::forget(user);
                }
                Err(e) => {
                    log::error!("Failed to get user data: {}", e);
                }
            }
        });
    }

    fn set_user_data(&self, user: &swaysettings_core::AccountsServiceUser) {
        let real_name = user.real_name().unwrap_or_default();

        // Avatar
        self.imp().avatar.set_text(Some(&real_name));
        if let Some(icon_file) = user.icon_file() {
            if !icon_file.is_empty() {
                let file = gio::File::for_path(&icon_file);
                let avatar_height = self.imp().avatar.size();
                let paintable =
                    gtk4::IconPaintable::for_file(&file, avatar_height, self.scale_factor());
                self.imp().avatar.set_custom_image(Some(&paintable));
            }
        }

        // Name
        self.imp().real_name.set_text(&real_name);
    }

    fn password_check(&self) {
        let lock_data = LockData::get();
        if lock_data.pwd_buffer().length() == 0 {
            return;
        }

        self.set_busy(true);
        lock_data.clear_messages();
        self.imp().status_revealer.set_reveal_child(false);

        let password = lock_data.pwd_buffer().text().to_string();
        let win = self.clone();
        pam::check_password_async(&password, move |status| {
            win.password_checked(status);
        });
    }

    fn password_checked(&self, status: PamStatus) {
        self.set_busy(false);

        match status {
            PamStatus::Error => {
                log::error!("PAM failed!");
            }
            PamStatus::AuthFailed => {
                log::error!("PAM Auth failed");
                LockData::get().add_message("Login Failed");
            }
            PamStatus::AuthSuccess => {
                FingerprintManager::get_instance().release_device();
                crate::do_unlock();
                return;
            }
        }

        self.imp().entry.grab_focus();

        // Resume fingerprint after failed attempt
        FingerprintManager::get_instance().set_suspended(false);

        self.set_status();
    }

    fn set_busy(&self, busy: bool) {
        if busy {
            if let Some(app) = self.application() {
                let guard = app.mark_busy();
                self.imp().busy_guard.replace(Some(guard));
            }
        } else {
            // Drop the guard to unmark busy
            self.imp().busy_guard.replace(None);
        }

        self.imp().entry.set_sensitive(!busy);
        if !busy {
            self.imp().entry.grab_focus();
        }
    }

    pub fn set_status(&self) {
        let status_box = self.imp().status.get();
        let status_revealer = self.imp().status_revealer.get();

        // Remove previous status widgets
        while let Some(child) = status_box.first_child() {
            status_box.remove(&child);
        }
        status_revealer.set_reveal_child(false);

        let lock_data = LockData::get();

        let errors = lock_data.errors();
        if !errors.is_empty() {
            for err in &errors {
                let banner = libadwaita::Banner::new(err);
                banner.set_revealed(true);
                banner.add_css_class("error");
                status_box.append(&banner);
            }
            status_revealer.set_reveal_child(true);
        }

        let messages = lock_data.messages();
        if !messages.is_empty() {
            for msg in &messages {
                let banner = libadwaita::Banner::new(msg);
                banner.set_revealed(true);
                status_box.append(&banner);
            }
            status_revealer.set_reveal_child(true);
        }
    }

    pub fn setup_fingerprint_ui(&self) {
        let fprint = FingerprintManager::get_instance();
        let win = self.clone();
        let win2 = self.clone();
        let win3 = self.clone();

        fprint.set_callbacks(FingerprintCallbacks {
            on_status_changed: Box::new(move |status, _is_error| {
                let lock_data = LockData::get();
                if lock_data.pwd_buffer().length() == 0 {
                    win.imp().entry.set_placeholder_text(Some(status));
                }
            }),
            on_auth_success: Box::new(move || {
                win2.imp().entry.set_placeholder_text(Some("Unlocked"));
                win2.imp().entry.set_icon_from_paintable(
                    gtk4::EntryIconPosition::Primary,
                    None::<&gdk4::Paintable>,
                );
                FingerprintManager::get_instance().release_device();
                crate::do_unlock();
            }),
            on_availability_changed: Box::new(move |available| {
                log::debug!("LockerWindow: availability_changed to {}", available);
                win3.update_fingerprint_ui(available);
            }),
        });

        self.update_fingerprint_ui(fprint.available());
    }

    fn update_fingerprint_ui(&self, available: bool) {
        let entry = self.imp().entry.get();
        if available {
            entry.set_icon_from_icon_name(
                gtk4::EntryIconPosition::Primary,
                Some("auth-fingerprint-symbolic"),
            );
            if LockData::get().pwd_buffer().length() == 0 {
                entry.set_placeholder_text(Some("Touch sensor"));
            }
        } else {
            entry.set_icon_from_paintable(
                gtk4::EntryIconPosition::Primary,
                None::<&gdk4::Paintable>,
            );
            entry.set_placeholder_text(Some(DEFAULT_PLACEHOLDER));
        }
    }

    fn check_fingerprint_on_focus(&self) {
        let fprint = FingerprintManager::get_instance();

        if !crate::fingerprint_initialized() {
            crate::set_fingerprint_initialized(true);
            log::debug!("LockerWindow: initializing fingerprint on focus");
            fprint.init();
        }

        if fprint.available() && !fprint.verifying() && !fprint.suspended() {
            fprint.start_verify();
        }

        self.update_fingerprint_ui(fprint.available());
    }
}
