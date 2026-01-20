using PulseAudio;
using Gee;

namespace SwaySettings {
    public class PulsePage : PageScroll {
        PulseContent content;

        public PulsePage (SettingsItem item, Adw.NavigationPage page) {
            base (item, page);
        }

        public override Gtk.Widget set_child () {
            this.content = new PulseContent ();
            return this.content;
        }

        public override async void on_back (Adw.NavigationPage page) {
            yield this.content.on_back ();
        }
    }

    [GtkTemplate (ui = "/org/erikreider/swaysettings/ui/PulseContent.ui")]
    private class PulseContent : Adw.Bin {
        public const string OUTPUT_ICON_MUTED = "audio-volume-muted-symbolic";
        public const string OUTPUT_ICON_UNMUTED = "audio-volume-high-symbolic";
        public const string INPUT_ICON_MUTED = "audio-input-microphone-muted-symbolic";
        public const string INPUT_ICON_UNMUTED = "audio-input-microphone-symbolic";

        [GtkChild]
        unowned Gtk.Stack stack;

        [GtkChild]
        unowned Gtk.Box pulse_page;
        [GtkChild]
        unowned Adw.StatusPage error_page;

        // Sink
        [GtkChild]
        unowned Adw.PreferencesGroup output_group;
        [GtkChild]
        unowned Adw.ActionRow output_balance_row;
        [GtkChild]
        unowned Gtk.Scale output_slider;
        [GtkChild]
        unowned Gtk.Scale output_balance_slider;
        [GtkChild]
        unowned Gtk.ToggleButton output_mute_toggle;
        [GtkChild]
        unowned Adw.ComboRow output_device_row;
        GLib.ListStore sink_list_store;

        // Bluetooth Profile ComboBox
        [GtkChild]
        unowned Adw.ComboRow output_profile_row;
        GLib.ListStore profile_list_store;

        // Source
        [GtkChild]
        unowned Adw.PreferencesGroup input_group;
        [GtkChild]
        unowned Gtk.Scale input_slider;
        [GtkChild]
        unowned Gtk.ToggleButton input_mute_toggle;
        [GtkChild]
        unowned Adw.ComboRow input_device_row;
        GLib.ListStore source_list_store;

        // Sink inputs
        [GtkChild]
        public unowned Gtk.ListBox levels_listbox;
        [GtkChild]
        public unowned Adw.PreferencesGroup sink_inputs_group;

        // Speaker test
        [GtkChild]
        unowned Gtk.MenuButton test_button;
        [GtkChild]
        unowned Gtk.Button test_left_button;
        [GtkChild]
        unowned Gtk.Button test_right_button;

        private PulseDevice ? default_sink = null;
        private PulseDevice ? default_source = null;
        private bool updating_balance = false;
        private bool updating_output_selection = false;
        private bool updating_input_selection = false;
        private bool updating_profile_selection = false;

        private PulseDaemon client = new PulseDaemon ();

        private Canberra.Context? canberra_context = null;

        construct {
            this.client.change_device.connect (device_change);
            this.client.new_device.connect (device_added);
            this.client.remove_device.connect (device_removed);

            this.client.change_active_sink.connect (active_sink_change);
            this.client.new_active_sink.connect (active_sink_added);
            this.client.remove_active_sink.connect (active_sink_removed);

            this.client.change_default_device.connect (default_device_changed);

            stack.set_visible_child_name (client.running ? "pulse_page" : "error_page");
            this.client.notify["running"].connect (() => {
                stack.set_visible_child_name (client.running ? "pulse_page" : "error_page");
            });

            // Initialize canberra for speaker testing
            init_canberra ();
            test_left_button.clicked.connect (() => play_test_sound ("front-left"));
            test_right_button.clicked.connect (() => play_test_sound ("front-right"));

            // UI signals
            output_mute_toggle.bind_property ("active",
                                              output_slider, "sensitive",
                                              BindingFlags.INVERT_BOOLEAN);
            output_mute_toggle.toggled.connect ((b) => {
                this.client.set_device_mute (b.active, default_sink);
            });
            input_mute_toggle.bind_property ("active",
                                             input_slider, "sensitive",
                                             BindingFlags.INVERT_BOOLEAN);
            input_mute_toggle.toggled.connect ((b) => {
                this.client.set_device_mute (b.active, default_source);
            });

            output_device_row.notify["selected-item"].connect (() => {
                device_row_changed.begin (output_device_row, false);
            });
            output_profile_row.notify["selected-item"].connect (() => {
                profile_row_changed.begin ();
            });
            input_device_row.notify["selected-item"].connect (() => {
                device_row_changed.begin (input_device_row, true);
            });

            output_slider.value_changed.connect (() => {
                this.client.set_device_volume (
                    default_sink,
                    (float) output_slider.get_value ());
            });
            output_balance_slider.value_changed.connect (() => {
                if (updating_balance) {
                    return;
                }
                if (default_sink == null
                    || !default_sink.channel_map.can_balance ()) {
                    return;
                }
                this.client.set_device_balance (default_sink,
                                                output_balance_slider.get_value ());
            });
            input_slider.value_changed.connect (() => {
                this.client.set_device_volume (
                    default_source,
                    (float) input_slider.get_value ());
            });

            output_group.set_sensitive (false);
            input_group.set_sensitive (false);
            sink_inputs_group.set_sensitive (false);
            output_balance_row.set_visible (false);
            output_profile_row.set_visible (false);
        }

