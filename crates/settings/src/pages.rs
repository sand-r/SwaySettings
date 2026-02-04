use glib::prelude::*;
use gtk4::prelude::*;

mod about_pc;
mod bluetooth;
mod power;
mod sound;
mod storage_row;
mod users;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, glib::Enum)]
#[enum_type(name = "SwaySettingsPageType")]
pub enum PageType {
    #[default]
    Users,
    AboutPc,
    Power,
    Wallpaper,
    Appearance,
    StartupApps,
    DefaultApps,
    Screenshot,
    Bluetooth,
    Sound,
    Keyboard,
    Mouse,
    Trackpad,
}

impl PageType {
    pub fn name(self) -> &'static str {
        match self {
            PageType::Users => "Users",
            PageType::AboutPc => "About This PC",
            PageType::Power => "Power",
            PageType::Wallpaper => "Wallpaper",
            PageType::Appearance => "Appearance",
            PageType::StartupApps => "Startup Apps",
            PageType::DefaultApps => "Default Apps",
            PageType::Screenshot => "Screenshot",
            PageType::Bluetooth => "Bluetooth",
            PageType::Sound => "Sound",
            PageType::Keyboard => "Keyboard",
            PageType::Mouse => "Mouse",
            PageType::Trackpad => "Trackpad",
        }
    }

    pub fn internal_name(self) -> &'static str {
        match self {
            PageType::Users => "users",
            PageType::AboutPc => "about",
            PageType::Power => "power",
            PageType::Wallpaper => "wallpaper",
            PageType::Appearance => "appearance",
            PageType::StartupApps => "startup-apps",
            PageType::DefaultApps => "default-apps",
            PageType::Screenshot => "screenshot",
            PageType::Bluetooth => "bluetooth",
            PageType::Sound => "sound",
            PageType::Keyboard => "keyboard",
            PageType::Mouse => "mouse",
            PageType::Trackpad => "trackpad",
        }
    }

    pub fn all() -> Vec<PageType> {
        vec![
            PageType::Users,
            PageType::AboutPc,
            PageType::Power,
            PageType::Wallpaper,
            PageType::Appearance,
            PageType::StartupApps,
            PageType::DefaultApps,
            PageType::Screenshot,
            PageType::Bluetooth,
            PageType::Sound,
            PageType::Keyboard,
            PageType::Mouse,
            PageType::Trackpad,
        ]
    }
}

pub fn create_placeholder(page: PageType) -> gtk4::Widget {
    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_bottom(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);

    let title = gtk4::Label::new(Some(page.name()));
    title.add_css_class("title-2");
    title.set_halign(gtk4::Align::Start);

    let subtitle = gtk4::Label::new(Some("Rust rewrite in progress."));
    subtitle.add_css_class("dim-label");
    subtitle.set_halign(gtk4::Align::Start);

    box_.append(&title);
    box_.append(&subtitle);

    box_.upcast()
}

pub fn create_page(page: PageType) -> gtk4::Widget {
    match page {
        PageType::AboutPc => about_pc::build_page(),
        PageType::Bluetooth => bluetooth::build_page(),
        PageType::Power => power::build_page(),
        PageType::Sound => sound::build_page(),
        PageType::Users => users::build_page(),
        _ => create_placeholder(page),
    }
}
