namespace SwaySettings {
    /**
     * WirePlumber-based audio daemon for native PipeWire support.
     */
    public class WpAudioDaemon : Object {
        private Wp.Core? core = null;
        private Wp.ObjectManager? nodes_om = null;
        private Wp.ObjectManager? metadata_om = null;
        private Wp.Plugin? mixer_api = null;
        private Wp.Metadata? default_metadata = null;

        private int pending_plugins = 0;
        private string? default_sink_name = null;
        private string? default_source_name = null;

        public Gee.HashMap<uint32, WpAudioDevice> sinks { get; private set; }
        public Gee.HashMap<uint32, WpAudioDevice> sources { get; private set; }

        public bool ready { get; private set; default = false; }

        public signal void on_ready ();
        public signal void device_added (WpAudioDevice device);
        public signal void device_changed (WpAudioDevice device);
        public signal void device_removed (WpAudioDevice device);
        public signal void default_sink_changed (WpAudioDevice? device);
        public signal void default_source_changed (WpAudioDevice? device);

        private static bool wp_initialized = false;

        construct {
            sinks = new Gee.HashMap<uint32, WpAudioDevice> ();
            sources = new Gee.HashMap<uint32, WpAudioDevice> ();

            // Only initialize WirePlumber once per process
            if (!wp_initialized) {
                Wp.init (Wp.InitFlags.ALL);
                wp_initialized = true;
            }
        }

        public void start () {
            core = new Wp.Core (null, null, null);

            if (!core.connect ()) {
                warning ("Failed to connect to PipeWire");
                return;
            }

            // Load mixer-api module
            pending_plugins = 1;
            core.load_component ("libwireplumber-module-mixer-api",
                "module", null, null, null, on_plugin_loaded);
        }

        public void stop () {
            if (core != null) {
                core.disconnect ();
                core = null;
            }
            nodes_om = null;
            metadata_om = null;
            mixer_api = null;
            default_metadata = null;
            sinks.clear ();
            sources.clear ();
            ready = false;
        }

        private void on_plugin_loaded (GLib.Object? source, GLib.AsyncResult res) {
            try {
                core.load_component_finish (res);
            } catch (Error e) {
                warning ("Failed to load mixer-api: %s", e.message);
                return;
            }

            pending_plugins--;
            if (pending_plugins == 0) {
                setup_after_plugins ();
            }
        }

        private void setup_after_plugins () {
            mixer_api = Wp.Plugin.find (core, "mixer-api");
            if (mixer_api == null) {
                warning ("mixer-api plugin not found");
                return;
            }

            // Use cubic scale (matches GNOME/pavucontrol)
            mixer_api.set ("scale", 1);

            setup_nodes_om ();
            setup_metadata_om ();
        }

        private void setup_nodes_om () {
            nodes_om = new Wp.ObjectManager ();

            // Audio sinks
            var sink_interest = new Wp.ObjectInterest (typeof (Wp.Node));
            sink_interest.add_constraint (
                Wp.ConstraintType.PW_GLOBAL_PROPERTY,
                "media.class",
                Wp.ConstraintVerb.MATCHES,
                new Variant.string ("Audio/Sink*")
            );
            nodes_om.add_interest_full ((owned) sink_interest);

            // Audio sources
            var source_interest = new Wp.ObjectInterest (typeof (Wp.Node));
            source_interest.add_constraint (
                Wp.ConstraintType.PW_GLOBAL_PROPERTY,
                "media.class",
                Wp.ConstraintVerb.MATCHES,
                new Variant.string ("Audio/Source*")
            );
            nodes_om.add_interest_full ((owned) source_interest);

            nodes_om.request_object_features (
                typeof (Wp.Node),
                (Wp.ObjectFeatures) Wp.ProxyFeatures.PIPEWIRE_OBJECT_FEATURES_MINIMAL
            );

            nodes_om.installed.connect (() => {
                ready = true;
                on_ready ();
            });
            nodes_om.object_added.connect (on_node_added);
            nodes_om.object_removed.connect (on_node_removed);

            core.install_object_manager (nodes_om);
        }

        private void setup_metadata_om () {
            metadata_om = new Wp.ObjectManager ();

            var interest = new Wp.ObjectInterest (typeof (Wp.Metadata));
            interest.add_constraint (
                Wp.ConstraintType.PW_GLOBAL_PROPERTY,
                "metadata.name",
                Wp.ConstraintVerb.EQUALS,
                new Variant.string ("default")
            );
            metadata_om.add_interest_full ((owned) interest);
            metadata_om.request_object_features (
                typeof (Wp.Metadata),
                (Wp.ObjectFeatures) Wp.MetadataFeatures.DATA
            );

            metadata_om.object_added.connect ((obj) => {
                var metadata = obj as Wp.Metadata;
                if (metadata == null) return;
                default_metadata = metadata;
                metadata.changed.connect (on_metadata_changed);

                // Read initial defaults
                update_default_sink ();
                update_default_source ();
            });

            core.install_object_manager (metadata_om);
        }