        public PulseContent () {
            // Sinks
            sink_list_store = new GLib.ListStore (typeof (PulseDevice));

            foreach (var item in this.client.sinks.values) {
                device_added (item);
            }
            output_device_row.set_model (sink_list_store);
            output_device_row.set_factory (create_device_factory ());
            output_device_row.set_list_factory (create_device_factory ());

            // Sink Bluetooth Profiles
            profile_list_store = new GLib.ListStore (typeof (PulseCardProfile));
            output_profile_row.set_model (profile_list_store);
            Gtk.PropertyExpression profile_expression =
                new Gtk.PropertyExpression (typeof (PulseCardProfile),
                                            null,
                                            "description");
            output_profile_row.set_expression (profile_expression);

            output_mute_toggle.toggled.connect (mute_toggle_cb);

            // Sources
            source_list_store = new GLib.ListStore (typeof (PulseDevice));

            foreach (var item in this.client.sources.values) {
                device_added (item);
            }
            input_device_row.set_model (source_list_store);
            input_device_row.set_factory (create_device_factory ());
            input_device_row.set_list_factory (create_device_factory ());

            input_mute_toggle.toggled.connect (mute_toggle_cb);

            // Active sink inputs
            foreach (var item in this.client.active_sinks.values) {
                levels_listbox.append (new SinkInputRow (item, client));
            }
            if (client.active_sinks.values.size > 0) {
                sink_inputs_group.set_sensitive (true);
            }
            levels_listbox.set_sort_func (this.active_sinks_list_store_sort);

            // Begin
            this.client.start ();
        }

        public async void on_back () {
            this.client.change_device.disconnect (device_change);
            this.client.new_device.disconnect (device_added);
            this.client.remove_device.disconnect (device_removed);

            this.client.change_active_sink.disconnect (active_sink_change);
            this.client.new_active_sink.disconnect (active_sink_added);
            this.client.remove_active_sink.disconnect (active_sink_removed);

            this.client.change_default_device.disconnect (default_device_changed);

            this.client.close ();
        }

        private void mute_toggle_cb (Gtk.ToggleButton button) {
            bool is_input = button == input_mute_toggle;
            string icon = button.active
                ? (is_input ? INPUT_ICON_MUTED : OUTPUT_ICON_MUTED)
                : (is_input ? INPUT_ICON_UNMUTED : OUTPUT_ICON_UNMUTED);
            button.set_icon_name (icon);
        }

        private async void device_row_changed (Adw.ComboRow row, bool is_input) {
            if (is_input ? updating_input_selection : updating_output_selection) {
                return;
            }
            PulseDevice ? device = row.get_selected_item () as PulseDevice;
            if (device == null) return;
            PulseDevice ? cmp_device = is_input ? default_source : default_sink;

            // Check if setting the same device (compare identity, not state)
            if (cmp_device != null &&
                device.get_current_hash_key () == cmp_device.get_current_hash_key ()) return;

            yield this.client.set_default_device (device);
        }

        private async void profile_row_changed () {
            if (updating_profile_selection) {
                return;
            }
            PulseCardProfile ? profile = output_profile_row.get_selected_item ()
                as PulseCardProfile;
            if (profile == null) return;
            PulseDevice ? device = output_device_row.get_selected_item ()
                as PulseDevice;

            // Check if setting the same profile
            if (device != null &&
                device.active_profile != null &&
                profile.cmp (device.active_profile)) return;

            yield this.client.set_bluetooth_card_profile (profile, device);
        }

        /*
         * Getters
         */

        private PulseDevice ? get_selected_device (Adw.ComboRow row) {
            return row.get_selected_item () as PulseDevice;
        }

        private uint find_device_position (GLib.ListStore list_store,
                                           PulseDevice device) {
            string key = device.get_current_hash_key ();
            for (uint i = 0; i < list_store.get_n_items (); i++) {
                PulseDevice ? item = list_store.get_item (i) as PulseDevice;
                if (item != null && item.get_current_hash_key () == key) {
                    return i;
                }
            }
            return uint.MAX;
        }

