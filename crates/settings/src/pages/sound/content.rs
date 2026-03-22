use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gio::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use super::audio_device::AudioDevice;
use super::profile_item::ProfileItem;
use crate::services::pipewire::{
    cubic_to_linear, linear_to_cubic, AudioDaemon, AudioEvent, DeviceType, ProfileInfo,
    StreamInfo,
};

#[derive(Clone, Default)]
pub(crate) struct ProfileState {
    profiles: Vec<ProfileInfo>,
    active_index: Option<u32>,
}

mod imp {
    use super::*;

    #[derive(CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/SoundContent.ui")]
    pub struct SoundContent {
        #[template_child]
        pub stack: TemplateChild<gtk4::Stack>,

        // Output widgets
        #[template_child]
        pub output_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child]
        pub output_no_devices_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child]
        pub output_device_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child]
        pub output_profile_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child]
        pub output_slider: TemplateChild<gtk4::Scale>,
        #[template_child]
        pub output_mute_toggle: TemplateChild<gtk4::ToggleButton>,
        #[template_child]
        pub output_level_bar: TemplateChild<gtk4::LevelBar>,

        // Input widgets
        #[template_child]
        pub input_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child]
        pub input_no_devices_group: TemplateChild<libadwaita::PreferencesGroup>,
        #[template_child]
        pub input_device_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child]
        pub input_profile_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child]
        pub input_slider: TemplateChild<gtk4::Scale>,
        #[template_child]
        pub input_mute_toggle: TemplateChild<gtk4::ToggleButton>,
        #[template_child]
        pub input_level_bar: TemplateChild<gtk4::LevelBar>,

        // Applications
        #[template_child]
        pub apps_group: TemplateChild<libadwaita::PreferencesGroup>,
        /// Map stream id -> row widget
        pub stream_rows: RefCell<HashMap<u32, gtk4::Box>>,

        pub sinks: RefCell<gio::ListStore>,
        pub sources: RefCell<gio::ListStore>,
        pub output_profiles: RefCell<gio::ListStore>,
        pub input_profiles: RefCell<gio::ListStore>,
        pub(super) profiles: RefCell<HashMap<u32, ProfileState>>,
        pub daemon: RefCell<Option<Rc<AudioDaemon>>>,
        pub updating_ui: Cell<bool>,
    }

    impl Default for SoundContent {
        fn default() -> Self {
            Self {
                stack: TemplateChild::default(),
                output_group: TemplateChild::default(),
                output_no_devices_group: TemplateChild::default(),
                output_device_row: TemplateChild::default(),
                output_profile_row: TemplateChild::default(),
                output_slider: TemplateChild::default(),
                output_mute_toggle: TemplateChild::default(),
                output_level_bar: TemplateChild::default(),
                input_group: TemplateChild::default(),
                input_no_devices_group: TemplateChild::default(),
                input_device_row: TemplateChild::default(),
                input_profile_row: TemplateChild::default(),
                input_slider: TemplateChild::default(),
                input_mute_toggle: TemplateChild::default(),
                input_level_bar: TemplateChild::default(),
                apps_group: TemplateChild::default(),
                stream_rows: RefCell::new(HashMap::new()),
                sinks: RefCell::new(gio::ListStore::new::<AudioDevice>()),
                sources: RefCell::new(gio::ListStore::new::<AudioDevice>()),
                output_profiles: RefCell::new(gio::ListStore::new::<ProfileItem>()),
                input_profiles: RefCell::new(gio::ListStore::new::<ProfileItem>()),
                profiles: RefCell::new(HashMap::new()),
                daemon: RefCell::new(None),
                updating_ui: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SoundContent {
        const NAME: &'static str = "SwaySettingsSoundContent";
        type Type = super::SoundContent;
        type ParentType = libadwaita::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SoundContent {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup();
        }

        fn dispose(&self) {
            // Drop the daemon to trigger shutdown
            self.daemon.borrow_mut().take();
        }
    }

    impl WidgetImpl for SoundContent {}
    impl BinImpl for SoundContent {}
}

glib::wrapper! {
    pub struct SoundContent(ObjectSubclass<imp::SoundContent>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl SoundContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        let imp = self.imp();

        // Set up ComboRow models with expression for description
        self.setup_combo_row(&imp.output_device_row, &imp.sinks.borrow());
        self.setup_combo_row(&imp.input_device_row, &imp.sources.borrow());
        self.setup_profile_row(&imp.output_profile_row, &imp.output_profiles.borrow());
        self.setup_profile_row(&imp.input_profile_row, &imp.input_profiles.borrow());

        imp.output_profile_row.set_visible(false);
        imp.input_profile_row.set_visible(false);

        // Connect slider value-changed signals
        self.connect_output_controls();
        self.connect_input_controls();

        // Start the PipeWire daemon
        self.start_daemon();
    }

    fn setup_combo_row(&self, row: &libadwaita::ComboRow, model: &gio::ListStore) {
        row.set_model(Some(model));
        // Use a single factory for both the selected row and the popover list,
        // mirroring GNOME Control Center's CcDeviceComboRow.
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            hbox.set_hexpand(true);
            hbox.set_halign(gtk4::Align::Fill);

            let device_icon = gtk4::Image::builder()
                .accessible_role(gtk4::AccessibleRole::Presentation)
                .build();
            device_icon.set_margin_start(6);
            device_icon.set_margin_end(6);
            hbox.append(&device_icon);

            let label = gtk4::Label::new(None);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_width_chars(1);
            label.set_valign(gtk4::Align::Center);
            label.set_hexpand(true);
            hbox.append(&label);

            let checkmark = gtk4::Image::builder()
                .accessible_role(gtk4::AccessibleRole::Presentation)
                .icon_name("object-select-symbolic")
                .build();
            checkmark.set_opacity(0.0);
            hbox.append(&checkmark);

            list_item.set_child(Some(&hbox));
        });

        let row_weak = row.downgrade();
        factory.connect_bind(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let item = list_item.item().and_downcast::<AudioDevice>();
            let hbox = list_item.child().and_downcast::<gtk4::Box>();

            if let (Some(device), Some(hbox)) = (item, hbox) {
                let device_icon = hbox.first_child().and_downcast::<gtk4::Image>();
                let label = device_icon
                    .as_ref()
                    .and_then(|i| i.next_sibling())
                    .and_downcast::<gtk4::Label>();
                let checkmark = hbox.last_child().and_downcast::<gtk4::Image>();

                if let Some(device_icon) = device_icon {
                    let icon_name = get_device_icon(&device);
                    device_icon.set_icon_name(Some(&icon_name));
                }

                if let Some(label) = label {
                    label.set_label(&device.description());
                }

                if let (Some(checkmark), Some(row)) = (checkmark.clone(), row_weak.upgrade()) {
                    let list_item_for_notify = list_item.clone();
                    let checkmark_for_notify = checkmark.clone();
                    row.connect_selected_item_notify(move |row| {
                        let is_selected =
                            row.selected_item().as_ref() == list_item_for_notify.item().as_ref();
                        checkmark_for_notify.set_opacity(if is_selected { 1.0 } else { 0.0 });
                    });

                    let is_selected = row.selected_item().as_ref() == list_item.item().as_ref();
                    checkmark.set_opacity(if is_selected { 1.0 } else { 0.0 });

                    let checkmark_for_root = checkmark.clone();
                    let row_for_root = row.clone();
                    hbox.connect_notify_local(Some("root"), move |widget, _| {
                        let in_popover = widget.ancestor(gtk4::Popover::static_type()).is_some()
                            && widget
                                .ancestor(libadwaita::ComboRow::static_type())
                                .and_then(|combo| combo.downcast::<libadwaita::ComboRow>().ok())
                                .map(|combo| combo == row_for_root)
                                .unwrap_or(false);
                        checkmark_for_root.set_visible(in_popover);
                    });

                    let in_popover = hbox.ancestor(gtk4::Popover::static_type()).is_some()
                        && hbox
                            .ancestor(libadwaita::ComboRow::static_type())
                            .and_then(|combo| combo.downcast::<libadwaita::ComboRow>().ok())
                            .map(|combo| combo == row)
                            .unwrap_or(false);
                    checkmark.set_visible(in_popover);
                }
            }
        });

        row.set_factory(Some(&factory));
        row.set_list_factory(Some(&factory));
    }

    fn setup_profile_row(&self, row: &libadwaita::ComboRow, model: &gio::ListStore) {
        row.set_model(Some(model));

        // Selected item factory (collapsed row) - label only
        let selected_factory = gtk4::SignalListItemFactory::new();
        selected_factory.connect_setup(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let label = gtk4::Label::new(None);
            label.set_xalign(0.0);
            label.set_valign(gtk4::Align::Center);
            list_item.set_child(Some(&label));
        });
        selected_factory.connect_bind(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let item = list_item.item().and_downcast::<ProfileItem>();
            let label = list_item.child().and_downcast::<gtk4::Label>();

            if let (Some(profile), Some(label)) = (item, label) {
                label.set_label(&profile.description());
            }
        });
        row.set_factory(Some(&selected_factory));

        // List factory (dropdown) - label + checkmark
        let list_factory = gtk4::SignalListItemFactory::new();
        list_factory.connect_setup(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

            let label = gtk4::Label::new(None);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_width_chars(1);
            label.set_valign(gtk4::Align::Center);
            label.set_hexpand(true);
            hbox.append(&label);

            let checkmark = gtk4::Image::from_icon_name("object-select-symbolic");
            checkmark.set_opacity(0.0);
            hbox.append(&checkmark);

            list_item.set_child(Some(&hbox));
        });

        let row_weak = row.downgrade();
        list_factory.connect_bind(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let item = list_item.item().and_downcast::<ProfileItem>();
            let hbox = list_item.child().and_downcast::<gtk4::Box>();

            if let (Some(profile), Some(hbox)) = (item, hbox) {
                let label = hbox.first_child().and_downcast::<gtk4::Label>();
                let checkmark = hbox.last_child().and_downcast::<gtk4::Image>();

                if let Some(label) = label {
                    label.set_label(&profile.description());
                }

                if let (Some(checkmark), Some(row)) = (checkmark.clone(), row_weak.upgrade()) {
                    let list_item_for_notify = list_item.clone();
                    let checkmark_for_notify = checkmark.clone();
                    row.connect_selected_item_notify(move |row| {
                        let is_selected =
                            row.selected_item().as_ref() == list_item_for_notify.item().as_ref();
                        checkmark_for_notify.set_opacity(if is_selected { 1.0 } else { 0.0 });
                    });

                    let is_selected = row.selected_item().as_ref() == list_item.item().as_ref();
                    checkmark.set_opacity(if is_selected { 1.0 } else { 0.0 });
                }
            }
        });

        row.set_list_factory(Some(&list_factory));
    }

    fn connect_output_controls(&self) {
        let imp = self.imp();

        // Volume slider
        imp.output_slider.connect_value_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |slider| {
                let imp = this.imp();

                if imp.updating_ui.get() {
                    return;
                }

                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    if let Some(device) = this.get_selected_output() {
                        // Convert from UI scale (0-100) to linear (0-1.0)
                        let ui_volume = slider.value() / 100.0;
                        let linear_volume = cubic_to_linear(ui_volume);
                        daemon.set_volume(device.id(), linear_volume);
                    }
                }
            }
        ));

        // Mute toggle
        imp.output_mute_toggle.connect_toggled(clone!(
            #[weak(rename_to = this)]
            self,
            move |toggle| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }

                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    if let Some(device) = this.get_selected_output() {
                        daemon.set_mute(device.id(), toggle.is_active());
                    }
                }

                this.update_output_mute_icon();
            }
        ));

        // Device selection
        imp.output_device_row.connect_selected_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }

                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    let selected = row.selected();
                    if selected != gtk4::INVALID_LIST_POSITION {
                        let sinks = imp.sinks.borrow();
                        if let Some(item) = sinks.item(selected) {
                            if let Some(device) = item.downcast_ref::<AudioDevice>() {
                                daemon.set_default_sink(device.id());
                            }
                        }
                    }
                }

                this.refresh_profile_rows();
            }
        ));

        // Profile selection
        imp.output_profile_row.connect_selected_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }

                let daemon_ref = imp.daemon.borrow();
                let Some(daemon) = daemon_ref.as_ref() else {
                    return;
                };

                let Some(device) = this.get_selected_output() else {
                    return;
                };

                let Some(device_id) = this.device_id_from_device(&device) else {
                    return;
                };

                let Some(profile_item) = row.selected_item().and_downcast::<ProfileItem>() else {
                    return;
                };

                if this.active_profile_index(device_id) == Some(profile_item.index()) {
                    return;
                }

                daemon.set_profile(device_id, profile_item.index());
                this.set_active_profile_override(device_id, profile_item.index());
            }
        ));
    }

    fn connect_input_controls(&self) {
        let imp = self.imp();

        // Volume slider
        imp.input_slider.connect_value_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |slider| {
                let imp = this.imp();

                if imp.updating_ui.get() {
                    return;
                }

                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    if let Some(device) = this.get_selected_input() {
                        let ui_volume = slider.value() / 100.0;
                        let linear_volume = cubic_to_linear(ui_volume);
                        daemon.set_volume(device.id(), linear_volume);
                    }
                }

                // Update icon to reflect current input level
                this.update_input_mute_icon();
            }
        ));

        // Mute toggle
        imp.input_mute_toggle.connect_toggled(clone!(
            #[weak(rename_to = this)]
            self,
            move |toggle| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }

                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    if let Some(device) = this.get_selected_input() {
                        daemon.set_mute(device.id(), toggle.is_active());
                    }
                }

                this.update_input_mute_icon();
            }
        ));

        // Device selection
        imp.input_device_row.connect_selected_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }

                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    let selected = row.selected();
                    if selected != gtk4::INVALID_LIST_POSITION {
                        let sources = imp.sources.borrow();
                        if let Some(item) = sources.item(selected) {
                            if let Some(device) = item.downcast_ref::<AudioDevice>() {
                                daemon.set_default_source(device.id());
                            }
                        }
                    }
                }

                this.refresh_profile_rows();
            }
        ));

        // Profile selection
        imp.input_profile_row.connect_selected_notify(clone!(
            #[weak(rename_to = this)]
            self,
            move |row| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }

                let daemon_ref = imp.daemon.borrow();
                let Some(daemon) = daemon_ref.as_ref() else {
                    return;
                };

                let Some(device) = this.get_selected_input() else {
                    return;
                };

                let Some(device_id) = this.device_id_from_device(&device) else {
                    return;
                };

                let Some(profile_item) = row.selected_item().and_downcast::<ProfileItem>() else {
                    return;
                };

                if this.active_profile_index(device_id) == Some(profile_item.index()) {
                    return;
                }

                daemon.set_profile(device_id, profile_item.index());
                this.set_active_profile_override(device_id, profile_item.index());
            }
        ));
    }

    fn start_daemon(&self) {
        use std::sync::mpsc;

        let (sender, receiver) = mpsc::channel::<AudioEvent>();

        let daemon = Rc::new(AudioDaemon::new(move |event| {
            let _ = sender.send(event);
        }));

        self.imp().daemon.replace(Some(daemon));

        // Poll for events on the GTK main thread
        glib::timeout_add_local(
            std::time::Duration::from_millis(50),
            clone!(
                #[weak(rename_to = this)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    while let Ok(event) = receiver.try_recv() {
                        this.handle_event(event);
                    }
                    glib::ControlFlow::Continue
                }
            ),
        );
    }

    fn handle_event(&self, event: AudioEvent) {
        match event {
            AudioEvent::Ready => {
                log::info!("PipeWire daemon ready");
                self.imp().stack.set_visible_child_name("page");
            }
            AudioEvent::DeviceAdded(info) => {
                self.add_device(&info);
            }
            AudioEvent::DeviceRemoved(id) => {
                self.remove_device(id);
            }
            AudioEvent::DeviceChanged(info) => {
                self.update_device(&info);
            }
            AudioEvent::DefaultChanged(device_type, id) => {
                self.update_default(device_type, id);
            }
            AudioEvent::ProfilesUpdated(device_id, profiles, active_index) => {
                self.handle_profiles_updated(device_id, profiles, active_index);
            }
            AudioEvent::PeakLevel(device_type, level) => {
                self.update_peak_level(device_type, level);
            }
            AudioEvent::StreamAdded(info) => {
                self.add_stream(&info);
            }
            AudioEvent::StreamRemoved(id) => {
                self.remove_stream(id);
            }
            AudioEvent::StreamChanged(info) => {
                self.update_stream(&info);
            }
            AudioEvent::Error(msg) => {
                log::error!("PipeWire error: {}", msg);
            }
        }
    }

    fn update_peak_level(&self, device_type: DeviceType, level: f32) {
        let imp = self.imp();
        // Use exponential moving average for smooth animation
        const SMOOTHING: f64 = 0.3;

        match device_type {
            DeviceType::Sink => {
                let prev = imp.output_level_bar.value();
                let smoothed = (level as f64 * SMOOTHING) + (prev * (1.0 - SMOOTHING));
                imp.output_level_bar.set_value(smoothed.clamp(0.0, 1.0));
            }
            DeviceType::Source => {
                let prev = imp.input_level_bar.value();
                let smoothed = (level as f64 * SMOOTHING) + (prev * (1.0 - SMOOTHING));
                imp.input_level_bar.set_value(smoothed.clamp(0.0, 1.0));
            }
        }
    }

    fn add_device(&self, info: &crate::services::pipewire::DeviceInfo) {
        let imp = self.imp();
        let device = AudioDevice::from_info(info);

        match info.device_type {
            DeviceType::Sink => {
                let sinks = imp.sinks.borrow();
                sinks.append(&device);

                // Show output group, hide no-devices placeholder
                imp.output_group.set_visible(true);
                imp.output_no_devices_group.set_visible(false);

                // Select first device if none selected, but don't update controls yet
                // (wait for DeviceChanged with actual volume from PipeWire)
                if imp.output_device_row.selected() == gtk4::INVALID_LIST_POSITION {
                    imp.updating_ui.set(true);
                    imp.output_device_row.set_selected(0);
                    imp.updating_ui.set(false);
                }
            }
            DeviceType::Source => {
                let sources = imp.sources.borrow();
                sources.append(&device);

                // Show input group, hide no-devices placeholder
                imp.input_group.set_visible(true);
                imp.input_no_devices_group.set_visible(false);

                if imp.input_device_row.selected() == gtk4::INVALID_LIST_POSITION {
                    imp.updating_ui.set(true);
                    imp.input_device_row.set_selected(0);
                    imp.updating_ui.set(false);
                }
            }
        }

        self.refresh_profile_rows();
    }

    fn remove_device(&self, id: u32) {
        let imp = self.imp();

        // Try sinks first
        {
            let sinks = imp.sinks.borrow();
            if let Some(pos) = self.find_device_position(&sinks, id) {
                sinks.remove(pos);

                // Show no-devices placeholder if empty
                if sinks.n_items() == 0 {
                    imp.output_group.set_visible(false);
                    imp.output_no_devices_group.set_visible(true);
                }
                self.refresh_profile_rows();
                return;
            }
        }

        // Then sources
        {
            let sources = imp.sources.borrow();
            if let Some(pos) = self.find_device_position(&sources, id) {
                sources.remove(pos);

                // Show no-devices placeholder if empty
                if sources.n_items() == 0 {
                    imp.input_group.set_visible(false);
                    imp.input_no_devices_group.set_visible(true);
                }
            }
        }

        self.refresh_profile_rows();
    }

    fn update_device(&self, info: &crate::services::pipewire::DeviceInfo) {
        let imp = self.imp();
        imp.updating_ui.set(true);

        match info.device_type {
            DeviceType::Sink => {
                let sinks = imp.sinks.borrow();
                if let Some(device) = self.find_device(&sinks, info.id) {
                    device.update_from_info(info);

                    // Update controls if this is the selected device
                    if let Some(selected) = self.get_selected_output() {
                        if selected.id() == info.id {
                            self.update_output_controls(&device);
                        }
                    }
                }
            }
            DeviceType::Source => {
                let sources = imp.sources.borrow();
                if let Some(device) = self.find_device(&sources, info.id) {
                    device.update_from_info(info);

                    if let Some(selected) = self.get_selected_input() {
                        if selected.id() == info.id {
                            self.update_input_controls(&device);
                        }
                    }
                }
            }
        }

        imp.updating_ui.set(false);
    }

    fn update_default(&self, device_type: DeviceType, id: Option<u32>) {
        let imp = self.imp();
        imp.updating_ui.set(true);

        let Some(id) = id else {
            imp.updating_ui.set(false);
            return;
        };

        match device_type {
            DeviceType::Sink => {
                let sinks = imp.sinks.borrow();

                // Clear old default
                for i in 0..sinks.n_items() {
                    if let Some(item) = sinks.item(i) {
                        if let Some(device) = item.downcast_ref::<AudioDevice>() {
                            device.set_is_default(device.id() == id);
                        }
                    }
                }

                // Select in combo row
                if let Some(pos) = self.find_device_position(&sinks, id) {
                    imp.output_device_row.set_selected(pos);

                    // Update controls
                    if let Some(device) = self.find_device(&sinks, id) {
                        self.update_output_controls(&device);
                    }
                }
            }
            DeviceType::Source => {
                let sources = imp.sources.borrow();

                for i in 0..sources.n_items() {
                    if let Some(item) = sources.item(i) {
                        if let Some(device) = item.downcast_ref::<AudioDevice>() {
                            device.set_is_default(device.id() == id);
                        }
                    }
                }

                if let Some(pos) = self.find_device_position(&sources, id) {
                    imp.input_device_row.set_selected(pos);

                    if let Some(device) = self.find_device(&sources, id) {
                        self.update_input_controls(&device);
                    }
                }
            }
        }

        imp.updating_ui.set(false);

        // Update profile rows to reflect newly selected device
        self.refresh_profile_rows();
    }

    fn handle_profiles_updated(
        &self,
        device_id: u32,
        profiles: Vec<ProfileInfo>,
        active_index: Option<u32>,
    ) {
        let imp = self.imp();
        {
            let mut profiles_map = imp.profiles.borrow_mut();
            let resolved_active = active_index.or_else(|| {
                profiles_map
                    .get(&device_id)
                    .and_then(|state| state.active_index)
            });
            profiles_map.insert(
                device_id,
                ProfileState {
                    profiles,
                    active_index: resolved_active,
                },
            );
        }

        self.refresh_profile_rows();
    }

    fn refresh_profile_rows(&self) {
        self.update_profile_row(DeviceType::Sink);
        self.update_profile_row(DeviceType::Source);
    }

    fn set_active_profile_override(&self, device_id: u32, profile_index: u32) {
        let imp = self.imp();
        {
            let mut profiles_map = imp.profiles.borrow_mut();
            if let Some(state) = profiles_map.get_mut(&device_id) {
                state.active_index = Some(profile_index);
            }
        }
        self.refresh_profile_rows();
    }

    fn update_profile_row(&self, device_type: DeviceType) {
        let imp = self.imp();

        let (row, store, selected_device) = match device_type {
            DeviceType::Sink => (
                &imp.output_profile_row,
                &imp.output_profiles,
                self.get_selected_output(),
            ),
            DeviceType::Source => (
                &imp.input_profile_row,
                &imp.input_profiles,
                self.get_selected_input(),
            ),
        };

        let Some(device) = selected_device else {
            row.set_visible(false);
            store.borrow_mut().remove_all();
            return;
        };

        let Some(device_id) = self.device_id_from_device(&device) else {
            row.set_visible(false);
            store.borrow_mut().remove_all();
            return;
        };

        let profile_device_index = self.profile_device_index_from_device(&device);

        let profiles_state = {
            let profiles = imp.profiles.borrow();
            profiles.get(&device_id).cloned()
        };

        let Some(mut profiles_state) = profiles_state else {
            row.set_visible(false);
            store.borrow_mut().remove_all();
            return;
        };

        let active_index = profiles_state.active_index;

        // Filter profiles for this device/direction
        profiles_state.profiles.retain(|profile| {
            let is_active = active_index == Some(profile.index);
            let available = profile.is_available() || is_active;
            let supports = profile.supports_device(device_type, profile_device_index) || is_active;
            available && supports
        });

        if profiles_state.profiles.len() <= 1 {
            row.set_visible(false);
            store.borrow_mut().remove_all();
            return;
        }

        // Sort by priority (descending) then label
        profiles_state.profiles.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.display_name().cmp(b.display_name()))
        });

        imp.updating_ui.set(true);
        {
            let store = store.borrow_mut();
            store.remove_all();
            for profile in &profiles_state.profiles {
                store.append(&ProfileItem::from_info(profile));
            }
        }

        // Select active profile if known
        if let Some(active_index) = profiles_state.active_index {
            let store = store.borrow();
            let mut selected = gtk4::INVALID_LIST_POSITION;
            for i in 0..store.n_items() {
                if let Some(item) = store.item(i) {
                    if let Some(profile) = item.downcast_ref::<ProfileItem>() {
                        if profile.index() == active_index {
                            selected = i;
                            break;
                        }
                    }
                }
            }
            if selected == gtk4::INVALID_LIST_POSITION {
                // Keep current selection if possible to avoid flicker on transient profile updates.
                let current = row.selected();
                if current != gtk4::INVALID_LIST_POSITION && current < store.n_items() {
                    row.set_selected(current);
                } else {
                    row.set_selected(selected);
                }
            } else {
                row.set_selected(selected);
            }
        } else {
            let store = store.borrow();
            let current = row.selected();
            if current != gtk4::INVALID_LIST_POSITION && current < store.n_items() {
                row.set_selected(current);
            } else {
                row.set_selected(gtk4::INVALID_LIST_POSITION);
            }
        }
        imp.updating_ui.set(false);

        row.set_visible(true);
    }

    fn add_stream(&self, info: &StreamInfo) {
        let imp = self.imp();

        let row = self.build_stream_row(info);

        // Wrap in a PreferencesRow for the group
        let pref_row = libadwaita::PreferencesRow::builder()
            .activatable(false)
            .selectable(false)
            .build();
        pref_row.set_child(Some(&row));

        imp.apps_group.add(&pref_row);
        imp.stream_rows.borrow_mut().insert(info.id, row);
        imp.apps_group.set_visible(true);
    }

    fn remove_stream(&self, id: u32) {
        let imp = self.imp();
        let mut rows = imp.stream_rows.borrow_mut();
        if let Some(row) = rows.remove(&id) {
            // The row is inside a PreferencesRow; remove the parent
            if let Some(parent) = row.parent() {
                imp.apps_group.remove(&parent);
            }
        }
        if rows.is_empty() {
            imp.apps_group.set_visible(false);
        }
    }

    fn update_stream(&self, info: &StreamInfo) {
        let imp = self.imp();
        let rows = imp.stream_rows.borrow();
        let Some(row) = rows.get(&info.id) else {
            return;
        };

        imp.updating_ui.set(true);

        // Find the slider and mute toggle inside the row
        if let Some(slider) = find_child_by_name::<gtk4::Scale>(row, "stream-slider") {
            let ui_volume = linear_to_cubic(info.volume) * 100.0;
            if (slider.value() - ui_volume).abs() > 0.5 {
                slider.set_value(ui_volume);
            }
        }

        if let Some(toggle) = find_child_by_name::<gtk4::ToggleButton>(row, "stream-mute") {
            if toggle.is_active() != info.is_muted {
                toggle.set_active(info.is_muted);
            }
            let icon = if info.is_muted {
                "audio-volume-muted-symbolic"
            } else {
                let vol = linear_to_cubic(info.volume) * 100.0;
                if vol < 33.0 {
                    "audio-volume-low-symbolic"
                } else if vol < 66.0 {
                    "audio-volume-medium-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                }
            };
            toggle.set_icon_name(icon);
        }

        imp.updating_ui.set(false);
    }

    fn build_stream_row(&self, info: &StreamInfo) -> gtk4::Box {
        let hbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        // App icon
        let icon_name = info
            .icon_name
            .as_deref()
            .unwrap_or("application-x-executable-symbolic");
        let icon = gtk4::Image::from_icon_name(icon_name);
        icon.set_pixel_size(24);
        hbox.append(&icon);

        // App name
        let label = gtk4::Label::new(Some(&info.app_name));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_width_chars(1);
        label.set_hexpand(true);
        hbox.append(&label);

        // Mute toggle
        let mute_toggle = gtk4::ToggleButton::builder()
            .icon_name("audio-volume-high-symbolic")
            .valign(gtk4::Align::Center)
            .tooltip_text("Mute")
            .build();
        mute_toggle.add_css_class("flat");
        mute_toggle.set_widget_name("stream-mute");
        hbox.append(&mute_toggle);

        // Volume slider
        let adjustment = gtk4::Adjustment::new(0.0, 0.0, 100.0, 1.0, 10.0, 0.0);
        let slider = gtk4::Scale::builder()
            .adjustment(&adjustment)
            .hexpand(true)
            .draw_value(false)
            .build();
        slider.set_widget_name("stream-slider");

        // Set initial volume
        let ui_volume = linear_to_cubic(info.volume) * 100.0;
        slider.set_value(ui_volume);

        if info.is_muted {
            mute_toggle.set_active(true);
            mute_toggle.set_icon_name("audio-volume-muted-symbolic");
        }

        hbox.append(&slider);

        // Connect signals
        let stream_id = info.id;

        slider.connect_value_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |slider| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }
                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    let ui_volume = slider.value() / 100.0;
                    let linear_volume = cubic_to_linear(ui_volume);
                    daemon.set_stream_volume(stream_id, linear_volume);
                }
            }
        ));

        mute_toggle.connect_toggled(clone!(
            #[weak(rename_to = this)]
            self,
            move |toggle| {
                let imp = this.imp();
                if imp.updating_ui.get() {
                    return;
                }
                if let Some(daemon) = imp.daemon.borrow().as_ref() {
                    daemon.set_stream_mute(stream_id, toggle.is_active());
                }

                let icon = if toggle.is_active() {
                    "audio-volume-muted-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                };
                toggle.set_icon_name(icon);
            }
        ));

        hbox
    }

    fn update_output_controls(&self, device: &AudioDevice) {
        let imp = self.imp();

        // Convert linear volume to cubic UI scale (0-100)
        let ui_volume = linear_to_cubic(device.volume()) * 100.0;
        let current = imp.output_slider.value();

        // Only update if significantly different to avoid flicker
        if (current - ui_volume).abs() > 0.5 {
            imp.output_slider.set_value(ui_volume);
        }

        if imp.output_mute_toggle.is_active() != device.is_muted() {
            imp.output_mute_toggle.set_active(device.is_muted());
        }
        self.update_output_mute_icon();
    }

    fn update_input_controls(&self, device: &AudioDevice) {
        let imp = self.imp();

        let ui_volume = linear_to_cubic(device.volume()) * 100.0;
        let current = imp.input_slider.value();

        if (current - ui_volume).abs() > 0.5 {
            imp.input_slider.set_value(ui_volume);
        }

        if imp.input_mute_toggle.is_active() != device.is_muted() {
            imp.input_mute_toggle.set_active(device.is_muted());
        }
        self.update_input_mute_icon();
    }

    fn update_output_mute_icon(&self) {
        let imp = self.imp();
        let is_muted = imp.output_mute_toggle.is_active();
        let volume = imp.output_slider.value();

        let icon = if is_muted {
            "audio-volume-muted-symbolic"
        } else if volume < 33.0 {
            "audio-volume-low-symbolic"
        } else if volume < 66.0 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        };

        imp.output_mute_toggle.set_icon_name(icon);
    }

    fn update_input_mute_icon(&self) {
        let imp = self.imp();
        let is_muted = imp.input_mute_toggle.is_active();
        let volume = imp.input_slider.value();

        let icon = if is_muted || volume <= 0.0 {
            "microphone-sensitivity-muted-symbolic"
        } else if volume < 30.0 {
            "microphone-sensitivity-low-symbolic"
        } else if volume < 70.0 {
            "microphone-sensitivity-medium-symbolic"
        } else {
            "microphone-sensitivity-high-symbolic"
        };

        imp.input_mute_toggle.set_icon_name(icon);
    }

    fn get_selected_output(&self) -> Option<AudioDevice> {
        let imp = self.imp();
        let selected = imp.output_device_row.selected();
        if selected == gtk4::INVALID_LIST_POSITION {
            return None;
        }

        let sinks = imp.sinks.borrow();
        sinks
            .item(selected)
            .and_then(|item| item.downcast::<AudioDevice>().ok())
    }

    fn get_selected_input(&self) -> Option<AudioDevice> {
        let imp = self.imp();
        let selected = imp.input_device_row.selected();
        if selected == gtk4::INVALID_LIST_POSITION {
            return None;
        }

        let sources = imp.sources.borrow();
        sources
            .item(selected)
            .and_then(|item| item.downcast::<AudioDevice>().ok())
    }

    fn device_id_from_device(&self, device: &AudioDevice) -> Option<u32> {
        let device_id = device.device_id();
        if device_id >= 0 {
            Some(device_id as u32)
        } else {
            None
        }
    }

    fn profile_device_index_from_device(&self, device: &AudioDevice) -> Option<u32> {
        let index = device.profile_device_index();
        if index >= 0 {
            Some(index as u32)
        } else {
            None
        }
    }

    fn active_profile_index(&self, device_id: u32) -> Option<u32> {
        let imp = self.imp();
        imp.profiles
            .borrow()
            .get(&device_id)
            .and_then(|state| state.active_index)
    }

    fn find_device_position(&self, store: &gio::ListStore, id: u32) -> Option<u32> {
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i) {
                if let Some(device) = item.downcast_ref::<AudioDevice>() {
                    if device.id() == id {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    fn find_device(&self, store: &gio::ListStore, id: u32) -> Option<AudioDevice> {
        for i in 0..store.n_items() {
            if let Some(item) = store.item(i) {
                if let Some(device) = item.downcast_ref::<AudioDevice>() {
                    if device.id() == id {
                        return Some(device.clone());
                    }
                }
            }
        }
        None
    }
}

impl Default for SoundContent {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the icon name for an audio device
/// Uses PipeWire's device.icon_name if available, otherwise falls back to heuristics
fn get_device_icon(device: &AudioDevice) -> String {
    // First try PipeWire's icon
    if let Some(icon) = device.icon_name() {
        return format!("{}-symbolic", icon);
    }

    // Fallback based on device name/description
    let desc = device.description().to_lowercase();
    let name = device.name().to_lowercase();

    // Check for specific device types
    if desc.contains("headphone") || name.contains("headphone") {
        return "audio-headphones-symbolic".to_string();
    }
    if desc.contains("headset") || name.contains("headset") {
        return "audio-headset-symbolic".to_string();
    }
    if desc.contains("hdmi") || name.contains("hdmi") || desc.contains("displayport") {
        return "video-display-symbolic".to_string();
    }
    if desc.contains("bluetooth") || name.contains("bluez") {
        return "bluetooth-symbolic".to_string();
    }
    if desc.contains("usb") || name.contains("usb") {
        return "audio-card-symbolic".to_string();
    }
    // Virtual/filter output devices get a different icon
    if device.is_sink()
        && (name.contains("effect") || name.contains("filter") || desc.contains("virtual"))
    {
        return "audio-card-symbolic".to_string();
    }

    // Default based on device type (sink vs source)
    if device.is_sink() {
        "audio-speakers-symbolic".to_string()
    } else {
        "audio-input-microphone-symbolic".to_string()
    }
}

/// Find a child widget by its widget name, searching recursively
fn find_child_by_name<T: glib::prelude::IsA<gtk4::Widget>>(parent: &gtk4::Box, name: &str) -> Option<T> {
    let mut child = parent.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == name {
            return widget.downcast::<T>().ok();
        }
        child = widget.next_sibling();
    }
    None
}
