use std::cell::RefCell;
use std::rc::Rc;

use clap::Parser;
use gio::prelude::*;
use gtk4::prelude::*;

mod pages;
mod window;


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

    let page_value: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let page_value_action = page_value.clone();

    let action = gio::SimpleAction::new(
        "page",
        Some(&glib::VariantType::new("s").expect("variant type")),
    );
    action.connect_activate(move |_, param| {
        if let Some(param) = param {
            if let Some(value) = param.get::<String>() {
                *page_value_action.borrow_mut() = Some(value);
            }
        }
    });

    app.add_action(&action);

    let settings_clone = settings.clone();
    let page_value_clone = page_value.clone();

    app.connect_activate(move |app| {
        let window_state = window::build_window(app, &settings_clone);

        swaysettings_core::resources::load_css(
            "/org/erikreider/swaysettings/style/settings-window.css",
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        if let Some(display) = gdk4::Display::default() {
            let icon_theme = gtk4::IconTheme::for_display(&display);
            icon_theme.add_resource_path("/org/erikreider/swaysettings/icons");
        }

        window_state.window.present();

        if let Some(page) = page_value_clone.borrow().clone() {
            window_state.navigate_to_page(&page);
        }
    });

    if let Some(page) = cli.page {
        app.activate_action("page", Some(&glib::Variant::from(page.as_str())));
    }

    app.run();
}
