/* WirePlumber Vala Bindings
 * Copyright (c) 2024
 * SPDX-License-Identifier: MIT
 *
 * Minimal bindings for WirePlumber audio management
 */

[CCode (cheader_filename = "wp/wp.h")]
namespace Wp {
    [CCode (cname = "WpInitFlags", cprefix = "WP_INIT_", has_type_id = false)]
    [Flags]
    public enum InitFlags {
        PIPEWIRE,
        SPA_TYPES,
        SET_PW_LOG,
        SET_GLIB_LOG,
        ALL
    }

    [CCode (cname = "wp_init")]
    public static void init (InitFlags flags);

    [CCode (cname = "wp_get_library_version")]
    public static unowned string get_library_version ();

    [CCode (cname = "wp_get_library_api_version")]
    public static unowned string get_library_api_version ();

    /* WpObjectFeatures */
    [CCode (cname = "WpObjectFeatures", has_type_id = false)]
    public struct ObjectFeatures : uint {}

    [CCode (cname = "WP_OBJECT_FEATURES_ALL")]
    public const ObjectFeatures OBJECT_FEATURES_ALL;

    /* WpCoreFeatures */
    [CCode (cname = "WpCoreFeatures", cprefix = "WP_CORE_FEATURE_", has_type_id = false)]
    [Flags]
    public enum CoreFeatures {
        CONNECTED,
        COMPONENTS
    }

    /* WpProxyFeatures */
    [CCode (cname = "WpProxyFeatures", cprefix = "WP_", has_type_id = false)]
    [Flags]
    public enum ProxyFeatures {
        PROXY_FEATURE_BOUND,
        PIPEWIRE_OBJECT_FEATURE_INFO,
        PIPEWIRE_OBJECT_FEATURE_PARAM_PROPS,
        PIPEWIRE_OBJECT_FEATURE_PARAM_FORMAT,
        PIPEWIRE_OBJECT_FEATURE_PARAM_PROFILE,
        PIPEWIRE_OBJECT_FEATURE_PARAM_PORT_CONFIG,
        PIPEWIRE_OBJECT_FEATURE_PARAM_ROUTE,
        PIPEWIRE_OBJECT_FEATURES_MINIMAL,
        PIPEWIRE_OBJECT_FEATURES_ALL
    }

    /* WpMetadataFeatures */
    [CCode (cname = "WpMetadataFeatures", cprefix = "WP_METADATA_FEATURE_", has_type_id = false)]
    [Flags]
    public enum MetadataFeatures {
        DATA
    }

    /* WpObject */
    [CCode (cname = "WpObject", type_id = "wp_object_get_type ()")]
    public abstract class Object : GLib.Object {
        [CCode (cname = "wp_object_get_id")]
        public uint get_id ();

        [CCode (cname = "wp_object_get_core")]
        public unowned Core? get_core ();

        [CCode (cname = "wp_object_get_active_features")]
        public ObjectFeatures get_active_features ();

        [CCode (cname = "wp_object_get_supported_features")]
        public ObjectFeatures get_supported_features ();

        [CCode (cname = "wp_object_activate")]
        public void activate (ObjectFeatures features, GLib.Cancellable? cancellable, GLib.AsyncReadyCallback? callback);

        [CCode (cname = "wp_object_activate_finish")]
        public bool activate_finish (GLib.AsyncResult res) throws GLib.Error;
    }

    /* WpProxy */
    [CCode (cname = "WpProxy", type_id = "wp_proxy_get_type ()")]
    public abstract class Proxy : Object {
        [CCode (cname = "wp_proxy_get_bound_id")]
        public uint32 get_bound_id ();

        public signal void bound (uint32 id);
        public signal void error (int seq, int res, string message);
    }

    /* WpGlobalProxy */
    [CCode (cname = "WpGlobalProxy", type_id = "wp_global_proxy_get_type ()")]
    public abstract class GlobalProxy : Proxy {
        [CCode (cname = "wp_global_proxy_get_permissions")]
        public uint32 get_permissions ();

        [CCode (cname = "wp_global_proxy_bind")]
        public bool bind ();
    }

    /* WpIterator */
    [CCode (cname = "WpIterator", type_id = "wp_iterator_get_type ()", free_function = "wp_iterator_unref")]
    [Compact]
    public class Iterator {
        [CCode (cname = "wp_iterator_reset")]
        public void reset ();

        [CCode (cname = "wp_iterator_next")]
        public bool next (out GLib.Value item);
    }

    /* WpMetadataItem */
    [CCode (cname = "WpMetadataItem", ref_function = "wp_metadata_item_ref", unref_function = "wp_metadata_item_unref")]
    [Compact]
    public class MetadataItem {
        [CCode (cname = "wp_metadata_item_get_subject")]
        public uint32 get_subject ();

        [CCode (cname = "wp_metadata_item_get_key")]
        public unowned string get_key ();

