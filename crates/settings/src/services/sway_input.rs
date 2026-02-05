use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Pointer,
    Touchpad,
    Keyboard,
}

impl InputKind {
    fn as_sway_type(self) -> &'static str {
        match self {
            InputKind::Pointer => "pointer",
            InputKind::Touchpad => "touchpad",
            InputKind::Keyboard => "keyboard",
        }
    }
}

#[derive(Clone, Debug)]
pub struct InputDeviceSettings {
    pub identifier: String,
    pub send_events: String,
    pub natural_scroll: bool,
    pub tap: bool,
    pub dwt: bool,
    pub scroll_method: String,
    pub click_method: String,
    pub accel_profile: String,
    pub accel_speed: f64,
    pub scroll_factor: f64,
}

impl Default for InputDeviceSettings {
    fn default() -> Self {
        Self {
            identifier: String::new(),
            send_events: "enabled".to_string(),
            natural_scroll: false,
            tap: false,
            dwt: false,
            scroll_method: "two_finger".to_string(),
            click_method: "clickfinger".to_string(),
            accel_profile: "adaptive".to_string(),
            accel_speed: 0.0,
            scroll_factor: 1.0,
        }
    }
}

pub fn load_first_device(kind: InputKind) -> Result<Option<InputDeviceSettings>> {
    let output = run_swaymsg(&["-t", "get_inputs", "-r"])?;
    let root: Value = serde_json::from_slice(&output.stdout)
        .context("Failed to parse swaymsg get_inputs output")?;
    let Some(items) = root.as_array() else {
        return Err(anyhow!("swaymsg get_inputs response is not a JSON array"));
    };

    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };

        let Some(input_type) = obj.get("type").and_then(Value::as_str) else {
            continue;
        };
        if input_type != kind.as_sway_type() {
            continue;
        }

        let Some(libinput) = obj.get("libinput").and_then(Value::as_object) else {
            continue;
        };
        if !libinput.contains_key("accel_speed") {
            continue;
        }

        let mut settings = InputDeviceSettings {
            identifier: obj
                .get("identifier")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            ..InputDeviceSettings::default()
        };

        settings.send_events = get_string(libinput.get("send_events"), "enabled");
        settings.natural_scroll = get_bool(libinput.get("natural_scroll"), false);
        settings.tap = get_bool(libinput.get("tap"), false);
        settings.dwt = get_bool(libinput.get("dwt"), false);
        settings.scroll_method = get_string(libinput.get("scroll_method"), "two_finger");
        settings.click_method = get_string(libinput.get("click_method"), "clickfinger");
        settings.accel_profile = get_string(libinput.get("accel_profile"), "adaptive");
        settings.accel_speed = get_f64(libinput.get("accel_speed"), 0.0);
        settings.scroll_factor = get_f64(obj.get("scroll_factor"), 1.0);

        return Ok(Some(settings));
    }

    Ok(None)
}

pub fn apply_setting(kind: InputKind, setting: &str, value: &str) -> Result<()> {
    let cmd = format!("input type:{} {} {}", kind.as_sway_type(), setting, value);
    run_swaymsg(&["-q", &cmd]).map(|_| ())
}

pub fn load_keyboard_layouts() -> Result<Option<Vec<String>>> {
    let output = run_swaymsg(&["-t", "get_inputs", "-r"])?;
    let root: Value = serde_json::from_slice(&output.stdout)
        .context("Failed to parse swaymsg get_inputs output")?;
    let Some(items) = root.as_array() else {
        return Err(anyhow!("swaymsg get_inputs response is not a JSON array"));
    };

    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };

        let input_type = obj.get("type").and_then(Value::as_str).unwrap_or_default();
        if input_type != InputKind::Keyboard.as_sway_type() {
            continue;
        }

        let Some(layouts) = obj.get("xkb_layout_names").and_then(Value::as_array) else {
            continue;
        };

        let mut values = Vec::new();
        for layout in layouts {
            if let Some(layout) = layout.as_str().filter(|s| !s.is_empty()) {
                values.push(layout.to_string());
            }
        }

        return Ok(Some(values));
    }

    Ok(None)
}

pub fn set_keyboard_layouts(layouts: &[String]) -> Result<()> {
    if layouts.is_empty() {
        return Ok(());
    }

    let joined = layouts.join(",");
    let cmd = format!(
        "input type:{} xkb_layout \"{}\"",
        InputKind::Keyboard.as_sway_type(),
        joined
    );
    run_swaymsg(&["-q", &cmd]).map(|_| ())
}

fn run_swaymsg(args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new("swaymsg")
        .args(args)
        .output()
        .context("Failed to execute swaymsg")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "unknown error".to_string()
        };

        return Err(anyhow!(
            "swaymsg failed (status {}): {}",
            output.status,
            detail
        ));
    }

    Ok(output)
}

fn get_string(value: Option<&Value>, default: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn get_bool(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(v)) => *v,
        Some(Value::String(v)) => matches!(v.as_str(), "enabled" | "true" | "yes" | "1" | "on"),
        _ => default,
    }
}

fn get_f64(value: Option<&Value>, default: f64) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(default)
}