        private bool should_show_device (PulseDevice device) {
            if (device == null || device.removed) {
                return false;
            }
            // Cardless devices (including virtual sinks/sources) always show
            if (!device.has_card) {
                return true;
            }
            // Default device always shows
            if (device.is_default) {
                return true;
            }
            // Devices with cards: hide only if port is definitely unavailable (NO)
            // UNKNOWN means no jack detection, so show it (benefit of the doubt)
            return device.port_available != PortAvailable.NO;
        }

        private string get_device_icon_name (PulseDevice device) {
            string icon = device.icon_name ?? "";
            if (icon == "") {
                icon = device.direction == Direction.INPUT
                    ? "audio-input-microphone-symbolic"
                    : "audio-speakers-symbolic";
            }
            if (!icon.has_suffix ("-symbolic")) {
                icon += "-symbolic";
            }
            return icon;
        }

        private Gtk.SignalListItemFactory create_device_factory () {
            var factory = new Gtk.SignalListItemFactory ();
            factory.setup.connect ((list_item) => {
                var list_item_widget = list_item as Gtk.ListItem;
                if (list_item_widget == null) return;
                var box = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
                var image = new Gtk.Image ();
                image.set_pixel_size (16);
                var label = new Gtk.Label (null);
                label.set_xalign (0);
                label.set_hexpand (true);
                label.set_ellipsize (Pango.EllipsizeMode.END);
                box.append (image);
                box.append (label);
                list_item_widget.set_child (box);
            });
            factory.bind.connect ((list_item) => {
                var list_item_widget = list_item as Gtk.ListItem;
                if (list_item_widget == null) return;
                var device = list_item_widget.get_item () as PulseDevice;
                if (device == null) return;
                var box = (Gtk.Box) list_item_widget.get_child ();
                var image = (Gtk.Image) box.get_first_child ();
                var label = (Gtk.Label) image.get_next_sibling ();
                image.set_from_icon_name (get_device_icon_name (device));
                label.set_text (device.get_display_name () ?? "");
            });
            return factory;
        }

        private void update_group_sensitivity (bool is_input, GLib.ListStore list_store) {
            (is_input ? input_group : output_group).set_sensitive (
                list_store.get_n_items () > 0);
        }

        private void set_device_profiles (PulseDevice device) {
            updating_profile_selection = true;
            profile_list_store.remove_all ();
            uint default_profile = 0;
            uint index = 0;
            foreach (var profile in device.profiles.data) {
                profile_list_store.append (profile);

                // Check if active profile
                if (profile.name == device.card_active_profile) {
                    default_profile = index;
                }
                index++;
            }

            output_profile_row.set_selected (default_profile);
            updating_profile_selection = false;
        }

        /*
         * Sinks/Sources
         */

        private void device_change (PulseDevice device) {
            bool is_input = device.direction == Direction.INPUT;
            GLib.ListStore list_store =
                is_input ? source_list_store : sink_list_store;

            uint position = find_device_position (list_store, device);
            if (!should_show_device (device)) {
                if (position != uint.MAX) {
                    list_store.remove (position);
                    update_group_sensitivity (is_input, list_store);
                }
                return;
            }

            if (position == uint.MAX) {
                // Device not in list, add it
                list_store.append (device);
                position = list_store.get_n_items () - 1;
                update_group_sensitivity (is_input, list_store);
            }
            // If device already in list, don't remove/reinsert - properties are already
            // updated on the same object instance. This avoids triggering selection changes.

            // Change UI if device is default device
            unowned PulseDevice ? default_device = is_input
                ? this.default_source : this.default_sink;
            if (default_device == null ||
                device.get_current_hash_key () != default_device.get_current_hash_key ()) return;

            Gtk.ToggleButton toggle;
            Gtk.Scale slider;
            Adw.ComboRow device_row;
            if (is_input) {
                device_row = input_device_row;
                toggle = input_mute_toggle;
                slider = input_slider;
            } else {
                device_row = output_device_row;
                toggle = output_mute_toggle;
                slider = output_slider;
            }

            // Only update selection if it actually needs to change
            PulseDevice? current_selected = device_row.get_selected_item () as PulseDevice;
            if (current_selected == null ||
                current_selected.get_current_hash_key () != device.get_current_hash_key ()) {
                if (is_input) {
                    updating_input_selection = true;
                } else {
                    updating_output_selection = true;
                }
                device_row.set_selected (position);
                if (is_input) {
                    updating_input_selection = false;
                } else {
                    updating_output_selection = false;
                }
            }

            if (device.direction == PulseAudio.Direction.OUTPUT) {
                if (device.is_bluetooth && device.has_card) {
                    output_profile_row.set_visible (true);
                    set_device_profiles (device);
                } else {
                    output_profile_row.set_visible (false);
                    profile_list_store.remove_all ();
                }
            }
            // Set mute state
            toggle.set_active (device.is_muted);
            // Set volume
            slider.set_value (device.volume);

            // Set balance (output only)
            if (!is_input) {
                bool can_balance = device.channel_map.can_balance ();
                output_balance_row.set_visible (can_balance);
                if (can_balance) {
                    updating_balance = true;
                    output_balance_slider.set_value (device.balance);
                    updating_balance = false;
                }
            }
        }