        [CCode (cname = "wp_metadata_item_get_value_type")]
        public unowned string get_value_type ();

        [CCode (cname = "wp_metadata_item_get_value")]
        public unowned string get_value ();
    }

    /* WpMetadata */
    [CCode (cname = "WpMetadata", type_id = "wp_metadata_get_type ()")]
    public class Metadata : GlobalProxy {
        [CCode (cname = "wp_metadata_new_iterator")]
        public Iterator new_iterator (uint32 subject);

        [CCode (cname = "wp_metadata_find")]
        public unowned string? find (uint32 subject, string key, out unowned string? type = null);

        [CCode (cname = "wp_metadata_set")]
        public void set (uint32 subject, string key, string? type, string? value);

        [CCode (cname = "wp_metadata_clear")]
        public void clear ();

        public signal void changed (uint32 subject, string key, string type, string value);
    }

    /* WpNode */
    [CCode (cname = "WpNodeState", cprefix = "WP_NODE_STATE_", has_type_id = false)]
    public enum NodeState {
        ERROR,
        CREATING,
        SUSPENDED,
        IDLE,
        RUNNING
    }

    [CCode (cname = "WpNode", type_id = "wp_node_get_type ()")]
    public class Node : GlobalProxy {
        [CCode (cname = "wp_node_get_state")]
        public NodeState get_state (out unowned string? error = null);

        [CCode (cname = "wp_node_get_n_input_ports")]
        public uint get_n_input_ports (out uint? max = null);

        [CCode (cname = "wp_node_get_n_output_ports")]
        public uint get_n_output_ports (out uint? max = null);

        [CCode (cname = "wp_node_send_command")]
        public void send_command (string command);
    }

    /* WpDevice */
    [CCode (cname = "WpDevice", type_id = "wp_device_get_type ()")]
    public class Device : GlobalProxy {
    }

    /* WpSpaPod - for reading params */
    [CCode (cname = "WpSpaPod", ref_function = "wp_spa_pod_ref", unref_function = "wp_spa_pod_unref")]
    [Compact]
    public class SpaPod {
        [CCode (cname = "wp_spa_pod_get_spa_type")]
        public uint32 get_spa_type ();

        [CCode (cname = "wp_spa_pod_is_object")]
        public bool is_object ();

        [CCode (cname = "wp_spa_pod_is_struct")]
        public bool is_struct ();

        [CCode (cname = "wp_spa_pod_get_object")]
        public bool get_object (out uint32 id, ...);

        [CCode (cname = "wp_spa_pod_new_iterator")]
        public Iterator new_iterator ();
    }

    /* WpSpaPodParser - for parsing pod contents */
    [CCode (cname = "WpSpaPodParser", free_function = "wp_spa_pod_parser_unref")]
    [Compact]
    public class SpaPodParser {
        [CCode (cname = "wp_spa_pod_parser_new_object")]
        public SpaPodParser.object (SpaPod pod, out uint32 id);

        [CCode (cname = "wp_spa_pod_parser_get")]
        public bool get (...);

        [CCode (cname = "wp_spa_pod_parser_get_boolean")]
        public bool get_boolean (out bool value);

        [CCode (cname = "wp_spa_pod_parser_get_int")]
        public bool get_int (out int value);

        [CCode (cname = "wp_spa_pod_parser_get_string")]
        public bool get_string (out unowned string value);
    }

    /* Param types */
    [CCode (cname = "WP_SPA_TYPE_OBJECT_ParamRoute")]
    public const uint32 SPA_TYPE_OBJECT_PARAM_ROUTE;

    /* WpPipewireObject interface */
    [CCode (cname = "WpPipewireObject", type_id = "wp_pipewire_object_get_type ()")]
    public interface PipewireObject : GLib.Object {
        [CCode (cname = "wp_pipewire_object_get_native_info")]
        public void* get_native_info ();

        [CCode (cname = "wp_pipewire_object_get_properties")]
        public Properties? get_properties ();

        [CCode (cname = "wp_pipewire_object_get_property")]
        public unowned string? get_property (string key);

        [CCode (cname = "wp_pipewire_object_enum_params")]
        public void enum_params (string params, GLib.Cancellable? cancellable, GLib.AsyncReadyCallback? callback);

        [CCode (cname = "wp_pipewire_object_enum_params_finish")]
        public Iterator? enum_params_finish (GLib.AsyncResult res) throws GLib.Error;

        [CCode (cname = "wp_pipewire_object_enum_params_sync")]
        public Iterator? enum_params_sync (string params, GLib.Cancellable? cancellable);

        public signal void params_changed (string params);
    }

    /* WpProperties */
    [CCode (cname = "WpProperties", ref_function = "wp_properties_ref", unref_function = "wp_properties_unref")]
    [Compact]
    public class Properties {
        [CCode (cname = "wp_properties_new_empty")]
        public Properties ();

        [CCode (cname = "wp_properties_get")]
        public unowned string? get (string key);