        private void on_node_added (GLib.Object obj) {
            var node = obj as Wp.Node;
            if (node == null) return;

            var device = WpAudioDevice.from_node (node);
            if (device == null) return;

            // Get initial volume/mute
            update_device_volume (device);

            if (device.is_sink) {
                sinks[device.id] = device;
                device.is_default = (device.name == default_sink_name);
            } else if (device.is_source) {
                sources[device.id] = device;
                device.is_default = (device.name == default_source_name);
            }

            device_added (device);
        }

        private void on_node_removed (GLib.Object obj) {
            var node = obj as Wp.Node;
            if (node == null) return;

            var id = node.get_bound_id ();

            if (sinks.has_key (id)) {
                var device = sinks[id];
                sinks.unset (id);
                device_removed (device);
            } else if (sources.has_key (id)) {
                var device = sources[id];
                sources.unset (id);
                device_removed (device);
            }
        }

        private void on_metadata_changed (uint32 subject, string key,
                                          string type, string value) {
            if (key == "default.audio.sink") {
                update_default_sink ();
            } else if (key == "default.audio.source") {
                update_default_source ();
            }
        }

        private void update_default_sink () {
            if (default_metadata == null) return;

            var value = default_metadata.find (0, "default.audio.sink", null);
            var new_name = value != null ? parse_json_name (value) : null;

            if (new_name != default_sink_name) {
                default_sink_name = new_name;

                WpAudioDevice? new_default = null;
                foreach (var device in sinks.values) {
                    bool was_default = device.is_default;
                    device.is_default = (device.name == default_sink_name);
                    if (device.is_default) {
                        new_default = device;
                    }
                    if (was_default != device.is_default) {
                        device_changed (device);
                    }
                }
                default_sink_changed (new_default);
            }
        }

        private void update_default_source () {
            if (default_metadata == null) return;

            var value = default_metadata.find (0, "default.audio.source", null);
            var new_name = value != null ? parse_json_name (value) : null;

            if (new_name != default_source_name) {
                default_source_name = new_name;

                WpAudioDevice? new_default = null;
                foreach (var device in sources.values) {
                    bool was_default = device.is_default;
                    device.is_default = (device.name == default_source_name);
                    if (device.is_default) {
                        new_default = device;
                    }
                    if (was_default != device.is_default) {
                        device_changed (device);
                    }
                }
                default_source_changed (new_default);
            }
        }

        private void update_device_volume (WpAudioDevice device) {
            if (mixer_api == null) return;

            Variant? variant = null;
            Signal.emit_by_name (mixer_api, "get-volume", device.id, out variant);

            if (variant != null) {
                double vol = 1.0;
                bool mute = false;
                variant.lookup ("volume", "d", out vol);
                variant.lookup ("mute", "b", out mute);
                device.volume = vol;
                device.is_muted = mute;
            }
        }

        public void set_volume (WpAudioDevice device, double volume) {
            if (mixer_api == null || device == null) return;

            var builder = new VariantBuilder (VariantType.VARDICT);
            builder.add ("{sv}", "volume", new Variant.double (volume));

            bool result = false;
            Signal.emit_by_name (mixer_api, "set-volume", device.id, builder.end (), out result);

            if (result) {
                device.volume = volume;
            }
        }

        public void set_mute (WpAudioDevice device, bool mute) {
            if (mixer_api == null || device == null) return;

            var builder = new VariantBuilder (VariantType.VARDICT);
            builder.add ("{sv}", "mute", new Variant.boolean (mute));

            bool result = false;
            Signal.emit_by_name (mixer_api, "set-volume", device.id, builder.end (), out result);

            if (result) {
                device.is_muted = mute;
            }
        }

        public void set_default_sink (WpAudioDevice device) {
            if (default_metadata == null || device == null) return;
            var json = """{"name":"%s"}""".printf (device.name);
            default_metadata.set (0, "default.audio.sink", "Spa:String:JSON", json);
        }

        public void set_default_source (WpAudioDevice device) {
            if (default_metadata == null || device == null) return;
            var json = """{"name":"%s"}""".printf (device.name);
            default_metadata.set (0, "default.audio.source", "Spa:String:JSON", json);
        }

        public WpAudioDevice? get_default_sink () {
            foreach (var device in sinks.values) {
                if (device.is_default) return device;
            }
            return null;
        }

        public WpAudioDevice? get_default_source () {
            foreach (var device in sources.values) {
                if (device.is_default) return device;
            }
            return null;
        }

        private string? parse_json_name (string json) {
            try {
                var parser = new Json.Parser ();
                parser.load_from_data (json);
                var obj = parser.get_root ()?.get_object ();
                return obj?.get_string_member_with_default ("name", "");
            } catch {
                return null;
            }
        }
    }
}
