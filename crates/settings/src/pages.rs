mod about_pc;
mod appearance;
mod bluetooth;
mod default_apps;
mod keyboard;
mod mouse;
mod power;
mod screenshot;
mod sound;
mod startup_apps;
mod storage_row;
mod trackpad;
mod users;
mod wallpaper;

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

pub fn create_page(page: PageType) -> gtk4::Widget {
    match page {
        PageType::AboutPc => about_pc::build_page(),
        PageType::Appearance => appearance::build_page(),
        PageType::Bluetooth => bluetooth::build_page(),
        PageType::DefaultApps => default_apps::build_page(),
        PageType::Keyboard => keyboard::build_page(),
        PageType::Mouse => mouse::build_page(),
        PageType::Power => power::build_page(),
        PageType::Screenshot => screenshot::build_page(),
        PageType::Sound => sound::build_page(),
        PageType::StartupApps => startup_apps::build_page(),
        PageType::Trackpad => trackpad::build_page(),
        PageType::Users => users::build_page(),
        PageType::Wallpaper => wallpaper::build_page(),
    }
}
