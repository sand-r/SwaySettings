namespace SwaySettings {
    public class BluetoothPage : Page {

        private const string NEARBY_EMPTY_TEXT = "No nearby devices";
        private const string PAIRED_EMPTY_TEXT = "No paired devices";

        Gtk.Stack stack;
        Adw.StatusPage status_page;
        Gtk.ScrolledWindow scrolled_window;

        Adw.SwitchRow status_row;
        bool pending_status_switch = false;

        Gtk.Spinner discovering_spinner;

        Adw.PreferencesGroup paired_group;
        Gtk.ListBox paired_list_box;

        Adw.PreferencesGroup nearby_group;
        Gtk.ListBox nearby_list_box;

        Bluez.Daemon daemon;

        public BluetoothPage (SettingsItem item, Adw.NavigationPage page) {
            base (item, page);
        }

        public override async void on_back (Adw.NavigationPage page) {
            this.freeze_notify ();
            if (this.daemon.powered || this.daemon.discovering) {
                yield this.daemon.set_discovering_state (false);
            }
            yield this.daemon.unregister_agent ();
        }

        public override void on_refresh () {
            // Init Bluetooth Daemon
            this.daemon = new Bluez.Daemon ();

            // Main stack for switching between content and error states
            stack = new Gtk.Stack () {
                transition_type = Gtk.StackTransitionType.CROSSFADE,
                vhomogeneous = false,
                vexpand = true,
            };
            set_child (stack);

            // Main content area
            var content_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 24) {
                valign = Gtk.Align.START,
            };
            this.scrolled_window = get_scroll_widget (content_box);
            stack.add_named (this.scrolled_window, "content");

            // Bluetooth toggle group
            var toggle_group = new Adw.PreferencesGroup ();
            content_box.append (toggle_group);

            this.status_row = new Adw.SwitchRow () {
                title = "Bluetooth",
            };
            toggle_group.add (this.status_row);

            // Paired devices group
            paired_group = new Adw.PreferencesGroup () {
                title = "Paired Devices",
            };
            content_box.append (paired_group);

            paired_list_box = new Gtk.ListBox () {
                valign = Gtk.Align.START,
                selection_mode = Gtk.SelectionMode.NONE,
            };
            paired_list_box.add_css_class ("boxed-list");
            paired_list_box.set_sort_func ((Gtk.ListBoxSortFunc) this.list_box_sort_func);
            paired_list_box.set_placeholder (create_placeholder (PAIRED_EMPTY_TEXT));
            paired_group.add (paired_list_box);

            // Nearby devices group with spinner in header
            nearby_group = new Adw.PreferencesGroup () {
                title = "Nearby Devices",
            };
            content_box.append (nearby_group);

            this.discovering_spinner = new Gtk.Spinner () {
                valign = Gtk.Align.CENTER,
            };
            nearby_group.set_header_suffix (this.discovering_spinner);

            nearby_list_box = new Gtk.ListBox () {
                valign = Gtk.Align.START,
                selection_mode = Gtk.SelectionMode.NONE,
            };
            nearby_list_box.add_css_class ("boxed-list");
            nearby_list_box.set_sort_func ((Gtk.ListBoxSortFunc) this.list_box_sort_func);
            nearby_list_box.set_placeholder (create_placeholder (NEARBY_EMPTY_TEXT));
            nearby_group.add (nearby_list_box);

            // Status page for errors/disabled state
            status_page = new Adw.StatusPage () {
                icon_name = "bluetooth-symbolic",
                vexpand = true,
            };
            stack.add_named (status_page, "status");

            remove_signals ();
            add_signals ();

            // Bind discoverable to subtitle
            this.daemon.bind_property ("discoverable",
                                       status_row, "subtitle",
                                       BindingFlags.SYNC_CREATE,
                                       (bind, from_value, ref to_value) => {
                to_value = "";
                if (!from_value.holds (Type.BOOLEAN)) return false;
                string ? name = this.daemon.get_alias ();
                if (!from_value.get_boolean () || name == null) return true;
                to_value = "Discoverable as \"%s\"".printf (name);
                return true;
            });

            this.daemon.start ();

            this.powered_state_change_cb ();
        }

        Gtk.Widget create_placeholder (string text) {
            var placeholder = new Adw.StatusPage () {
                icon_name = "bluetooth-symbolic",
                title = text,
            };
            placeholder.add_css_class ("compact");
            return placeholder;
        }

        [CCode (instance_pos = -1)]
        int list_box_sort_func (BluetoothDeviceRow a, BluetoothDeviceRow b) {
            Bluez.Device1 a_device = a.device;
            Bluez.Device1 b_device = b.device;

            int16 a_range = a.device.rssi;
            int16 b_range = b.device.rssi;
            if (a_range == b_range) {
                string a_name = a_device.name ?? a_device.address;
                string b_name = b_device.name ?? b_device.address;
                return a_name.collate (b_name);
            }
            // Rows with RSSI values of 0 should be on the bottom
            if (a_range == 0) return 1;
            if (b_range == 0) return -1;
            // Shortest range on top
            return a_range < b_range ? 1 : -1;
        }

        void add_signals () {
            this.daemon.adapter_added.connect_after (this.adapter_added_cb);
            this.daemon.adapter_removed.connect_after (this.adapter_removed_cb);
            this.daemon.device_added.connect_after (this.device_added_cb);
            this.daemon.device_removed.connect_after (this.device_removed_cb);

            this.daemon.bluetooth_bus_state_change.connect (this.bus_state_change_cb);
            this.daemon.notify["powered"].connect (this.powered_state_change_cb);
            this.daemon.notify["rfkill-blocking"].connect (this.powered_state_change_cb);
            this.daemon.notify["discovering"].connect (this.discovering_cb);
            this.status_row.notify["active"].connect (this.status_switch_cb);
        }

        void remove_signals () {
            this.daemon.adapter_added.disconnect (this.adapter_added_cb);
            this.daemon.adapter_removed.disconnect (this.adapter_removed_cb);
            this.daemon.device_added.disconnect (this.device_added_cb);
            this.daemon.device_removed.disconnect (this.device_removed_cb);

            this.daemon.bluetooth_bus_state_change.disconnect (this.bus_state_change_cb);
            this.daemon.notify["powered"].disconnect (this.powered_state_change_cb);
            this.daemon.notify["rfkill-blocking"].disconnect (this.powered_state_change_cb);
            this.daemon.notify["discovering"].disconnect (this.discovering_cb);
            this.status_row.notify["active"].disconnect (this.status_switch_cb);
        }

        void adapter_added_cb (Bluez.Adapter1 adapter) {
            powered_state_change_cb ();
        }

        void adapter_removed_cb (Bluez.Adapter1 adapter) {
            var adapters = this.daemon.get_adapters ();
            if (adapters.is_empty ()) {
                status_page.title = "No Bluetooth Adapters";
                status_page.description = "Connect a Bluetooth adapter to use Bluetooth";
                stack.set_visible_child_name ("status");
            }
        }

        void device_added_cb (Bluez.Device1 device) {
            unowned Gtk.ListBox list_box = device.paired ? paired_list_box : nearby_list_box;

            var adapter = this.daemon.get_adapter (device.adapter);
            var _device = this.daemon.get_device (
                ((DBusProxy) device).get_object_path ());
            var row = new BluetoothDeviceRow (_device, adapter);
            // Watch property changes
            row.on_update.connect (this.device_changed_cb);
            list_box.append (row);
        }

        void device_removed_cb (Bluez.Device1 device) {
            BoolFunc<Gtk.Widget> func = (widget) => {
                if (widget is BluetoothDeviceRow) {
                    BluetoothDeviceRow row = (BluetoothDeviceRow) widget;
                    if (row.device == device) {
                        // Remove watch property changes
                        row.before_destroy ();
                        row.destroy ();
                        return true;
                    }
                }
                return false;
            };
            Functions.iter_listbox_children (paired_list_box, func);
            Functions.iter_listbox_children (nearby_list_box, func);
        }

        void device_changed_cb (BluetoothDeviceRow row) {
            if (!(row.parent is Gtk.ListBox)) {
                return;
            }
            Gtk.ListBox parent = (Gtk.ListBox) row.parent;
            bool paired = row.device.paired;

            // Move the Row to the correct LisBox
            // Moving the Row causes a few GTK critical warnings, so
            // creating a new row is the only option...
            unowned Gtk.ListBox ? remove_list_box = null;
            unowned Gtk.ListBox ? add_list_box = null;
            if (paired && parent != paired_list_box) {
                add_list_box = paired_list_box;
                remove_list_box = nearby_list_box;
            }
            if (!paired && parent != nearby_list_box) {
                add_list_box = nearby_list_box;
                remove_list_box = paired_list_box;
            }

            if (remove_list_box != null && add_list_box != null) {
                var adapter = this.daemon.get_adapter (row.device.adapter);
                var device = new BluetoothDeviceRow (row.device, adapter);
                add_list_box.append (device);
                row.before_destroy ();
                row.destroy ();
            }
        }

        /**
         * Called when ever the status switch is clicked.
         * Waits until the powered state changes.
         */
        void status_switch_cb () {
            bool state = this.status_row.active;
            pending_status_switch = true;
            this.status_row.sensitive = false;
            this.status_row.notify["active"].disconnect (this.status_switch_cb);
            this.daemon.change_bluetooth_state.begin (state, () => {
                pending_status_switch = false;
                this.status_row.sensitive = true;
                this.status_row.notify["active"].connect (this.status_switch_cb);
            });
        }

        void bus_state_change_cb (bool state) {
            if (!state) {
                status_page.title = "Bluetooth Service Unavailable";
                status_page.description = "The Bluetooth service is not running";
                stack.set_visible_child_name ("status");
            } else {
                this.powered_state_change_cb ();
                this.daemon.register_agent.begin (
                    (Gtk.Window) this.get_root ());
            }
        }

        void discovering_cb () {
            bool discovering = this.daemon.discovering;
            if (discovering) {
                this.discovering_spinner.start ();
            } else {
                this.discovering_spinner.stop ();
            }
        }

        void powered_state_change_cb () {
            bool powered = this.daemon.powered;
            bool blocking = this.daemon.rfkill_blocking;
            this.status_row.notify["active"].disconnect (this.status_switch_cb);
            if (!blocking && powered) {
                stack.set_visible_child_name ("content");
                if (!pending_status_switch) this.status_row.active = true;
            } else {
                stack.set_visible_child_name ("content");
                if (!pending_status_switch) this.status_row.active = false;
            }
            this.status_row.notify["active"].connect (this.status_switch_cb);
        }
    }
}
