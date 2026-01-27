use gio::prelude::*;

fn main() -> anyhow::Result<()> {
    let mut autostart_path = glib::user_config_dir();
    autostart_path.push("autostart");

    let directory = gio::File::for_path(&autostart_path);
    if !directory.query_exists(None::<&gio::Cancellable>) {
        eprintln!("ERROR: Couldn't find path {}", autostart_path.display());
        return Ok(());
    }

    let enumerator = directory.enumerate_children(
        gio::FILE_ATTRIBUTE_STANDARD_NAME,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    )?;

    while let Some(info) = enumerator.next_file(None::<&gio::Cancellable>)? {
        let file_path = autostart_path.join(info.name());
        if let Some(app) = gio::DesktopAppInfo::from_filename(file_path) {
            if app.is_hidden() {
                continue;
            }
            let _ = app.launch(&[], None::<&gio::AppLaunchContext>);
        }
    }

    Ok(())
}
