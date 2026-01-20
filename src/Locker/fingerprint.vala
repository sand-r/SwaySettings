// Fingerprint authentication via fprintd D-Bus interface
// Similar to gtklock implementation

public class FingerprintManager : Object {
    private static FingerprintManager? _instance = null;

    // D-Bus proxies
    private DBusProxy? fprint_manager_proxy = null;
    private DBusProxy? fprint_device_proxy = null;

    // State flags
    private bool _available = false;
    private bool _verifying = false;
    private bool _claimed = false;
    private bool _suspended = false;
    private int _claim_retry_count = 0;

    // Signal handler ID
    private ulong verify_status_handler_id = 0;


    // Signals
    public signal void status_changed (string status, bool is_error);
    public signal void auth_success ();
    public signal void availability_changed (bool available);

    public bool available {
        get { return _available; }
    }

    public bool verifying {
        get { return _verifying; }
    }

    public bool suspended {
        get { return _suspended; }
        set {
            _suspended = value;
            if (_suspended) {
                stop_verify ();
            } else if (_available && _claimed) {
                start_verify ();
            }
        }
    }

    public static FingerprintManager get_instance () {
        if (_instance == null) {
            _instance = new FingerprintManager ();
        }
        return _instance;
    }

    private FingerprintManager () {
    }

    public void init () {
        debug ("Fingerprint: starting init");
        _claim_retry_count = 0;  // Reset for new session
        try {
            // Connect to fprintd Manager
            fprint_manager_proxy = new DBusProxy.for_bus_sync (
                BusType.SYSTEM,
                DBusProxyFlags.NONE,
                null,
                "net.reactivated.Fprint",
                "/net/reactivated/Fprint/Manager",
                "net.reactivated.Fprint.Manager",
                null
            );
            debug ("Fingerprint: connected to fprintd manager");
        } catch (Error e) {
            debug ("Fingerprint: could not connect to fprintd manager: %s", e.message);
            _available = false;
            return;
        }

        // Get default device
        try {
            Variant result = fprint_manager_proxy.call_sync (
                "GetDefaultDevice",
                null,
                DBusCallFlags.NONE,
                -1,
                null
            );

            string device_path;
            result.get ("(o)", out device_path);
            debug ("Fingerprint: device path = %s", device_path ?? "null");

            if (device_path == null || device_path.length == 0) {
                debug ("Fingerprint: no device found");
                _available = false;
                return;
            }

            // Connect to the device
            fprint_device_proxy = new DBusProxy.for_bus_sync (
                BusType.SYSTEM,
                DBusProxyFlags.NONE,
                null,
                "net.reactivated.Fprint",
                device_path,
                "net.reactivated.Fprint.Device",
                null
            );
            debug ("Fingerprint: connected to device");

            // Try to claim the device (with retry for busy device)
            // Use delayed claim to give hardware time to settle after previous session
            claim_with_retry ();

        } catch (Error e) {
            debug ("Fingerprint: error getting device: %s", e.message);
            _available = false;
        }
    }

    private void reinitialize_device () {
        debug ("Fingerprint: reinitializing device after suspend");

        // Clean up old state
        if (verify_status_handler_id > 0 && fprint_device_proxy != null) {
            SignalHandler.disconnect (fprint_device_proxy, verify_status_handler_id);
            verify_status_handler_id = 0;
        }

        // Try to release if we think we're claimed
        if (fprint_device_proxy != null && _claimed) {
            try {
                fprint_device_proxy.call_sync ("Release", null, DBusCallFlags.NONE, -1, null);
            } catch (Error e) {
                // Ignore
            }
        }

        _claimed = false;
        _verifying = false;
        fprint_device_proxy = null;

        // Re-connect to device and claim
        try {
            Variant result = fprint_manager_proxy.call_sync (
                "GetDefaultDevice", null, DBusCallFlags.NONE, -1, null);

            string device_path;
            result.get ("(o)", out device_path);

            if (device_path == null || device_path.length == 0) {
                _available = false;
                availability_changed (false);
                return;
            }

            fprint_device_proxy = new DBusProxy.for_bus_sync (
                BusType.SYSTEM, DBusProxyFlags.NONE, null,
                "net.reactivated.Fprint", device_path,
                "net.reactivated.Fprint.Device", null);

            _claim_retry_count = 0;
            claim_with_retry ();

        } catch (Error e) {
            debug ("Fingerprint: reinit failed: %s", e.message);
            _available = false;
            availability_changed (false);
        }
    }

    private void claim_with_retry () {
        if (claim_device ()) {
            _available = true;
            debug ("Fingerprint: device claimed, available = true");
            availability_changed (_available);
            start_verify ();
        } else if (_claim_retry_count < 10) {
            // Device might be busy from previous process, retry after delay
            _claim_retry_count++;
            // Exponential backoff: 500ms, 1000ms, 1500ms, 2000ms...
            uint delay = 500 * _claim_retry_count;
            debug ("Fingerprint: claim failed, retry %d in %ums", _claim_retry_count, delay);
            Timeout.add (delay, () => {
                claim_with_retry ();
                return false;
            });
        } else {
            debug ("Fingerprint: claim failed after %d retries", _claim_retry_count);
            _available = false;
            availability_changed (false);
        }
    }

