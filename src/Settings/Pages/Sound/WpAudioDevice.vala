namespace SwaySettings {
    /**
     * Represents an audio device (sink or source) from WirePlumber.
     */
    public class WpAudioDevice : Object {
        public uint32 id { get; set; }
        public string name { get; set; }
        public string description { get; set; }
        public string media_class { get; set; }
        public double volume { get; set; default = 1.0; }
        public bool is_muted { get; set; default = false; }
        public bool is_default { get; set; default = false; }
        public bool is_available { get; set; default = true; }

        public bool is_sink {
            get { return media_class != null && media_class.has_prefix ("Audio/Sink"); }
        }

        public bool is_source {
            get { return media_class != null && media_class.has_prefix ("Audio/Source"); }
        }

        public WpAudioDevice (uint32 id, string name, string description, string media_class, bool available = true) {
            this.id = id;
            this.name = name;
            this.description = description;
            this.media_class = media_class;
            this.is_available = available;
        }

        public static WpAudioDevice? from_node (Wp.Node node) {
            var pw_obj = node as Wp.PipewireObject;
            if (pw_obj == null) return null;

            var id = node.get_bound_id ();
            var name = pw_obj.get_property ("node.name") ?? "";
            var media_class = pw_obj.get_property ("media.class") ?? "";

            // Get nick (specific port name like "Speaker", "Headphones", "HDMI 1")
            var nick = pw_obj.get_property ("node.nick");
            var card_desc = pw_obj.get_property ("node.description");

            // Use nick if available, otherwise card description, otherwise node name
            string desc;
            if (nick != null && nick.length > 0) {
                desc = nick;
            } else if (card_desc != null && card_desc.length > 0) {
                desc = card_desc;
            } else {
                desc = name;
            }

            // Check availability based on node properties
            bool available = check_availability (pw_obj, name, media_class);

            return new WpAudioDevice (id, name, desc, media_class, available);
        }

        private static bool check_availability (Wp.PipewireObject pw_obj, string name, string media_class) {
            string lower_name = name.down ();
            var nick = pw_obj.get_property ("node.nick") ?? "";
            string lower_nick = nick.down ();

            // Filter monitor sources (they mirror sinks, not real inputs)
            if (lower_name.contains (".monitor") || media_class.contains ("Monitor")) {
                return false;
            }

            // Check port.available property (works for jack detection)
            var available_prop = pw_obj.get_property ("port.available");
            if (available_prop != null && available_prop == "no") {
                return false;
            }

            // Check card.profile.available
            var profile_avail = pw_obj.get_property ("card.profile.available");
            if (profile_avail != null && profile_avail == "no") {
                return false;
            }

            // For HDMI/DP without explicit availability, filter by default
            if (lower_name.contains ("hdmi") || lower_name.contains ("displayport")) {
                // Only show if explicitly available
                if (available_prop == null || available_prop != "yes") {
                    return false;
                }
            }

            return true;
        }

        /**
         * Basic availability check (monitors, etc.) - for external use
         */
        public static bool basic_availability_check (string name, string media_class) {
            string lower_name = name.down ();

            // Filter monitor sources
            if (lower_name.contains (".monitor") || media_class.contains ("Monitor")) {
                return false;
            }

            return true;
        }
    }
}
