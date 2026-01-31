use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gio::prelude::*;
use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use super::audio_device::AudioDevice;
use crate::services::pipewire::{
    cubic_to_linear, linear_to_cubic, AudioDaemon, AudioEvent, DeviceType,
};

mod imp {
    use super::*;

    #[derive(CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/SoundContent.ui")]
    pub struct SoundContent {
        #[template_child]
        pub output_device_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child]
        pub output_slider: TemplateChild<gtk4::Scale>,
        #[template_child]
        pub output_mute_toggle: TemplateChild<gtk4::ToggleButton>,
        #[template_child]
        pub input_device_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child]
        pub input_slider: TemplateChild<gtk4::Scale>,
        #[template_child]
        pub input_mute_toggle: TemplateChild<gtk4::ToggleButton>,

        pub sinks: RefCell<gio::ListStore>,
        pub sources: RefCell<gio::ListStore>,
        pub daemon: RefCell<Option<Rc<AudioDaemon>>>,
        pub updating_ui: Cell<bool>,
    }

    impl Default for SoundContent {
        fn default() -> Self {
            Self {
                output_device_row: TemplateChild::default(),
                output_slider: TemplateChild::default(),
                output_mute_toggle: TemplateChild::default(),
                input_device_row: TemplateChild::default(),
                input_slider: TemplateChild::default(),
                input_mute_toggle: TemplateChild::default(),
                sinks: RefCell::new(gio::ListStore::new::<AudioDevice>()),
                sources: RefCell::new(gio::ListStore::new::<AudioDevice>()),
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

        // Connect slider value-changed signals
        self.connect_output_controls();
        self.connect_input_controls();

        // Start the PipeWire daemon
        self.start_daemon();
    }

    fn setup_combo_row(&self, row: &libadwaita::ComboRow, model: &gio::ListStore) {
        // Create expression to get description from AudioDevice
        let expression = gtk4::PropertyExpression::new(
            AudioDevice::static_type(),
            gtk4::Expression::NONE,
            "description",
        );

        row.set_model(Some(model));
        row.set_expression(Some(expression));

        // Create a custom factory for the dropdown that shows full text with icon
        let factory = gtk4::SignalListItemFactory::new();

        factory.connect_setup(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();

            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);

            let icon = gtk4::Image::new();
            icon.set_icon_size(gtk4::IconSize::Normal);
            hbox.append(&icon);

            let label = gtk4::Label::new(None);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            // Don't ellipsize - show full text
            label.set_ellipsize(gtk4::pango::EllipsizeMode::None);
            label.set_wrap(true);
            label.set_wrap_mode(gtk4::pango::WrapMode::Word);
            hbox.append(&label);

            list_item.set_child(Some(&hbox));
        });

        factory.connect_bind(|_, list_item| {
            let list_item = list_item.downcast_ref::<gtk4::ListItem>().unwrap();
            let item = list_item.item().and_downcast::<AudioDevice>();
            let hbox = list_item.child().and_downcast::<gtk4::Box>();

            if let (Some(device), Some(hbox)) = (item, hbox) {
                // Get icon and label from hbox
                if let Some(icon) = hbox.first_child().and_downcast::<gtk4::Image>() {
                    let icon_name = get_device_icon(&device);
                    icon.set_icon_name(Some(icon_name));
                }
                if let Some(label) = hbox.last_child().and_downcast::<gtk4::Label>() {
                    label.set_label(&device.description());
                }
            }
        });

        row.set_list_factory(Some(&factory));
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
                        // Convert from UI scale (0-150) to linear (0-1.5)
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
            AudioEvent::Error(msg) => {
                log::error!("PipeWire error: {}", msg);
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

                if imp.input_device_row.selected() == gtk4::INVALID_LIST_POSITION {
                    imp.updating_ui.set(true);
                    imp.input_device_row.set_selected(0);
                    imp.updating_ui.set(false);
                }
            }
        }
    }

    fn remove_device(&self, id: u32) {
        let imp = self.imp();

        // Try sinks first
        {
            let sinks = imp.sinks.borrow();
            if let Some(pos) = self.find_device_position(&sinks, id) {
                sinks.remove(pos);
                return;
            }
        }

        // Then sources
        {
            let sources = imp.sources.borrow();
            if let Some(pos) = self.find_device_position(&sources, id) {
                sources.remove(pos);
            }
        }
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

        let icon = if is_muted {
            "microphone-disabled-symbolic"
        } else {
            "audio-input-microphone-symbolic"
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

/// Get an appropriate icon name for an audio device based on its name/description
fn get_device_icon(device: &AudioDevice) -> &'static str {
    let desc = device.description().to_lowercase();
    let name = device.name().to_lowercase();

    // Check for specific device types
    if desc.contains("headphone") || name.contains("headphone") {
        return "audio-headphones-symbolic";
    }
    if desc.contains("headset") || name.contains("headset") {
        return "audio-headset-symbolic";
    }
    if desc.contains("hdmi") || name.contains("hdmi") || desc.contains("displayport") {
        return "video-display-symbolic";
    }
    if desc.contains("bluetooth") || name.contains("bluez") {
        return "bluetooth-symbolic";
    }
    if desc.contains("usb") || name.contains("usb") {
        return "audio-card-symbolic";
    }

    // Default based on device type (sink vs source)
    if device.is_sink() {
        "audio-speakers-symbolic"
    } else {
        "audio-input-microphone-symbolic"
    }
}
