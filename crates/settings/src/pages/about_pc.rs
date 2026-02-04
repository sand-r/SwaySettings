use std::collections::HashSet;
use std::ffi::CStr;

use gio::prelude::*;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;
use once_cell::sync::Lazy;
use regex::Regex;

use udisks2::block::BlockProxy;

use super::storage_row::StorageRow;

static CLEAN_RULES: Lazy<Vec<CleanRule>> = Lazy::new(|| {
    vec![
        CleanRule::Replace(Regex::new(r"(\d+\.\d+.\d.-\d+).*").unwrap(), "$1"),
        CleanRule::Replace(Regex::new(r"Mesa DRI ").unwrap(), ""),
        CleanRule::Replace(Regex::new(r"Mesa (.*)").unwrap(), "$1"),
        CleanRule::Replace(Regex::new(r"\(R\)").unwrap(), "®"),
        CleanRule::Replace(Regex::new(r"\(TM\)").unwrap(), "™"),
        CleanRule::Replace(Regex::new(r"Gallium .* on (AMD .*)").unwrap(), "$1"),
        CleanRule::Replace(Regex::new(r"^(AMD .*) \(.*").unwrap(), "$1"),
        CleanRule::Replace(Regex::new(r"^(AMD Ryzen) (.*)").unwrap(), "$1 $2"),
        CleanRule::Lowercase(Regex::new(r"^(AMD [A-Z])(.*)").unwrap()),
        CleanRule::Replace(
            Regex::new(r"^Advanced Micro Devices, Inc\. \[.*?\] .*? \[(.*?)\] .*").unwrap(),
            "AMD $1",
        ),
        CleanRule::Replace(
            Regex::new(r"^Advanced Micro Devices, Inc\. \[.*?\] (.*)").unwrap(),
            "AMD $1",
        ),
        CleanRule::Replace(Regex::new(r"Graphics Controller").unwrap(), "Graphics"),
        CleanRule::Replace(Regex::new(r"Intel Corporation").unwrap(), "Intel"),
        CleanRule::Replace(
            Regex::new(r"NVIDIA Corporation (.*) \[(\S*) (\S*) (.*)\]").unwrap(),
            "NVIDIA® $2 $3 $4",
        ),
    ]
});

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/AboutPCContent.ui")]
pub struct AboutPcContentImpl {
    #[template_child(id = "os_image")]
    pub os_image: TemplateChild<gtk4::Image>,
    #[template_child(id = "os_name_label")]
    pub os_name_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "os_version_label")]
    pub os_version_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "kernel_label")]
    pub kernel_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "cpu_label")]
    pub cpu_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "mem_label")]
    pub mem_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "graphics_label")]
    pub graphics_label: TemplateChild<gtk4::Label>,

    #[template_child(id = "storage_group")]
    pub storage_group: TemplateChild<libadwaita::PreferencesGroup>,
    #[template_child(id = "storage_list_box")]
    pub storage_list_box: TemplateChild<gtk4::ListBox>,
    #[template_child(id = "external_storage_group")]
    pub external_storage_group: TemplateChild<libadwaita::PreferencesGroup>,
    #[template_child(id = "external_storage_list_box")]
    pub external_storage_list_box: TemplateChild<gtk4::ListBox>,
}

#[glib::object_subclass]
impl ObjectSubclass for AboutPcContentImpl {
    const NAME: &'static str = "SwaySettingsAboutPCContent";
    type Type = AboutPcContent;
    type ParentType = libadwaita::Bin;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for AboutPcContentImpl {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        obj.setup();
    }
}
impl WidgetImpl for AboutPcContentImpl {}
impl BinImpl for AboutPcContentImpl {}

