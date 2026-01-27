use gio::prelude::*;

fn main() {
    env_logger::init();

    let app = gio::Application::new(
        Some("org.erikreider.swaysettings-upower-monitor"),
        gio::ApplicationFlags::IS_SERVICE,
    );

    let settings = gio::Settings::new("org.erikreider.swaysettings");
    settings.connect_changed(Some("power-auto-power-saver"), |settings, key| {
        let value = settings.boolean(key);
        log::info!("power-auto-power-saver changed: {value}");
    });

    app.connect_startup(|app| {
        let _ = app.hold();
    });

    app.run();
}
