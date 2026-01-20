using Gee;

namespace SwaySettings {
    public class DefaultApps : PageScroll {

        public static DefaultAppGroup[] app_groups = {
            DefaultAppGroup ("Internet", {
                DefaultAppData ("Web Browser", "x-scheme-handler/http",
                                  { "text/html", "application/xhtml+xml", "x-scheme-handler/https" }),
                DefaultAppData ("Mail Client", "x-scheme-handler/mailto"),
                DefaultAppData ("Calendar", "text/calendar"),
            }),
            DefaultAppGroup ("Media", {
                DefaultAppData ("Music", "audio/x-vorbis+ogg", { "audio/*" }),
                DefaultAppData ("Video", "video/x-ogm+ogg", { "video/*" }),
                DefaultAppData ("Photos", "image/jpeg", { "image/*" }),
            }),
            DefaultAppGroup ("System", {
                DefaultAppData ("Text Editor", "text/plain"),
                DefaultAppData ("File Browser", "inode/directory"),
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
                    pref_group.add (get_item (app_data));
                }

                box.append (pref_group);
            }

            return box;
        }

        Adw.ActionRow get_item (DefaultAppData def_app) {
            var row = new Adw.ActionRow ();
            row.title = def_app.category_name;

            var chooser = new Gtk.AppChooserButton (def_app.mime_type) {
                valign = Gtk.Align.CENTER,
            };
            chooser.show_dialog_item = true;
            chooser.show_default_item = true;
            chooser.changed.connect ((combo_box) => {
                var selected_app = chooser.get_app_info ();
                if (selected_app == null) return;
                set_default_app (def_app, selected_app);
            });

            row.add_suffix (chooser);
            row.activatable_widget = chooser;

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