glib::wrapper! {
    pub struct AboutPcContent(ObjectSubclass<AboutPcContentImpl>)
        @extends libadwaita::Bin, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl AboutPcContent {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn setup(&self) {
        let imp = self.imp();

        imp.storage_list_box.set_sort_func(|row1, row2| {
            let row1 = row1
                .downcast_ref::<StorageRow>()
                .expect("StorageRow expected");
            let row2 = row2
                .downcast_ref::<StorageRow>()
                .expect("StorageRow expected");

            let order = row1
                .sorting_priority()
                .cmp(&row2.sorting_priority())
                .then_with(|| row1.drive_name().cmp(&row2.drive_name()));
            gtk_ordering(order)
        });

        imp.external_storage_list_box.set_sort_func(|row1, row2| {
            let row1 = row1
                .downcast_ref::<StorageRow>()
                .expect("StorageRow expected");
            let row2 = row2
                .downcast_ref::<StorageRow>()
                .expect("StorageRow expected");
            gtk_ordering(row1.drive_name().cmp(&row2.drive_name()))
        });

        self.load_os_info();
        self.load_gpu_info();
        self.load_storage_devices();
    }

    fn load_os_info(&self) {
        let imp = self.imp();

        let logo = glib::os_info("LOGO")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "item-missing-symbolic".to_string());
        imp.os_image.set_icon_name(Some(&logo));

        let os_name = glib::os_info("NAME")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        imp.os_name_label.set_text(&os_name);

        let version = glib::os_info("VERSION")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        imp.os_version_label.set_text(&version);

        let kernel_version = get_kernel_version().unwrap_or_else(|| "Unknown".to_string());
        imp.kernel_label.set_text(&kernel_version);

        let cpu_info = get_cpu_string().unwrap_or_else(|| "Unknown".to_string());
        imp.cpu_label.set_text(&cpu_info);

        let memory = get_mem_string().unwrap_or_else(|| "Unknown".to_string());
        imp.mem_label.set_text(&memory);

        imp.graphics_label.set_text("Unknown");
    }

    fn load_gpu_info(&self) {
        let graphics_label = self.imp().graphics_label.get();
        glib::MainContext::default().spawn_local(async move {
            if let Some(info) = get_gpu_info().await {
                graphics_label.set_text(&info);
            }
        });
    }

    fn load_storage_devices(&self) {
        let storage_list_box = self.imp().storage_list_box.get();
        let external_list_box = self.imp().external_storage_list_box.get();
        let storage_group = self.imp().storage_group.get();
        let external_group = self.imp().external_storage_group.get();

        glib::MainContext::default().spawn_local(async move {
            let client = match udisks2::Client::new().await {
                Ok(client) => client,
                Err(err) => {
                    log::warn!("Failed to connect to UDisks2: {err}");
                    return;
                }
            };

            let objects = match client.object_manager().get_managed_objects().await {
                Ok(objects) => objects,
                Err(err) => {
                    log::warn!("Failed to enumerate storage devices: {err}");
                    return;
                }
            };

            let mut has_internal = false;
            let mut has_external = false;

            for (path, _) in objects {
                let object = match client.object(path) {
                    Ok(object) => object,
                    Err(_) => continue,
                };

                let block = match object.block().await {
                    Ok(block) => block,
                    Err(_) => continue,
                };

                if block.hint_ignore().await.unwrap_or(true) {
                    continue;
                }

                let filesystem = match object.filesystem().await {
                    Ok(filesystem) => filesystem,
                    Err(_) => continue,
                };

                if !should_show_boot_partition(&block, &filesystem).await {
                    continue;
                }

                let Some(row) = StorageRow::from_udisks(&client, &block, &filesystem).await else {
                    continue;
                };

                if row.removable() {
                    external_list_box.append(&row);
                    has_external = true;
                } else {
                    storage_list_box.append(&row);
                    has_internal = true;
                }
            }

            storage_group.set_visible(has_internal);
            external_group.set_visible(has_external);
        });
    }
}

pub fn build_page() -> gtk4::Widget {
    AboutPcContent::new().upcast()
}

fn gtk_ordering(order: std::cmp::Ordering) -> gtk4::Ordering {
    match order {
        std::cmp::Ordering::Less => gtk4::Ordering::Smaller,
        std::cmp::Ordering::Equal => gtk4::Ordering::Equal,
        std::cmp::Ordering::Greater => gtk4::Ordering::Larger,
    }
}

