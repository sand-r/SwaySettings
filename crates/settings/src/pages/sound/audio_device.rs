use std::cell::{Cell, RefCell};

use glib::prelude::*;
use glib::subclass::prelude::*;
use glib::Properties;

use crate::services::pipewire::{DeviceInfo, DeviceType};

mod imp {
    use super::*;

    #[derive(Default, Properties)]
    #[properties(wrapper_type = super::AudioDevice)]
    pub struct AudioDevice {
        #[property(get, set)]
        pub id: Cell<u32>,

        #[property(get, set)]
        pub name: RefCell<String>,

        #[property(get, set)]
        pub description: RefCell<String>,

        #[property(get, set)]
        pub volume: Cell<f64>,

        #[property(get, set)]
        pub is_muted: Cell<bool>,

        #[property(get, set)]
        pub is_default: Cell<bool>,

        #[property(get, set)]
        pub is_sink: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioDevice {
        const NAME: &'static str = "SwaySettingsAudioDevice";
        type Type = super::AudioDevice;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioDevice {}
}

glib::wrapper! {
    pub struct AudioDevice(ObjectSubclass<imp::AudioDevice>);
}

impl Default for AudioDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioDevice {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Create from DeviceInfo
    pub fn from_info(info: &DeviceInfo) -> Self {
        let device: Self = glib::Object::builder()
            .property("id", info.id)
            .property("name", &info.name)
            .property("description", &info.description)
            .property("volume", info.volume)
            .property("is-muted", info.is_muted)
            .property("is-default", info.is_default)
            .property("is-sink", matches!(info.device_type, DeviceType::Sink))
            .build();
        device
    }

    /// Update from DeviceInfo
    pub fn update_from_info(&self, info: &DeviceInfo) {
        self.set_volume(info.volume);
        self.set_is_muted(info.is_muted);
        self.set_is_default(info.is_default);
    }

    /// Get the device type
    pub fn device_type(&self) -> DeviceType {
        if self.is_sink() {
            DeviceType::Sink
        } else {
            DeviceType::Source
        }
    }
}