        private void device_added (PulseDevice device) {
            bool is_input = device.direction == Direction.INPUT;
            GLib.ListStore list_store =
                is_input ? source_list_store : sink_list_store;

            if (!should_show_device (device)) {
                update_group_sensitivity (is_input, list_store);
                return;
            }

            if (find_device_position (list_store, device) == uint.MAX) {
                list_store.append (device);
            }
            update_group_sensitivity (is_input, list_store);
        }

        private void device_removed (PulseDevice device) {
            bool is_input = device.direction == Direction.INPUT;
            GLib.ListStore list_store =
                is_input ? source_list_store : sink_list_store;

            uint position = find_device_position (list_store, device);
            if (position == uint.MAX) return;
            list_store.remove (position);
            update_group_sensitivity (is_input, list_store);
        }

        /*
         * Default Sink/Source
         */

        private void default_device_changed (PulseDevice device) {
            if (device == null) return;
            switch (device.direction) {
                case Direction.INPUT:
                    this.default_source = device;
                    break;
                case Direction.OUTPUT:
                    this.default_sink = device;
                    break;
            }
        }

        /*
         * Active Sinks
         */

        /** Sorts by each device HashMap id */
        private int active_sinks_list_store_sort (Gtk.ListBoxRow a, Gtk.ListBoxRow b) {
            uint32 a_id = ((SinkInputRow) a).sink_input.index;
            uint32 b_id = ((SinkInputRow) b).sink_input.index;
            if (a_id == b_id) return 0;
            return a_id < b_id ? -1 : 1;
        }

        /** Updates the correct `SinkInputRow` with new information */
        private void active_sink_change (PulseSinkInput sink) {
            Functions.iter_listbox_children<Gtk.Widget> (levels_listbox, (row) => {
                if (row == null || !(row is SinkInputRow)) return false;
                var s = (SinkInputRow) row;
                if (s.sink_input.cmp (sink)) {
                    s.update (sink);
                    return true;
                }
                return false;
            });
        }

        /** Adds a new `SinkInputRow` */
        private void active_sink_added (PulseSinkInput sink) {
            levels_listbox.append (new SinkInputRow (sink, client));
            sink_inputs_group.set_sensitive (true);
        }

        /** Removes the correct `SinkInputRow` */
        private void active_sink_removed (PulseSinkInput sink) {
            int count = 0;
            Functions.iter_listbox_children<Gtk.Widget> (levels_listbox, (row) => {
                if (row == null || !(row is SinkInputRow)) return false;
                var s = (SinkInputRow) row;
                count++;
                if (s.sink_input.cmp (sink)) {
                    levels_listbox.remove (row);
                    return true;
                }
                return false;
            });
            if (count == 0) {
                sink_inputs_group.set_sensitive (false);
            }
        }

        /*
         * Speaker Test (Canberra)
         */

        private void init_canberra () {
            int result = Canberra.Context.create (out canberra_context);
            if (result != Canberra.SUCCESS) {
                warning ("Failed to create canberra context: %s",
                         Canberra.strerror (result));
                test_button.sensitive = false;
                test_left_button.sensitive = false;
                test_right_button.sensitive = false;
                return;
            }

            canberra_context.change_props (
                Canberra.PROP_APPLICATION_NAME, "Sway Settings",
                null);
        }

        private void play_test_sound (string channel) {
            if (canberra_context == null) {
                return;
            }

            // Cancel any currently playing test sound
            canberra_context.cancel (1);

            int result = canberra_context.play (
                1,  // id for cancellation
                Canberra.PROP_EVENT_ID, "audio-channel-" + channel,
                Canberra.PROP_EVENT_DESCRIPTION, "Testing %s speaker".printf (channel),
                Canberra.PROP_CANBERRA_FORCE_CHANNEL, channel,
                Canberra.PROP_MEDIA_ROLE, "test",
                null);

            if (result != Canberra.SUCCESS) {
                warning ("Failed to play test sound: %s", Canberra.strerror (result));
            }
        }
    }
}
