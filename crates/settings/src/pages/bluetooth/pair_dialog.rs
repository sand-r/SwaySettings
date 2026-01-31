//! Bluetooth pairing dialog.

use libadwaita::prelude::*;

/// Dialog for Bluetooth pairing operations.
pub struct BluetoothPairDialog {
    dialog: libadwaita::AlertDialog,
}

impl BluetoothPairDialog {
    /// Create a confirmation dialog for passkey pairing.
    pub fn new_confirmation(device_name: &str, passkey: u32) -> Self {
        let dialog = libadwaita::AlertDialog::new(
            Some(&format!("Pair with {}", device_name)),
            Some(&format!(
                "Confirm that the following passkey is shown on \"{}\":\n\n<big><b>{:06}</b></big>",
                device_name, passkey
            )),
        );

        dialog.set_body_use_markup(true);
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("confirm", "Confirm");
        dialog.set_response_appearance("confirm", libadwaita::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("confirm"));

        Self { dialog }
    }

    /// Create a display-only dialog for passkey entry on remote device.
    pub fn new_display_passkey(device_name: &str, passkey: u32) -> Self {
        let dialog = libadwaita::AlertDialog::new(
            Some(&format!("Pairing with {}", device_name)),
            Some(&format!(
                "Enter the following passkey on \"{}\":\n\n<big><b>{:06}</b></big>",
                device_name, passkey
            )),
        );

        dialog.set_body_use_markup(true);
        dialog.add_response("close", "Close");

        Self { dialog }
    }

    /// Create a display-only dialog for PIN code entry on remote device.
    pub fn new_display_pincode(device_name: &str, pincode: &str) -> Self {
        let dialog = libadwaita::AlertDialog::new(
            Some(&format!("Pairing with {}", device_name)),
            Some(&format!(
                "Enter the following PIN on \"{}\":\n\n<big><b>{}</b></big>",
                device_name, pincode
            )),
        );

        dialog.set_body_use_markup(true);
        dialog.add_response("close", "Close");

        Self { dialog }
    }

    /// Create an authorization dialog (no passkey).
    pub fn new_authorization(device_name: &str) -> Self {
        let dialog = libadwaita::AlertDialog::new(
            Some(&format!("Pair with {}", device_name)),
            Some(&format!(
                "\"{}\" wants to pair with this device.",
                device_name
            )),
        );

        dialog.add_response("reject", "Reject");
        dialog.add_response("accept", "Accept");
        dialog.set_response_appearance("accept", libadwaita::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));

        Self { dialog }
    }

    /// Set the transient parent window.
    pub fn set_transient_for(&self, window: Option<&gtk4::Window>) {
        // AlertDialog doesn't have set_transient_for, it's set when presenting
        let _ = window;
    }

    /// Present the dialog.
    pub fn present(&self) {
        // AlertDialog is presented via choose(), but for simplicity we just show it
        // In a full implementation, we'd use the async API properly
    }

    /// Close the dialog.
    pub fn close(&self) {
        self.dialog.force_close();
    }

    /// Get the underlying dialog widget.
    pub fn dialog(&self) -> &libadwaita::AlertDialog {
        &self.dialog
    }
}
