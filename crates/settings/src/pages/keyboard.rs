use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::prelude::*;
use libadwaita::subclass::prelude::*;

use crate::services::sway_input::{load_keyboard_layouts, set_keyboard_layouts};

#[derive(Clone, Debug)]
struct LayoutEntry {
    code: String,
    description: String,
}

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/org/erikreider/swaysettings/ui/KeyboardContent.ui")]
    pub struct KeyboardContent {
        #[template_child(id = "stack")]
        pub(super) stack: TemplateChild<gtk4::Stack>,
        #[template_child(id = "layouts_list")]
        pub(super) layouts_list: TemplateChild<gtk4::ListBox>,
        #[template_child(id = "available_layout_row")]
        pub(super) available_layout_row: TemplateChild<libadwaita::ComboRow>,
        #[template_child(id = "add_button")]
        pub(super) add_button: TemplateChild<gtk4::Button>,
        #[template_child(id = "remove_button")]
        pub(super) remove_button: TemplateChild<gtk4::Button>,
        #[template_child(id = "up_button")]
        pub(super) up_button: TemplateChild<gtk4::Button>,
        #[template_child(id = "down_button")]
        pub(super) down_button: TemplateChild<gtk4::Button>,

        pub(super) available_layouts: RefCell<Vec<LayoutEntry>>,
        pub(super) selectable_layouts: RefCell<Vec<LayoutEntry>>,
        pub(super) selected_layout_codes: RefCell<Vec<String>>,
        pub(super) updating_ui: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for KeyboardContent {
        const NAME: &'static str = "SwaySettingsKeyboardContent";
        type Type = super::KeyboardContent;
        type ParentType = libadwaita::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for KeyboardContent {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for KeyboardContent {}
    impl BinImpl for KeyboardContent {}
}

glib::wrapper! {
    pub struct KeyboardContent(ObjectSubclass<imp::KeyboardContent>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

pub fn build_page() -> gtk4::Widget {
    KeyboardContent::new().upcast()
}

impl KeyboardContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        let imp = self.imp();
        imp.available_layouts.replace(load_available_layouts());

        self.connect_signals();
        self.refresh_from_system();
    }

    fn connect_signals(&self) {
        let imp = self.imp();

        imp.layouts_list.connect_selected_rows_changed(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.update_buttons();
            }
        ));

        imp.add_button.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.add_selected_layout();
            }
        ));

        imp.remove_button.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.remove_selected_layout();
            }
        ));

        imp.up_button.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.move_selected_layout(-1);
            }
        ));

        imp.down_button.connect_clicked(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.move_selected_layout(1);
            }
        ));
    }

    fn refresh_from_system(&self) {
        let current = match load_keyboard_layouts() {
            Ok(layouts) => layouts,
            Err(err) => {
                log::warn!("Failed to load keyboard layouts from sway: {err}");
                None
            }
        };

        let imp = self.imp();
        if let Some(mut layouts) = current {
            if layouts.is_empty() {
                if let Some(first) = imp.available_layouts.borrow().first() {
                    layouts.push(first.code.clone());
                }
            }

            imp.selected_layout_codes.replace(layouts);
            imp.stack.set_visible_child_name("page");
            self.refresh_layout_widgets();
        } else {
            imp.stack.set_visible_child_name("placeholder");
        }
    }

    fn refresh_layout_widgets(&self) {
        let imp = self.imp();
        imp.updating_ui.set(true);

        self.rebuild_selected_list();
        self.rebuild_available_combo();

        if imp.layouts_list.selected_row().is_none() {
            if let Some(row) = imp.layouts_list.row_at_index(0) {
                imp.layouts_list.select_row(Some(&row));
            }
        }

        imp.updating_ui.set(false);
        self.update_buttons();
    }

    fn rebuild_selected_list(&self) {
        let imp = self.imp();

        while let Some(child) = imp.layouts_list.first_child() {
            imp.layouts_list.remove(&child);
        }

        for code in imp.selected_layout_codes.borrow().iter() {
            let title = layout_description(code, &imp.available_layouts.borrow());
            let row = libadwaita::ActionRow::builder()
                .title(title)
                .subtitle(code)
                .build();
            row.set_activatable(true);
            row.set_selectable(true);
            imp.layouts_list.append(&row);
        }
    }

    fn rebuild_available_combo(&self) {
        let imp = self.imp();

        let selected: HashSet<_> = imp.selected_layout_codes.borrow().iter().cloned().collect();
        let selectable: Vec<LayoutEntry> = imp
            .available_layouts
            .borrow()
            .iter()
            .filter(|layout| !selected.contains(&layout.code))
            .cloned()
            .collect();

        let model = gio::ListStore::new::<gtk4::StringObject>();
        for layout in &selectable {
            model.append(&gtk4::StringObject::new(&layout.description));
        }

        imp.selectable_layouts.replace(selectable.clone());
        imp.available_layout_row.set_model(Some(&model));
        imp.available_layout_row.set_selected(0);
        imp.available_layout_row.set_visible(!selectable.is_empty());
    }

    fn update_buttons(&self) {
        let imp = self.imp();
        let selected_index = imp
            .layouts_list
            .selected_row()
            .map(|row| row.index() as usize);

        let len = imp.selected_layout_codes.borrow().len();
        let has_selectable = !imp.selectable_layouts.borrow().is_empty();

        imp.add_button.set_sensitive(has_selectable);
        imp.remove_button
            .set_sensitive(selected_index.is_some() && len > 1);

        let can_move_up = selected_index.map(|idx| idx > 0).unwrap_or(false);
        let can_move_down = selected_index.map(|idx| idx + 1 < len).unwrap_or(false);

        imp.up_button.set_sensitive(can_move_up);
        imp.down_button.set_sensitive(can_move_down);
    }

    fn add_selected_layout(&self) {
        let imp = self.imp();
        if imp.updating_ui.get() {
            return;
        }

        let idx = imp.available_layout_row.selected() as usize;
        let Some(layout) = imp.selectable_layouts.borrow().get(idx).cloned() else {
            return;
        };

        let mut selected = imp.selected_layout_codes.borrow_mut();
        if !selected.contains(&layout.code) {
            selected.push(layout.code);
        }
        drop(selected);

        self.persist_layouts();
        self.refresh_layout_widgets();
    }

    fn remove_selected_layout(&self) {
        let imp = self.imp();
        if imp.updating_ui.get() {
            return;
        }

        if imp.selected_layout_codes.borrow().len() <= 1 {
            return;
        }

        let Some(index) = imp
            .layouts_list
            .selected_row()
            .map(|row| row.index() as usize)
        else {
            return;
        };

        let mut selected = imp.selected_layout_codes.borrow_mut();
        if index < selected.len() {
            selected.remove(index);
        }
        drop(selected);

        self.persist_layouts();
        self.refresh_layout_widgets();
    }

    fn move_selected_layout(&self, direction: isize) {
        let imp = self.imp();
        if imp.updating_ui.get() {
            return;
        }

        let Some(index) = imp
            .layouts_list
            .selected_row()
            .map(|row| row.index() as usize)
        else {
            return;
        };

        let mut selected = imp.selected_layout_codes.borrow_mut();
        let next_index = index as isize + direction;
        if next_index < 0 || next_index as usize >= selected.len() {
            return;
        }

        selected.swap(index, next_index as usize);
        drop(selected);

        self.persist_layouts();
        self.refresh_layout_widgets();

        if let Some(row) = imp.layouts_list.row_at_index(next_index as i32) {
            imp.layouts_list.select_row(Some(&row));
        }
    }

    fn persist_layouts(&self) {
        let layouts = self.imp().selected_layout_codes.borrow().clone();
        if let Err(err) = set_keyboard_layouts(&layouts) {
            log::warn!("Failed to apply keyboard layouts: {err}");
            self.refresh_from_system();
        }
    }
}

