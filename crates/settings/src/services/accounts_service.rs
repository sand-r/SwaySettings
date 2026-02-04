use std::cell::RefCell;

use gio::prelude::*;

/// D-Bus proxy for AccountsService User interface.
///
/// Provides async access to read/write user properties like real name and avatar.
pub struct AccountsServiceUser {
    proxy: gio::DBusProxy,
    changed_handler_id: RefCell<Option<glib::SignalHandlerId>>,
}

impl AccountsServiceUser {
    /// Create a proxy for the current user.
    pub async fn for_current_user() -> Result<Self, glib::Error> {
        let uid = unsafe { libc::getuid() };
        let user_path = format!("/org/freedesktop/Accounts/User{}", uid);

        let proxy = gio::DBusProxy::for_bus_future(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "org.freedesktop.Accounts",
            &user_path,
            "org.freedesktop.Accounts.User",
        )
        .await?;

        Ok(Self {
            proxy,
            changed_handler_id: RefCell::new(None),
        })
    }

    /// Get the system username.
    pub fn user_name(&self) -> Option<String> {
        self.proxy
            .cached_property("UserName")
            .and_then(|v| v.get::<String>())
    }

    /// Get the user's real (display) name.
    pub fn real_name(&self) -> Option<String> {
        self.proxy
            .cached_property("RealName")
            .and_then(|v| v.get::<String>())
    }

    /// Get the path to the user's avatar icon file.
    pub fn icon_file(&self) -> Option<String> {
        self.proxy
            .cached_property("IconFile")
            .and_then(|v| v.get::<String>())
    }

    /// Check if this is a system account.
    pub fn system_account(&self) -> bool {
        self.proxy
            .cached_property("SystemAccount")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Set the user's real (display) name.
    pub async fn set_real_name(&self, name: &str) -> Result<(), glib::Error> {
        self.proxy
            .call_future(
                "SetRealName",
                Some(&(name,).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Set the path to the user's avatar icon file.
    pub async fn set_icon_file(&self, path: &str) -> Result<(), glib::Error> {
        self.proxy
            .call_future(
                "SetIconFile",
                Some(&(path,).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Connect a callback to the Changed signal.
    ///
    /// The callback will be invoked when any user property changes.
    pub fn connect_changed<F: Fn() + 'static>(&self, callback: F) {
        let handler_id = self
            .proxy
            .connect_local("g-properties-changed", false, move |_values| {
                callback();
                None
            });
        *self.changed_handler_id.borrow_mut() = Some(handler_id);
    }

    /// Disconnect from the Changed signal.
    pub fn disconnect_changed(&self) {
        if let Some(handler_id) = self.changed_handler_id.borrow_mut().take() {
            self.proxy.disconnect(handler_id);
        }
    }
}

impl Drop for AccountsServiceUser {
    fn drop(&mut self) {
        self.disconnect_changed();
    }
}
