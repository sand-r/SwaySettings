using Gee;

namespace SwaySettings {
    [GtkTemplate (ui = "/org/erikreider/swaysettings/ui/MouseContent.ui")]
    public class MouseContent : Adw.Bin {
        [GtkChild]
        unowned Gtk.Stack stack;

        // General
        [GtkChild]
        unowned Adw.SwitchRow natural_scroll_row;
        [GtkChild]
        unowned Gtk.Scale scroll_factor_scale;

        // Speed
        [GtkChild]
        unowned Adw.ComboRow accel_profile_row;
        [GtkChild]
        unowned Gtk.Scale pointer_accel_scale;

        private IPC ipc;
        private InputDevice? device = null;
        private bool updating = false;

        public MouseContent (IPC ipc) {
            this.ipc = ipc;
        }

        construct {
            // Set up combo row models
            setup_combo_row (accel_profile_row, typeof (AccelProfiles));

            // Connect signals
            natural_scroll_row.notify["active"].connect (() => {
                if (updating || device == null) return;
                var val = natural_scroll_row.get_active ();
                device.data.natural_scroll = BoolEnum.from_bool (val);
                write_setting ("natural_scroll %s".printf (val.to_string ()));
            });

            scroll_factor_scale.value_changed.connect (() => {
                if (updating || device == null) return;
                var val = scroll_factor_scale.get_value ();
                device.scroll_factor = val;
                var str_val = val.to_string ().replace (",", ".");
                write_setting ("scroll_factor %s".printf (str_val));
            });

            accel_profile_row.notify["selected"].connect (() => {
                if (updating || device == null) return;
                var val = (AccelProfiles) accel_profile_row.get_selected ();
                device.data.accel_profile = val;
                write_setting ("accel_profile %s".printf (val.parse ()));
            });

            pointer_accel_scale.value_changed.connect (() => {
                if (updating || device == null) return;
                var val = pointer_accel_scale.get_value ();
                device.data.accel_speed = val;
                var str_val = val.to_string ().replace (",", ".");
                write_setting ("pointer_accel %s".printf (str_val));
            });

            // Add marks to scales
            scroll_factor_scale.add_mark (1.0, Gtk.PositionType.BOTTOM, null);
            pointer_accel_scale.add_mark (0.0, Gtk.PositionType.BOTTOM, null);
        }

        private void setup_combo_row (Adw.ComboRow row, GLib.Type enum_type) {
            var enumc = (EnumClass) enum_type.class_ref ();
            var model = new GLib.ListStore (typeof (Gtk.StringObject));

            foreach (var value in enumc.values) {
                weak string nick = value.value_nick;
                // Convert snake_case to Title Case
                var parts = nick.split ("_");
                string[] titled = {};
                foreach (var part in parts) {
                    if (part.length > 0) {
                        titled += part.up (1) + part.slice (1, part.length);
                    }
                }
                string name = string.joinv (" ", titled);
                model.append (new Gtk.StringObject (name));
            }

            row.set_model (model);
        }

        public void load_device (InputDevice? dev) {
            this.device = dev;
            updating = true;

            if (device == null) {
                stack.set_visible_child_name ("placeholder");
                return;
            }

            stack.set_visible_child_name ("page");

            // Load values
            natural_scroll_row.set_active (device.data.natural_scroll.to_bool ());
            scroll_factor_scale.set_value (device.scroll_factor);
            accel_profile_row.set_selected (device.data.accel_profile);
            pointer_accel_scale.set_value (device.data.accel_speed);

            updating = false;
        }

        private void write_setting (string setting) {
            if (device == null) return;

            string cmd = "input type:pointer %s".printf (setting);
            ipc.run_command (cmd);
            Functions.write_settings (
                Strings.SETTINGS_FOLDER_INPUT_POINTER,
                device.get_settings ());
        }
    }
}