async fn should_show_boot_partition(
    block: &BlockProxy<'_>,
    filesystem: &udisks2::filesystem::FilesystemProxy<'_>,
) -> bool {
    let mut gvfs_show = true;
    if let Ok(config) = block.configuration().await {
        if let Some((_, details)) = config.first() {
            gvfs_show = false;
            if let Some(value) = details.get("opts") {
                if let Ok(value) = value.try_clone() {
                    if let Ok(bytes) = Vec::<u8>::try_from(value) {
                        let opts = String::from_utf8_lossy(&bytes)
                            .trim_end_matches('\0')
                            .to_string();
                        gvfs_show = opts.contains("x-gvfs-show");
                    }
                }
            }
        }
    }

    let mount_points = filesystem.mount_points().await.unwrap_or_default();
    let is_boot_partition = mount_points
        .iter()
        .filter_map(|bytes| bytes_to_string(bytes))
        .any(|path| is_boot_mount(&path));

    gvfs_show || !is_boot_partition
}

fn is_boot_mount(path: &str) -> bool {
    path.starts_with("/boot") || path.starts_with("/efi")
}

fn bytes_to_string(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let value = String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

async fn get_gpu_info() -> Option<String> {
    let proxy = gio::DBusProxy::for_bus_future(
        gio::BusType::System,
        gio::DBusProxyFlags::NONE,
        None::<&gio::DBusInterfaceInfo>,
        "net.hadess.SwitcherooControl",
        "/net/hadess/SwitcherooControl",
        "net.hadess.SwitcherooControl",
    )
    .await
    .ok()?;

    let gpus = proxy.cached_property("GPUs")?;

    let mut names = Vec::new();
    for gpu in gpus.iter() {
        let dict = glib::VariantDict::new(Some(&gpu));
        if let Ok(Some(name)) = dict.lookup::<String>("Name") {
            names.push(clean_name(&name));
        }
    }

    if names.is_empty() {
        None
    } else {
        Some(names.join("\n\t\t"))
    }
}

fn get_kernel_version() -> Option<String> {
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::uname(&mut uts) };
    if result != 0 {
        return None;
    }

    let sysname = unsafe { CStr::from_ptr(uts.sysname.as_ptr()) }
        .to_string_lossy()
        .to_string();
    let release = unsafe { CStr::from_ptr(uts.release.as_ptr()) }
        .to_string_lossy()
        .to_string();

    Some(format!("{} {}", sysname, clean_name(&release)))
}

fn get_cpu_string() -> Option<String> {
    let data = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut cpus = HashSet::new();

    for line in data.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            if matches!(key, "model name" | "cpu" | "Processor") {
                let value = clean_name(value.trim());
                if !value.is_empty() {
                    cpus.insert(value);
                }
            }
        }
    }

    if cpus.is_empty() {
        None
    } else {
        let mut values: Vec<String> = cpus.into_iter().collect();
        values.sort();
        Some(values.join("\n\t\t"))
    }
}

fn get_mem_string() -> Option<String> {
    let data = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in data.lines() {
        if line.starts_with("MemTotal:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(kb) = parts[1].parse::<u64>() {
                    let bytes = kb.saturating_mul(1024);
                    return Some(
                        glib::format_size_full(bytes, glib::FormatSizeFlags::IEC_UNITS).to_string(),
                    );
                }
            }
        }
    }
    None
}

fn clean_name(info: &str) -> String {
    let mut pretty = glib::markup_escape_text(info).trim().to_string();
    for rule in CLEAN_RULES.iter() {
        if let Some(value) = rule.apply(&pretty) {
            pretty = value;
            break;
        }
    }
    pretty
}

enum CleanRule {
    Replace(Regex, &'static str),
    Lowercase(Regex),
}

impl CleanRule {
    fn apply(&self, input: &str) -> Option<String> {
        match self {
            CleanRule::Replace(regex, replacement) => {
                if regex.is_match(input) {
                    Some(regex.replace_all(input, *replacement).to_string())
                } else {
                    None
                }
            }
            CleanRule::Lowercase(regex) => {
                let caps = regex.captures(input)?;
                let head = caps.get(1)?.as_str();
                let tail = caps.get(2)?.as_str();
                Some(format!("{}{}", head, tail.to_lowercase()))
            }
        }
    }
}
