use std::cell::RefCell;
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::*;

mod pages;
mod services;
mod settings_window;
mod sidebar_row;

use settings_window::SettingsWindow;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Navigate to a page by internal name
    #[arg(short, long)]
    page: Option<String>,

    /// List all available pages
    #[arg(short, long)]
    list_pages: bool,
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    if cli.list_pages {
        for page in pages::PageType::all() {
            println!("{}", page.internal_name());
        }
        return;
    }

    gtk4::init().expect("Failed to init GTK");
    let _ = libadwaita::init();

    swaysettings_core::resources::init_resources();

    let settings = gio::Settings::new("org.erikreider.swaysettings");

    let app = libadwaita::Application::new(
        Some("org.erikreider.swaysettings"),
        gio::ApplicationFlags::FLAGS_NONE,
    );

    let initial_page: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(cli.page));

    let settings_clone = settings.clone();
    let initial_page_clone = initial_page.clone();

    app.connect_activate(move |app| {
        let window = SettingsWindow::new(app, &settings_clone);

        swaysettings_core::resources::load_css(
            "/org/erikreider/swaysettings/style/settings-window.css",
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        if let Some(display) = gdk4::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            icon_theme.add_resource_path("/org/erikreider/swaysettings/icons");
        }

        window.present();

        if let Some(page) = initial_page_clone.borrow_mut().take() {
            window.navigate_to_page(&page);
        }
    });

    // Pass empty args so GApplication doesn't try to parse our CLI flags
    app.run_with_args::<String>(&[]);
}