        [CCode (cname = "wp_properties_set")]
        public int set (string key, string? value);

        [CCode (cname = "wp_properties_get_count")]
        public uint get_count ();

        [CCode (cname = "wp_properties_new_iterator")]
        public Iterator new_iterator ();
    }

    /* WpObjectInterest */
    [CCode (cname = "WpConstraintType", cprefix = "WP_CONSTRAINT_TYPE_", has_type_id = false)]
    public enum ConstraintType {
        NONE,
        PW_GLOBAL_PROPERTY,
        PW_PROPERTY,
        G_PROPERTY
    }

    [CCode (cname = "WpConstraintVerb", cprefix = "WP_CONSTRAINT_VERB_", has_type_id = false)]
    public enum ConstraintVerb {
        EQUALS,
        NOT_EQUALS,
        IN_LIST,
        IN_RANGE,
        MATCHES
    }

    [CCode (cname = "WpObjectInterest", ref_function = "wp_object_interest_ref", unref_function = "wp_object_interest_unref")]
    [Compact]
    public class ObjectInterest {
        [CCode (cname = "wp_object_interest_new_type")]
        public ObjectInterest (GLib.Type gtype);

        [CCode (cname = "wp_object_interest_add_constraint")]
        public void add_constraint (ConstraintType type, string subject, ConstraintVerb verb, GLib.Variant? value);
    }

    /* WpObjectManager */
    [CCode (cname = "WpObjectManager", type_id = "wp_object_manager_get_type ()")]
    public class ObjectManager : GLib.Object {
        [CCode (cname = "wp_object_manager_new")]
        public ObjectManager ();

        [CCode (cname = "wp_object_manager_is_installed")]
        public bool is_installed ();

        [CCode (cname = "wp_object_manager_add_interest_full")]
        public void add_interest_full (owned ObjectInterest interest);

        [CCode (cname = "wp_object_manager_request_object_features")]
        public void request_object_features (GLib.Type object_type, ObjectFeatures wanted_features);

        [CCode (cname = "wp_object_manager_get_n_objects")]
        public uint get_n_objects ();

        [CCode (cname = "wp_object_manager_new_iterator")]
        public Iterator new_iterator ();

        [CCode (cname = "wp_object_manager_lookup_full")]
        public GLib.Object? lookup_full (ObjectInterest interest);

        public signal void installed ();
        public signal void object_added (GLib.Object object);
        public signal void object_removed (GLib.Object object);
        public signal void objects_changed ();
    }

    /* WpPlugin */
    [CCode (cname = "WpPluginFeatures", cprefix = "WP_PLUGIN_FEATURE_", has_type_id = false)]
    [Flags]
    public enum PluginFeatures {
        ENABLED
    }

    [CCode (cname = "WpPlugin", type_id = "wp_plugin_get_type ()")]
    public class Plugin : Object {
        [CCode (cname = "wp_plugin_find")]
        public static Plugin? find (Core core, string plugin_name);

        [CCode (cname = "wp_plugin_get_name")]
        public unowned string get_name ();
    }

    /* WpCore */
    [CCode (cname = "WpCore", type_id = "wp_core_get_type ()")]
    public class Core : Object {
        [CCode (cname = "wp_core_new")]
        public Core (GLib.MainContext? context, void* conf = null, Properties? properties = null);

        [CCode (cname = "wp_core_clone")]
        public Core clone ();

        [CCode (cname = "wp_core_get_g_main_context")]
        public unowned GLib.MainContext? get_g_main_context ();

        [CCode (cname = "wp_core_connect")]
        public bool connect ();

        [CCode (cname = "wp_core_disconnect")]
        public void disconnect ();

        [CCode (cname = "wp_core_is_connected")]
        public bool is_connected ();

        [CCode (cname = "wp_core_get_remote_cookie")]
        public uint32 get_remote_cookie ();

        [CCode (cname = "wp_core_get_remote_name")]
        public unowned string? get_remote_name ();

        [CCode (cname = "wp_core_get_remote_version")]
        public unowned string? get_remote_version ();

        [CCode (cname = "wp_core_install_object_manager")]
        public void install_object_manager (ObjectManager om);

        [CCode (cname = "wp_core_load_component")]
        public void load_component (string component, string type, GLib.Variant? args, string? provides, GLib.Cancellable? cancellable, GLib.AsyncReadyCallback? callback);

        [CCode (cname = "wp_core_load_component_finish")]
        public bool load_component_finish (GLib.AsyncResult res) throws GLib.Error;

        [CCode (cname = "wp_core_sync")]
        public bool sync (GLib.Cancellable? cancellable, GLib.AsyncReadyCallback? callback);

        [CCode (cname = "wp_core_sync_finish")]
        public bool sync_finish (GLib.AsyncResult res) throws GLib.Error;

        public signal void connected ();
        public signal void disconnected ();
    }
}
