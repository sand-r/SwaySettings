mod imp {
    use std::cell::Cell;

    use gtk4::subclass::prelude::*;
    use gtk4::CompositeTemplate;

    use crate::pages::PageType;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/SidebarRow.ui")]
    pub struct SidebarRowImpl {
        #[template_child]
        pub icon: TemplateChild<gtk4::Image>,
        #[template_child]
        pub label: TemplateChild<gtk4::Label>,

        pub page_type: Cell<PageType>,
        pub group: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarRowImpl {
        const NAME: &'static str = "SwaySettingsSidebarRow";
        type Type = super::SidebarRow;
        type ParentType = gtk4::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SidebarRowImpl {}
    impl WidgetImpl for SidebarRowImpl {}
    impl ListBoxRowImpl for SidebarRowImpl {}
}

use glib::Object;
use gtk4::subclass::prelude::*;

use crate::pages::PageType;

glib::wrapper! {
    pub struct SidebarRow(ObjectSubclass<imp::SidebarRowImpl>)
        @extends gtk4::ListBoxRow, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Actionable, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl SidebarRow {
    pub fn new(page: PageType, icon: &str, group: u32) -> Self {
        let row: Self = Object::builder().build();
        let imp = row.imp();

        imp.page_type.set(page);
        imp.group.set(group);
        imp.icon.set_icon_name(Some(icon));
        imp.label.set_label(page.name());

        row
    }

    pub fn page_type(&self) -> PageType {
        self.imp().page_type.get()
    }

    pub fn group(&self) -> u32 {
        self.imp().group.get()
    }
}
