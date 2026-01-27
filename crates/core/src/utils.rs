use std::path::{Path, PathBuf};

use gio::prelude::*;
use glib::VariantTy;
use once_cell::sync::Lazy;
use std::cell::RefCell;

use crate::constants;
use crate::functions;

thread_local! {
    static WALLPAPER_APP: RefCell<Option<gio::Application>> = RefCell::new(None);
}
static DEFAULT_WALLPAPER_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = glib::user_config_dir();
    path.push("swaysettings-wallpaper");
    path
});

pub fn wallpaper_application_registered() -> bool {
    WALLPAPER_APP.with(|cell| {
        let mut app_opt = cell.borrow_mut();
        if app_opt.is_none() {
            *app_opt = Some(gio::Application::new(
                Some("org.erikreider.swaysettings-wallpaper"),
                gio::ApplicationFlags::IS_LAUNCHER,
            ));
        }
        if let Some(app) = app_opt.as_ref() {
            if app.is_registered() {
                return true;
            }
            match app.register(None::<&gio::Cancellable>) {
                Ok(_) => true,
                Err(err) => {
                    log::debug!("Failed to register wallpaper application: {err}");
                    false
                }
            }
        } else {
            false
        }
    })
}

pub fn wallpaper_application() -> Option<gio::Application> {
    if wallpaper_application_registered() {
        WALLPAPER_APP.with(|cell| cell.borrow().clone())
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleMode {
    Fill = 0,
    Stretch = 1,
    Fit = 2,
    Center = 3,
}

impl ScaleMode {
    pub fn to_string(self) -> &'static str {
        match self {
            ScaleMode::Fill => "fill",
            ScaleMode::Stretch => "stretch",
            ScaleMode::Fit => "fit",
            ScaleMode::Center => "center",
        }
    }

    pub fn to_title(self) -> &'static str {
        match self {
            ScaleMode::Fill => "Fill Screen",
            ScaleMode::Stretch => "Stretch Screen",
            ScaleMode::Fit => "Fit Screen",
            ScaleMode::Center => "Center Screen",
        }
    }

    pub fn to_content_fit(self) -> gtk4::ContentFit {
        match self {
            ScaleMode::Fill => gtk4::ContentFit::Fill,
            ScaleMode::Stretch => gtk4::ContentFit::Cover,
            ScaleMode::Fit => gtk4::ContentFit::Contain,
            ScaleMode::Center => gtk4::ContentFit::ScaleDown,
        }
    }

    pub fn parse_mode(value: Option<&str>) -> ScaleMode {
        match value.unwrap_or("fill") {
            "stretch" => ScaleMode::Stretch,
            "fit" => ScaleMode::Fit,
            "center" => ScaleMode::Center,
            _ => ScaleMode::Fill,
        }
    }
}

impl TryFrom<i32> for ScaleMode {
    type Error = anyhow::Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => ScaleMode::Stretch,
            2 => ScaleMode::Fit,
            3 => ScaleMode::Center,
            _ => ScaleMode::Fill,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub path: String,
    pub scale_mode: ScaleMode,
    pub color: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            path: String::new(),
            scale_mode: ScaleMode::Fill,
            color: "#FFFFFF".to_string(),
        }
    }
}

impl Config {
    pub fn default_path() -> &'static Path {
        DEFAULT_WALLPAPER_PATH.as_path()
    }

    pub fn to_string(&self) -> String {
        format!("{} {} {}", self.path, self.scale_mode.to_string(), self.color)
    }

    pub fn is_path_valid(&self) -> bool {
        !self.path.is_empty()
    }

    pub fn has_image(&self, cancellable: Option<&gio::Cancellable>) -> Option<gio::File> {
        if !self.is_path_valid() {
            return None;
        }
        let file = gio::File::for_path(&self.path);
        let ftype = file.query_file_type(gio::FileQueryInfoFlags::NONE, cancellable);
        match ftype {
            gio::FileType::Regular | gio::FileType::SymbolicLink | gio::FileType::Shortcut => {
                Some(file)
            }
            _ => None,
        }
    }

    pub fn cmp(&self, other: &Config) -> bool {
        self.path == other.path
            && self.scale_mode as i32 == other.scale_mode as i32
            && self.color == other.color
    }

    pub fn get_color(&self) -> gdk4::RGBA {
        let mut rgba = gdk4::RGBA::new(0.0, 0.0, 0.0, 1.0);
        let mut color = self.color.as_str();
        if !color.starts_with('#') || color.len() != 7 {
            log::warn!("Color not valid: {}. Using #FFFFFF", color);
            color = "#FFFFFF";
        }
        let color = &color[1..];
        if let Ok(value) = u32::from_str_radix(color, 16) {
            let r = ((value >> 16) & 0xFF) as f32 / 255.0;
            let g = ((value >> 8) & 0xFF) as f32 / 255.0;
            let b = (value & 0xFF) as f32 / 255.0;
            rgba.set_red(r);
            rgba.set_green(g);
            rgba.set_blue(b);
            rgba.set_alpha(1.0);
        }
        rgba
    }
}

pub fn get_scale_mode_gschema(settings: &gio::Settings) -> ScaleMode {
    let variant = functions::get_gsetting(
        settings,
        constants::SETTINGS_WALLPAPER_SCALING_MODE,
        VariantTy::INT32,
    );
    if let Some(variant) = variant {
        if let Some(value) = variant.get::<i32>() {
            return ScaleMode::try_from(value).unwrap_or(ScaleMode::Fill);
        }
    }
    ScaleMode::Fill
}

pub fn get_wallpaper_gschema(settings: &gio::Settings) -> Option<String> {
    let variant =
        functions::get_gsetting(settings, constants::SETTINGS_WALLPAPER_PATH, VariantTy::STRING);
    variant.and_then(|v| v.get::<String>())
}
