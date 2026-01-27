namespace SwaySettings {
    public class SoundPage : PageScroll {
        SoundContent content;

        public SoundPage (SettingsItem item, Adw.NavigationPage page) {
            base (item, page);
        }

        public override Gtk.Widget set_child () {
            this.content = new SoundContent ();
            return this.content;
        }
    }

    [GtkTemplate (ui = "/org/erikreider/swaysettings/ui/SoundContent.ui")]
    private class SoundContent : Adw.Bin {
        [GtkChild]
        unowned Adw.PreferencesGroup output_group;
        [GtkChild]
        unowned Adw.ComboRow output_device_row;
        [GtkChild]
        unowned Gtk.Scale output_slider;
        [GtkChild]
        unowned Gtk.ToggleButton output_mute_toggle;

        [GtkChild]
        unowned Adw.PreferencesGroup input_group;
        [GtkChild]
        unowned Adw.ComboRow input_device_row;
        [GtkChild]
        unowned Gtk.Scale input_slider;
        [GtkChild]
        unowned Gtk.ToggleButton input_mute_toggle;

        private WpAudioDaemon daemon = new WpAudioDaemon ();
        private GLib.ListStore sink_list = new GLib.ListStore (typeof (WpAudioDevice));
        private GLib.ListStore source_list = new GLib.ListStore (typeof (WpAudioDevice));

        private bool updating_output = false;
        private bool updating_input = false;

        construct {
            // Set up device combo rows
            output_device_row.set_model (sink_list);
            output_device_row.set_expression (new Gtk.PropertyExpression (
                typeof (WpAudioDevice), null, "description"));

            input_device_row.set_model (source_list);
            input_device_row.set_expression (new Gtk.PropertyExpression (
                typeof (WpAudioDevice), null, "description"));

            // Mute toggle bindings
            output_mute_toggle.bind_property ("active", output_slider, "sensitive",
                BindingFlags.INVERT_BOOLEAN);
            input_mute_toggle.bind_property ("active", input_slider, "sensitive",
                BindingFlags.INVERT_BOOLEAN);

            // Connect UI signals
            output_device_row.notify["selected-item"].connect (on_output_device_changed);
            output_slider.value_changed.connect (on_output_volume_changed);
            output_mute_toggle.toggled.connect (on_output_mute_toggled);

            input_device_row.notify["selected-item"].connect (on_input_device_changed);
            input_slider.value_changed.connect (on_input_volume_changed);
            input_mute_toggle.toggled.connect (on_input_mute_toggled);

            // Connect daemon signals
            daemon.on_ready.connect (on_daemon_ready);
            daemon.device_added.connect (on_device_added);
            daemon.device_removed.connect (on_device_removed);
            daemon.device_changed.connect (on_device_changed);
            daemon.default_sink_changed.connect (on_default_sink_changed);
            daemon.default_source_changed.connect (on_default_source_changed);

            // Start disabled
            output_group.set_sensitive (false);
            input_group.set_sensitive (false);

            // Start daemon
            daemon.start ();
        }

        ~SoundContent () {
            daemon.stop ();
        }

        private void on_daemon_ready () {
            output_group.set_sensitive (sink_list.get_n_items () > 0);
            input_group.set_sensitive (source_list.get_n_items () > 0);
        }

        private void on_device_added (WpAudioDevice device) {
            // Skip unavailable devices (unconnected HDMI/DP, monitors, etc.)
            if (!device.is_available) {
                return;
            }

            if (device.is_sink) {
                sink_list.append (device);
                output_group.set_sensitive (true);

                if (device.is_default) {
                    select_device (output_device_row, sink_list, device);
                    update_output_ui (device);
                }
            } else if (device.is_source) {
                source_list.append (device);
                input_group.set_sensitive (true);

                if (device.is_default) {
                    select_device (input_device_row, source_list, device);
                    update_input_ui (device);
                }
            }
        }

        private void on_device_removed (WpAudioDevice device) {
            if (device.is_sink) {
                remove_from_list (sink_list, device);
                output_group.set_sensitive (sink_list.get_n_items () > 0);
            } else if (device.is_source) {
                remove_from_list (source_list, device);
                input_group.set_sensitive (source_list.get_n_items () > 0);
            }
        }

        private void on_device_changed (WpAudioDevice device) {
            // Handle availability changes
            bool in_list = is_in_list (device.is_sink ? sink_list : source_list, device);

            if (device.is_available && !in_list) {
                // Device became available - add it
                on_device_added (device);
                return;
            } else if (!device.is_available && in_list) {
                // Device became unavailable - remove it
                on_device_removed (device);
                return;
            }

            // Update UI if this is the selected device
            if (device.is_sink) {
                var selected = output_device_row.get_selected_item () as WpAudioDevice;
                if (selected != null && selected.id == device.id) {
                    update_output_ui (device);
                }
            } else if (device.is_source) {
                var selected = input_device_row.get_selected_item () as WpAudioDevice;
                if (selected != null && selected.id == device.id) {
                    update_input_ui (device);
                }
            }
        }

        private bool is_in_list (GLib.ListStore list, WpAudioDevice device) {
            for (uint i = 0; i < list.get_n_items (); i++) {
                var item = list.get_item (i) as WpAudioDevice;
                if (item != null && item.id == device.id) {
                    return true;
                }
            }
            return false;
        }

        private void on_default_sink_changed (WpAudioDevice? device) {
            if (device != null) {
                select_device (output_device_row, sink_list, device);
                update_output_ui (device);
            }
        }

        private void on_default_source_changed (WpAudioDevice? device) {
            if (device != null) {
                select_device (input_device_row, source_list, device);
                update_input_ui (device);
            }
        }

        private void on_output_device_changed () {
            if (updating_output) return;

            var device = output_device_row.get_selected_item () as WpAudioDevice;
            if (device != null) {
                daemon.set_default_sink (device);
                update_output_ui (device);
            }
        }

        private void on_output_volume_changed () {
            if (updating_output) return;

            var device = output_device_row.get_selected_item () as WpAudioDevice;
            if (device != null) {
                daemon.set_volume (device, output_slider.get_value () / 100.0);
            }
        }

        private void on_output_mute_toggled () {
            if (updating_output) return;

            var device = output_device_row.get_selected_item () as WpAudioDevice;
            if (device != null) {
                daemon.set_mute (device, output_mute_toggle.active);
            }
        }

        private void on_input_device_changed () {
            if (updating_input) return;

            var device = input_device_row.get_selected_item () as WpAudioDevice;
            if (device != null) {
                daemon.set_default_source (device);
                update_input_ui (device);
            }
        }

        private void on_input_volume_changed () {
            if (updating_input) return;

            var device = input_device_row.get_selected_item () as WpAudioDevice;
            if (device != null) {
                daemon.set_volume (device, input_slider.get_value () / 100.0);
            }
        }

        private void on_input_mute_toggled () {
            if (updating_input) return;

            var device = input_device_row.get_selected_item () as WpAudioDevice;
            if (device != null) {
                daemon.set_mute (device, input_mute_toggle.active);
            }
        }

        private void update_output_ui (WpAudioDevice device) {
            updating_output = true;
            output_slider.set_value (device.volume * 100.0);
            output_mute_toggle.set_active (device.is_muted);
            updating_output = false;
        }

        private void update_input_ui (WpAudioDevice device) {
            updating_input = true;
            input_slider.set_value (device.volume * 100.0);
            input_mute_toggle.set_active (device.is_muted);
            updating_input = false;
        }

        private void select_device (Adw.ComboRow row, GLib.ListStore list, WpAudioDevice device) {
            for (uint i = 0; i < list.get_n_items (); i++) {
                var item = list.get_item (i) as WpAudioDevice;
                if (item != null && item.id == device.id) {
                    if (row == output_device_row) {
                        updating_output = true;
                    } else {
                        updating_input = true;
                    }
                    row.set_selected (i);
                    if (row == output_device_row) {
                        updating_output = false;
                    } else {
                        updating_input = false;
                    }
                    return;
                }
            }
        }

        private void remove_from_list (GLib.ListStore list, WpAudioDevice device) {
            for (uint i = 0; i < list.get_n_items (); i++) {
                var item = list.get_item (i) as WpAudioDevice;
                if (item != null && item.id == device.id) {
                    list.remove (i);
                    return;
                }
            }
        }
    }
}