fn layout_description(code: &str, available: &[LayoutEntry]) -> String {
    available
        .iter()
        .find(|layout| layout.code == code)
        .map(|layout| layout.description.clone())
        .unwrap_or_else(|| code.to_string())
}

fn load_available_layouts() -> Vec<LayoutEntry> {
    let xml = match std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.xml") {
        Ok(xml) => xml,
        Err(err) => {
            log::warn!("Failed to read XKB layout file: {err}");
            return Vec::new();
        }
    };

    let doc = match roxmltree::Document::parse(&xml) {
        Ok(doc) => doc,
        Err(err) => {
            log::warn!("Failed to parse XKB layout file: {err}");
            return Vec::new();
        }
    };

    let mut layouts = Vec::new();

    for layout in doc.descendants().filter(|node| node.has_tag_name("layout")) {
        let Some(config_item) = layout
            .children()
            .find(|child| child.is_element() && child.has_tag_name("configItem"))
        else {
            continue;
        };

        let name = config_item
            .children()
            .find(|child| child.is_element() && child.has_tag_name("name"))
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|text| !text.is_empty());

        let description = config_item
            .children()
            .find(|child| child.is_element() && child.has_tag_name("description"))
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|text| !text.is_empty());

        let (Some(code), Some(description)) = (name, description) else {
            continue;
        };

        layouts.push(LayoutEntry {
            code: code.to_string(),
            description: format!("{} ({})", description, code),
        });
    }

    layouts.sort_by(|a, b| a.description.cmp(&b.description));
    layouts.dedup_by(|a, b| a.code == b.code);
    layouts
}
