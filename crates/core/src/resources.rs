use once_cell::sync::OnceCell;

static RESOURCES: OnceCell<()> = OnceCell::new();

pub fn init_resources() {
    RESOURCES.get_or_init(|| {
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/swaysettings.gresource"));
        let resource = gio::Resource::from_data(&glib::Bytes::from_static(bytes))
            .expect("Failed to load gresource");
        gio::resources_register(&resource);
    });
}

pub fn load_css(resource_path: &str, priority: u32) {
    init_resources();
    let provider = gtk4::CssProvider::new();
    provider.load_from_resource(resource_path);
    gtk4::style_context_add_provider_for_display(
        &gdk4::Display::default().expect("display"),
        &provider,
        priority,
    );
}
