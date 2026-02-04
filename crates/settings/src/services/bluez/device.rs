//! BluezDevice - D-Bus proxy for org.bluez.Device1 interface.

use std::cell::RefCell;

use gio::prelude::*;
use glib::variant::ObjectPath;

/// D-Bus proxy for BlueZ Device1 interface.
///
/// Provides async access to Bluetooth device properties and methods
/// like connection, pairing, and trust management.
pub struct BluezDevice {
    proxy: gio::DBusProxy,
    changed_handler_id: RefCell<Option<glib::SignalHandlerId>>,
}

impl BluezDevice {
    /// Create a proxy for a device at the given object path.
    pub async fn new(object_path: &str) -> Result<Self, glib::Error> {
        let proxy = gio::DBusProxy::for_bus_future(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "org.bluez",
            object_path,
            "org.bluez.Device1",
        )
        .await?;

        Ok(Self {
            proxy,
            changed_handler_id: RefCell::new(None),
        })
    }

    /// Create a proxy from an existing DBusProxy.
    pub fn from_proxy(proxy: gio::DBusProxy) -> Self {
        Self {
            proxy,
            changed_handler_id: RefCell::new(None),
        }
    }

    /// Get the object path of this device.
    pub fn object_path(&self) -> String {
        self.proxy.object_path().to_string()
    }

    /// Get the underlying proxy.
    pub fn proxy(&self) -> &gio::DBusProxy {
        &self.proxy
    }

    /// Check if the device is paired.
    pub fn paired(&self) -> bool {
        self.proxy
            .cached_property("Paired")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Check if the device is connected.
    pub fn connected(&self) -> bool {
        self.proxy
            .cached_property("Connected")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Check if the device is trusted.
    pub fn trusted(&self) -> bool {
        self.proxy
            .cached_property("Trusted")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Check if the device is blocked.
    pub fn blocked(&self) -> bool {
        self.proxy
            .cached_property("Blocked")
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }

    /// Get the device's RSSI (signal strength).
    /// Returns None if not available (e.g., paired but not in range).
    pub fn rssi(&self) -> Option<i16> {
        self.proxy
            .cached_property("RSSI")
            .and_then(|v| v.get::<i16>())
    }

    /// Get the device's friendly name/alias.
    pub fn alias(&self) -> Option<String> {
        self.proxy
            .cached_property("Alias")
            .and_then(|v| v.get::<String>())
    }

    /// Get the device's name (may be empty for unnamed devices).
    pub fn name(&self) -> Option<String> {
        self.proxy
            .cached_property("Name")
            .and_then(|v| v.get::<String>())
    }

    /// Get the device's icon name.
    pub fn icon(&self) -> Option<String> {
        self.proxy
            .cached_property("Icon")
            .and_then(|v| v.get::<String>())
    }

    /// Get the device's Bluetooth address.
    pub fn address(&self) -> Option<String> {
        self.proxy
            .cached_property("Address")
            .and_then(|v| v.get::<String>())
    }

    /// Get the adapter path this device belongs to.
    pub fn adapter(&self) -> Option<String> {
        self.proxy
            .cached_property("Adapter")
            .and_then(|v| v.get::<ObjectPath>().map(|p| p.to_string()))
    }

    /// Get the device class (type of device).
    pub fn class(&self) -> Option<u32> {
        self.proxy
            .cached_property("Class")
            .and_then(|v| v.get::<u32>())
    }

    /// Get the device's display name (alias or address).
    pub fn display_name(&self) -> String {
        self.alias()
            .or_else(|| self.name())
            .or_else(|| self.address())
            .unwrap_or_else(|| "Unknown Device".to_string())
    }

    /// Get the icon name for display (with fallback).
    pub fn display_icon(&self) -> String {
        self.icon()
            .unwrap_or_else(|| "bluetooth-symbolic".to_string())
    }

    /// Connect to the device.
    pub async fn connect(&self) -> Result<(), glib::Error> {
        self.proxy
            .call_future("Connect", None, gio::DBusCallFlags::NONE, 30000)
            .await?;
        Ok(())
    }

    /// Disconnect from the device.
    pub async fn disconnect(&self) -> Result<(), glib::Error> {
        self.proxy
            .call_future("Disconnect", None, gio::DBusCallFlags::NONE, -1)
            .await?;
        Ok(())
    }

    /// Pair with the device.
    pub async fn pair(&self) -> Result<(), glib::Error> {
        self.proxy
            .call_future("Pair", None, gio::DBusCallFlags::NONE, 60000)
            .await?;
        Ok(())
    }

    /// Cancel an ongoing pairing attempt.
    pub async fn cancel_pairing(&self) -> Result<(), glib::Error> {
        self.proxy
            .call_future("CancelPairing", None, gio::DBusCallFlags::NONE, -1)
            .await?;
        Ok(())
    }

    /// Set the device's trusted state.
    pub async fn set_trusted(&self, trusted: bool) -> Result<(), glib::Error> {
        let value = glib::Variant::from(trusted);
        self.proxy
            .call_future(
                "org.freedesktop.DBus.Properties.Set",
                Some(&("org.bluez.Device1", "Trusted", value).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Set the device's blocked state.
    pub async fn set_blocked(&self, blocked: bool) -> Result<(), glib::Error> {
        let value = glib::Variant::from(blocked);
        self.proxy
            .call_future(
                "org.freedesktop.DBus.Properties.Set",
                Some(&("org.bluez.Device1", "Blocked", value).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Connect a callback to property changes.
    pub fn connect_properties_changed<F: Fn(&str, &glib::Variant) + 'static>(&self, callback: F) {
        let handler_id = self
            .proxy
            .connect_local("g-properties-changed", false, move |values| {
                // The second argument is the changed properties dict as a{sv}
                if let Some(changed) = values.get(1).and_then(|v| v.get::<glib::Variant>().ok()) {
                    // The changed dict contains property name -> value pairs
                    // We just notify that something changed without detailed info
                    callback("properties-changed", &changed);
                }
                None
            });
        *self.changed_handler_id.borrow_mut() = Some(handler_id);
    }

    /// Disconnect from property change signals.
    pub fn disconnect_changed(&self) {
        if let Some(handler_id) = self.changed_handler_id.borrow_mut().take() {
            self.proxy.disconnect(handler_id);
        }
    }
}

impl Drop for BluezDevice {
    fn drop(&mut self) {
        self.disconnect_changed();
    }
}
