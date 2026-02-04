use std::collections::HashMap;

use gio::prelude::*;
use zbus::blocking::connection::Builder;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use swaysettings_core::functions;

const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
const ACCENT_COLOR: &str = "accent-color";

struct WallpaperPortal;

#[interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl WallpaperPortal {
    fn set_wallpaper_uri(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        uri: &str,
        _options: HashMap<String, OwnedValue>,
    ) -> u32 {
        let settings = gio::Settings::new("org.erikreider.swaysettings");
        let file = gio::File::for_uri(uri);
        if let Some(path) = file.path() {
            if functions::set_wallpaper(path.to_string_lossy().as_ref(), &settings) {
                return 0;
            }
        }
        1
    }
}

struct SettingsPortal;

#[interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsPortal {
    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }

    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut table = HashMap::new();
        if namespaces_match(APPEARANCE_NAMESPACE, &namespaces) {
            let mut appearance = HashMap::new();
            appearance.insert(ACCENT_COLOR.to_string(), accent_color_value());
            table.insert(APPEARANCE_NAMESPACE.to_string(), appearance);
        }
        table
    }

    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if namespace == APPEARANCE_NAMESPACE && key == ACCENT_COLOR {
            return Ok(accent_color_value());
        }
        Err(zbus::fdo::Error::Failed(
            "Requested setting not found".into(),
        ))
    }
}

fn namespaces_match(target: &str, namespaces: &[String]) -> bool {
    if namespaces.is_empty() {
        return true;
    }
    namespaces.iter().any(|ns| ns.is_empty() || ns == target)
}

fn accent_color_value() -> OwnedValue {
    let settings = gio::Settings::new("org.gnome.desktop.interface");
    let rgba = functions::get_accent_color(Some(&settings));
    OwnedValue::try_from(Value::from((
        rgba.red() as f64,
        rgba.green() as f64,
        rgba.blue() as f64,
    )))
    .unwrap_or_else(|_| OwnedValue::from(0.0f64))
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    std::env::remove_var("GTK_USE_PORTAL");

    let connection = Builder::session()?
        .serve_at("/org/freedesktop/portal/desktop", WallpaperPortal)?
        .serve_at("/org/freedesktop/portal/desktop", SettingsPortal)?
        .build()?;

    connection.request_name("org.freedesktop.impl.portal.desktop.swaysettings")?;

    loop {
        std::thread::park();
    }
}
