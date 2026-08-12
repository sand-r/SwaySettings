use std::cell::{Cell, RefCell};

use glib::prelude::*;
use glib::subclass::prelude::*;
use glib::Properties;

use crate::services::audio::ProfileInfo;

mod imp {
    use super::*;

    #[derive(Default, Properties)]
    #[properties(wrapper_type = super::ProfileItem)]
    pub struct ProfileItem {
        #[property(get, set)]
        pub index: Cell<u32>,

        #[property(get, set)]
        pub name: RefCell<String>,

        #[property(get, set)]
        pub description: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProfileItem {
        const NAME: &'static str = "SwaySettingsProfileItem";
        type Type = super::ProfileItem;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ProfileItem {}
}

glib::wrapper! {
    pub struct ProfileItem(ObjectSubclass<imp::ProfileItem>);
}

impl Default for ProfileItem {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileItem {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    pub fn from_info(info: &ProfileInfo) -> Self {
        let description = info.display_name().to_string();
        glib::Object::builder()
            .property("index", info.index)
            .property("name", &info.name)
            .property("description", &description)
            .build()
    }
}
