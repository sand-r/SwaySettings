using Gee;

namespace SwaySettings {
    public class DefaultApps : PageScroll {

        public static DefaultAppGroup[] app_groups = {
            DefaultAppGroup ("Default Apps", {
                DefaultAppData ("Web", "x-scheme-handler/http",
                                  { "text/html", "application/xhtml+xml", "x-scheme-handler/https" }),
                DefaultAppData ("Mail", "x-scheme-handler/mailto"),
                DefaultAppData ("Calendar", "text/calendar"),
                DefaultAppData ("Music", "audio/x-vorbis+ogg", { "audio/*" }),
                DefaultAppData ("Video", "video/x-ogm+ogg", { "video/*" }),
                DefaultAppData ("Photos", "image/jpeg", { "image/*" }),
            }),
        };

        public DefaultApps (SettingsItem item, Adw.NavigationPage page) {
            base (item, page);
        }

        public override Gtk.Widget set_child () {
            var box = new Gtk.Box (Gtk.Orientation.VERTICAL, 24);
            box.valign = Gtk.Align.START;

            foreach (var group in app_groups) {
                var pref_group = new Adw.PreferencesGroup ();
                pref_group.title = group.name;

                foreach (var app_data in group.items) {
                    pref_group.add (create_app_row (app_data));
                }

                box.append (pref_group);
            }

            return box;
        }

        Adw.ComboRow create_app_row (DefaultAppData def_app) {
            var row = new Adw.ComboRow ();
            row.title = def_app.category_name;

            // Get available apps for this MIME type
            var apps = AppInfo.get_all_for_type (def_app.mime_type);
            var default_app = AppInfo.get_default_for_type (def_app.mime_type, false);

            // Create model with AppInfo objects
            var model = new GLib.ListStore (typeof (AppInfoItem));
            uint default_index = 0;
            uint index = 0;

            foreach (var app in apps) {
                if (app == null) continue;
                model.append (new AppInfoItem (app));
                if (default_app != null && app.get_id () == default_app.get_id ()) {
                    default_index = index;
                }
                index++;
            }

            row.model = model;

            // Create factory for displaying app icon + name
            var factory = new Gtk.SignalListItemFactory ();
            factory.setup.connect ((item) => {
                var list_item = item as Gtk.ListItem;
                var box = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
                box.valign = Gtk.Align.CENTER;

                var image = new Gtk.Image ();
                image.pixel_size = 16;
                box.append (image);

                var label = new Gtk.Label ("");
                label.xalign = 0;
                box.append (label);

                list_item.child = box;
            });

            factory.bind.connect ((item) => {
                var list_item = item as Gtk.ListItem;
                var app_item = list_item.item as AppInfoItem;
                if (app_item == null) return;

                var box = list_item.child as Gtk.Box;
                var image = box.get_first_child () as Gtk.Image;
                var label = image.get_next_sibling () as Gtk.Label;

                var icon = app_item.app_info.get_icon ();
                if (icon != null) {
                    image.set_from_gicon (icon);
                } else {
                    image.set_from_icon_name ("application-x-executable");
                }
                label.label = app_item.app_info.get_name ();
            });

            row.factory = factory;
            row.list_factory = factory;

            // Set current selection
            row.selected = default_index;

            // Handle selection changes
            row.notify["selected"].connect (() => {
                var selected_item = model.get_item (row.selected) as AppInfoItem;
                if (selected_item == null) return;
                set_default_app (def_app, selected_item.app_info);
            });

            return row;
        }

        void set_default_app (DefaultAppData def_data, AppInfo selected_app) {
            set_default_for_mime (def_data.mime_type, selected_app);

            // Try to set as default for the other extra types
            if (def_data.extra_types.length > 0) {
                PatternSpec[] patterns = {};
                foreach (var type in def_data.extra_types) {
                    patterns += new PatternSpec (type);
                }

                foreach (var mime in selected_app.get_supported_types ()) {
                    bool found_match = false;
                    foreach (unowned PatternSpec pattern in patterns) {
                        if (pattern.match_string (mime)) {
                            found_match = true;
                            continue;
                        }
                    }
                    if (!found_match) continue;
                    set_default_for_mime (mime, selected_app);
                }
            }
        }

        void set_default_for_mime (string mime_type, AppInfo selected_app) {
            try {
                selected_app.set_as_default_for_type (mime_type);
            } catch (Error e) {
                stderr.printf ("Error! Could not set %s as default app!\n",
                               selected_app.get_name ());
            }
        }
    }

    // Wrapper class for AppInfo to use in ListStore
    public class AppInfoItem : Object {
        public AppInfo app_info { get; construct; }

        public AppInfoItem (AppInfo app_info) {
            Object (app_info: app_info);
        }
    }

    public struct DefaultAppGroup {
        string name;
        DefaultAppData[] items;

        DefaultAppGroup (string name, DefaultAppData[] items) {
            this.name = name;
            this.items = items;
        }
    }

    public struct DefaultAppData {
        string category_name;
        string mime_type;
        string[] extra_types;

        DefaultAppData (string category_name,
                          string mime_type,
                          string[] extra_types = {}) {
            this.mime_type = mime_type;
            this.category_name = category_name;
            this.extra_types = extra_types;
        }
    }
}