    private bool claim_device () {
        if (fprint_device_proxy == null) {
            debug ("Fingerprint: claim_device - proxy is null");
            return false;
        }

        // Try to release first in case device is stuck from previous session
        try {
            fprint_device_proxy.call_sync (
                "Release",
                null,
                DBusCallFlags.NONE,
                -1,
                null
            );
            debug ("Fingerprint: released stale claim");
        } catch (Error e) {
            debug ("Fingerprint: pre-release failed (expected): %s", e.message);
        }

        try {
            // Claim for current user (empty string = current user)
            fprint_device_proxy.call_sync (
                "Claim",
                new Variant ("(s)", ""),
                DBusCallFlags.NONE,
                -1,
                null
            );

            _claimed = true;
            debug ("Fingerprint: device claimed successfully");

            // Connect to VerifyStatus signal (D-Bus signal from device)
            verify_status_handler_id = fprint_device_proxy.g_signal.connect (on_verify_status);
            debug ("Fingerprint: signal handler connected");

            return true;

        } catch (Error e) {
            debug ("Fingerprint: claim error: %s", e.message);
            if (e.message.contains ("NoEnrolledPrints")) {
                status_changed ("No fingerprints", false);
            }
            return false;
        }
    }

    public void release_device () {
        debug ("Fingerprint: release_device called");
        stop_verify ();

        if (fprint_device_proxy != null && _claimed) {
            if (verify_status_handler_id > 0) {
                SignalHandler.disconnect (fprint_device_proxy, verify_status_handler_id);
                verify_status_handler_id = 0;
            }

            try {
                fprint_device_proxy.call_sync (
                    "Release",
                    null,
                    DBusCallFlags.NONE,
                    -1,
                    null
                );
            } catch (Error e) {
                debug ("Error releasing fingerprint device: %s", e.message);
            }

            _claimed = false;
        }
    }

    public void start_verify () {
        if (fprint_device_proxy == null || _verifying || _suspended || !_claimed) {
            return;
        }

        try {
            fprint_device_proxy.call_sync (
                "VerifyStart",
                new Variant ("(s)", "any"),
                DBusCallFlags.NONE,
                -1,
                null
            );

            _verifying = true;
            status_changed ("Touch sensor", false);

        } catch (Error e) {
            debug ("Could not start fingerprint verification: %s", e.message);
        }
    }

    public void stop_verify () {
        if (fprint_device_proxy == null || !_verifying) {
            return;
        }

        try {
            fprint_device_proxy.call_sync (
                "VerifyStop",
                null,
                DBusCallFlags.NONE,
                -1,
                null
            );
        } catch (Error e) {
            // May fail if already stopped, that's ok
            debug ("Error stopping verification: %s", e.message);
        }

        _verifying = false;
    }

    private void on_verify_status (DBusProxy proxy,
                                    string? sender_name,
                                    string signal_name,
                                    Variant parameters) {
        if (signal_name != "VerifyStatus") {
            return;
        }

        if (_suspended) {
            return;
        }

        string status;
        bool done;
        parameters.get ("(sb)", out status, out done);

        debug ("Fingerprint status: %s, done: %s", status, done.to_string ());

        switch (status) {
            case "verify-match":
                status_changed ("Fingerprint matched", false);
                auth_success ();
                return;

            case "verify-no-match":
                status_changed ("Not recognized", true);
                break;

            case "verify-retry-scan":
                status_changed ("Try again", false);
                break;

            case "verify-swipe-too-short":
                status_changed ("Swipe too short", false);
                break;

            case "verify-finger-not-centered":
                status_changed ("Center your finger", false);
                break;

            case "verify-remove-and-retry":
                status_changed ("Retry", false);
                break;

            case "verify-disconnected":
                status_changed ("Disconnected", true);
                _available = false;
                availability_changed (false);
                return;

            case "verify-unknown-error":
                // Often happens after resume from suspend - need full re-init
                status_changed ("Reconnecting...", false);
                _verifying = false;
                // Release and re-initialize after delay
                Timeout.add (1000, () => {
                    if (!_suspended) {
                        reinitialize_device ();
                    }
                    return false;
                });
                return;  // Don't fall through to normal done handling

            default:
                status_changed (status, false);
                break;
        }

        if (done) {
            // Stop the previous verify session before restarting, otherwise some
            // fprintd/libfprint stacks will reject the next VerifyStart.
            stop_verify ();
            // Restart verification after a delay
            uint delay = (status == "verify-unknown-error") ? 2000 : 1500;
            Timeout.add (delay, () => {
                if (!_suspended && _available && _claimed) {
                    start_verify ();
                }
                return false;
            });
        }
    }

}
