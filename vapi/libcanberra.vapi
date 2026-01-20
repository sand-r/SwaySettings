[CCode (cheader_filename = "canberra.h")]
namespace Canberra {
    [CCode (cname = "ca_context", free_function = "ca_context_destroy")]
    [Compact]
    public class Context {
        [CCode (cname = "ca_context_create")]
        public static int create (out Context? context);

        [CCode (cname = "ca_context_destroy")]
        public int destroy ();

        [CCode (cname = "ca_context_set_driver")]
        public int set_driver (string driver);

        [CCode (cname = "ca_context_change_device")]
        public int change_device (string device);

        [CCode (cname = "ca_context_open")]
        public int open ();

        [CCode (cname = "ca_context_cancel")]
        public int cancel (uint32 id);

        [CCode (cname = "ca_context_change_props", sentinel = "NULL")]
        public int change_props (...);

        [CCode (cname = "ca_context_play", sentinel = "NULL")]
        public int play (uint32 id, ...);

        [CCode (cname = "ca_context_play_full", sentinel = "NULL")]
        public int play_full (uint32 id, Proplist? proplist, FinishCallback? callback);
    }

    [CCode (cname = "ca_proplist", free_function = "ca_proplist_destroy")]
    [Compact]
    public class Proplist {
        [CCode (cname = "ca_proplist_create")]
        public static int create (out Proplist? proplist);

        [CCode (cname = "ca_proplist_destroy")]
        public int destroy ();

        [CCode (cname = "ca_proplist_sets")]
        public int sets (string key, string value);

        [CCode (cname = "ca_proplist_setf")]
        public int setf (string key, string format, ...);
    }

    [CCode (cname = "ca_finish_callback_t", has_target = false)]
    public delegate void FinishCallback (Context context, uint32 id, int error_code);

    // Property keys
    [CCode (cname = "CA_PROP_MEDIA_NAME")]
    public const string PROP_MEDIA_NAME;
    [CCode (cname = "CA_PROP_MEDIA_FILENAME")]
    public const string PROP_MEDIA_FILENAME;
    [CCode (cname = "CA_PROP_MEDIA_ROLE")]
    public const string PROP_MEDIA_ROLE;
    [CCode (cname = "CA_PROP_EVENT_ID")]
    public const string PROP_EVENT_ID;
    [CCode (cname = "CA_PROP_EVENT_DESCRIPTION")]
    public const string PROP_EVENT_DESCRIPTION;
    [CCode (cname = "CA_PROP_CANBERRA_FORCE_CHANNEL")]
    public const string PROP_CANBERRA_FORCE_CHANNEL;
    [CCode (cname = "CA_PROP_APPLICATION_NAME")]
    public const string PROP_APPLICATION_NAME;

    // Error codes
    [CCode (cname = "CA_SUCCESS")]
    public const int SUCCESS;
    [CCode (cname = "CA_ERROR_NOTSUPPORTED")]
    public const int ERROR_NOTSUPPORTED;
    [CCode (cname = "CA_ERROR_INVALID")]
    public const int ERROR_INVALID;
    [CCode (cname = "CA_ERROR_STATE")]
    public const int ERROR_STATE;
    [CCode (cname = "CA_ERROR_OOM")]
    public const int ERROR_OOM;
    [CCode (cname = "CA_ERROR_NODRIVER")]
    public const int ERROR_NODRIVER;
    [CCode (cname = "CA_ERROR_SYSTEM")]
    public const int ERROR_SYSTEM;
    [CCode (cname = "CA_ERROR_CORRUPT")]
    public const int ERROR_CORRUPT;
    [CCode (cname = "CA_ERROR_TOOBIG")]
    public const int ERROR_TOOBIG;
    [CCode (cname = "CA_ERROR_NOTFOUND")]
    public const int ERROR_NOTFOUND;
    [CCode (cname = "CA_ERROR_DESTROYED")]
    public const int ERROR_DESTROYED;
    [CCode (cname = "CA_ERROR_CANCELED")]
    public const int ERROR_CANCELED;
    [CCode (cname = "CA_ERROR_NOTAVAILABLE")]
    public const int ERROR_NOTAVAILABLE;
    [CCode (cname = "CA_ERROR_ACCESS")]
    public const int ERROR_ACCESS;
    [CCode (cname = "CA_ERROR_IO")]
    public const int ERROR_IO;
    [CCode (cname = "CA_ERROR_INTERNAL")]
    public const int ERROR_INTERNAL;
    [CCode (cname = "CA_ERROR_DISABLED")]
    public const int ERROR_DISABLED;
    [CCode (cname = "CA_ERROR_FORKED")]
    public const int ERROR_FORKED;
    [CCode (cname = "CA_ERROR_DISCONNECTED")]
    public const int ERROR_DISCONNECTED;

    [CCode (cname = "ca_strerror")]
    public static unowned string strerror (int code);
}
