using Gee;

namespace SwaySettings {
    public class TrackpadPage : PageScroll, IIpcPage {
        public IPC ipc { get; set; }
        private TrackpadContent content;

        public TrackpadPage (SettingsItem item,
                             Adw.NavigationPage page,
                             IPC ipc) {
            base (item, page);
            this.ipc = ipc;
        }

        public override Gtk.Widget set_child () {
            content = new TrackpadContent (ipc);
            var device = get_input_device (InputTypes.TOUCHPAD);
            content.load_device (device);
            return content;
        }

        private InputDevice? get_input_device (InputTypes input_type) {
            Json.Node ipc_output = ipc.get_reply (SwayCommands.GET_INPUTS);
            if (ipc_output.get_node_type () == Json.NodeType.ARRAY) {
                foreach (var node in ipc_output.get_array ().get_elements ()) {
                    if (node.get_node_type () != Json.NodeType.OBJECT) continue;
                    unowned Json.Object? obj = node.get_object ();
                    if (obj == null) continue;

                    InputTypes type = InputTypes.parse_string (
                        obj.get_string_member ("type") ?? "");
                    if (input_type != type || type == InputTypes.NEITHER) {
                        continue;
                    }

                    // Skip devices without accel_speed (e.g., keyboards reporting as mouse)
                    if (obj.get_member ("libinput")
                        ?.get_object ()
                        ?.has_member ("accel_speed") == false) {
                        continue;
                    }

                    return get_device_settings (obj, type);
                }
            }
            return null;
        }

        private InputDevice get_device_settings (Json.Object obj, InputTypes type) {
            var device = new InputDevice (
                obj.get_string_member ("identifier"), type);

            unowned Json.Node? lib = obj.get_member ("libinput");
            if (lib != null && lib.get_node_type () == Json.NodeType.OBJECT) {
                device.data = (InputData) Json.gobject_deserialize (
                    typeof (InputData), lib);

                // Get scroll factor
                unowned Json.Node? scroll_node = obj.get_member ("scroll_factor");
                if (scroll_node != null && scroll_node.get_value_type () == Type.DOUBLE) {
                    device.scroll_factor = scroll_node.get_double ();
                }
            }

            return device;
        }
    }
}
