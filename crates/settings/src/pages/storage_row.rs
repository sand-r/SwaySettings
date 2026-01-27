use std::cell::{Cell, RefCell};
use std::ffi::CString;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{CompositeTemplate, TemplateChild};
use libadwaita::subclass::prelude::*;
use once_cell::sync::Lazy;
use regex::Regex;

use udisks2::{block::BlockProxy, filesystem::FilesystemProxy, Client};

const DEVICE_ICON_DRIVE: &str = "drive-harddisk";
const DEVICE_ICON_REMOVABLE_DRIVE: &str = "drive-removable-media";
const DEVICE_ICON_REMOVABLE_FLASH: &str = "media-flash";
const DEVICE_ICON_REMOVABLE_FLOPPY: &str = "media-floppy";
const DEVICE_ICON_OPTICAL: &str = "media-optical";

static PRIORITY_REGEXES: Lazy<Vec<(Regex, i32)>> = Lazy::new(|| {
    vec![
        (Regex::new(r"^/(usr|data|var).*").unwrap(), 2),
        (Regex::new(r"^/mnt/.*").unwrap(), 3),
        (Regex::new(r"^/run/.*").unwrap(), 4),
    ]
});

#[derive(Default, CompositeTemplate)]
#[template(resource = "/org/erikreider/swaysettings/ui/StorageRow.ui")]
pub struct StorageRowImpl {
    #[template_child(id = "icon")]
    pub icon: TemplateChild<gtk4::Image>,
    #[template_child(id = "type_label")]
    pub type_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "name_label")]
    pub name_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "size_label")]
    pub size_label: TemplateChild<gtk4::Label>,
    #[template_child(id = "browse_button")]
    pub browse_button: TemplateChild<gtk4::Button>,
    #[template_child(id = "usage_bar")]
    pub usage_bar: TemplateChild<gtk4::ProgressBar>,

    sorting_priority: Cell<i32>,
    removable: Cell<bool>,
    drive_name: RefCell<String>,
}

#[glib::object_subclass]
impl ObjectSubclass for StorageRowImpl {
    const NAME: &'static str = "SwaySettingsStorageRow";
    type Type = StorageRow;
    type ParentType = libadwaita::PreferencesRow;

    fn class_init(klass: &mut Self::Class) {
        Self::bind_template(klass);
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for StorageRowImpl {}
impl WidgetImpl for StorageRowImpl {}
impl ListBoxRowImpl for StorageRowImpl {}
impl PreferencesRowImpl for StorageRowImpl {}

glib::wrapper! {
    pub struct StorageRow(ObjectSubclass<StorageRowImpl>)
        @extends libadwaita::PreferencesRow, gtk4::ListBoxRow, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Actionable;
}

impl StorageRow {
    pub async fn from_udisks(
        client: &Client,
        block: &BlockProxy<'_>,
        filesystem: &FilesystemProxy<'_>,
    ) -> Option<Self> {
        let data = StorageRowData::from_udisks(client, block, filesystem).await?;
        Some(Self::new_from_data(data))
    }

    pub fn sorting_priority(&self) -> i32 {
        self.imp().sorting_priority.get()
    }

    pub fn removable(&self) -> bool {
        self.imp().removable.get()
    }

    pub fn drive_name(&self) -> String {
        self.imp().drive_name.borrow().clone()
    }

    fn new_from_data(data: StorageRowData) -> Self {
        let row: StorageRow = glib::Object::builder().build();
        let imp = row.imp();

        imp.icon.set_from_icon_name(Some(&data.icon));
        imp.type_label.set_text(&data.type_label);
        imp.name_label.set_text(&data.name_label);
        imp.size_label.set_text(&data.size_label);
        imp.usage_bar.set_fraction(data.usage_fraction);

        row.set_sensitive(data.mounted);
        imp.browse_button.set_sensitive(data.mounted);

        imp.sorting_priority.set(data.sorting_priority);
        imp.removable.set(data.removable);
        *imp.drive_name.borrow_mut() = data.drive_name;

        if let Some(path) = data.mount_path {
            let file = gio::File::for_path(path);
            imp.browse_button.connect_clicked(move |_| {
                let launcher = gtk4::FileLauncher::new(Some(&file));
                launcher.launch(None::<&gtk4::Window>, None::<&gio::Cancellable>, |_| {});
            });
        }

        row
    }
}

struct StorageRowData {
    icon: String,
    type_label: String,
    name_label: String,
    size_label: String,
    usage_fraction: f64,
    mounted: bool,
    removable: bool,
    sorting_priority: i32,
    drive_name: String,
    mount_path: Option<String>,
}

impl StorageRowData {
    async fn from_udisks(
        client: &Client,
        block: &BlockProxy<'_>,
        filesystem: &FilesystemProxy<'_>,
    ) -> Option<Self> {
        let total_size = block.size().await.unwrap_or(0);
        let total_str = glib::format_size_full(total_size, glib::FormatSizeFlags::IEC_UNITS)
            .to_string();

        let mut removable = false;
        let mut icon = DEVICE_ICON_DRIVE.to_string();

        if !block.hint_system().await.unwrap_or(true) {
            removable = true;
            icon = DEVICE_ICON_REMOVABLE_DRIVE.to_string();
        }

        if let Ok(drive) = client.drive_for_block(block).await {
            if drive.media_removable().await.unwrap_or(false) || drive.removable().await.unwrap_or(false)
            {
                removable = true;
                icon = DEVICE_ICON_REMOVABLE_DRIVE.to_string();
            }

            if let Ok(media) = drive.media().await {
                if let Some(media_icon) = icon_from_media(media) {
                    icon = media_icon;
                }
            }
        }

        let type_label = block
            .id_type()
            .await
            .unwrap_or_default()
            .to_uppercase();
        let type_display = if type_label.is_empty() {
            "Unknown".to_string()
        } else {
            type_label.clone()
        };

        let mut drive_name = block.id_label().await.unwrap_or_default();
        if drive_name.is_empty() {
            drive_name = format!("{} {} Partition", total_str, type_display);
        }
        if block.read_only().await.unwrap_or(false) {
            drive_name = format!("{} (Read-Only)", drive_name);
        }

        let mount_path = filesystem
            .mount_points()
            .await
            .ok()
            .and_then(|mounts| mounts.first().cloned())
            .and_then(bytes_to_string);

        let mounted = mount_path.is_some();

        let (size_label, usage_fraction) = if let Some(ref mount_path) = mount_path {
            match statvfs(mount_path) {
                Some((total, available)) if total > 0 => {
                    let used = total.saturating_sub(available);
                    let available_str =
                        glib::format_size_full(available, glib::FormatSizeFlags::IEC_UNITS)
                            .to_string();
                    (
                        format!("{} available of {}", available_str, total_str),
                        used as f64 / total as f64,
                    )
                }
                _ => ("Not Mounted".to_string(), 0.0),
            }
        } else {
            ("Not Mounted".to_string(), 0.0)
        };

        let sorting_priority = compute_sorting_priority(removable, mount_path.as_deref());

        Some(Self {
            icon,
            type_label,
            name_label: drive_name.clone(),
            size_label,
            usage_fraction,
            mounted,
            removable,
            sorting_priority,
            drive_name,
            mount_path,
        })
    }
}

fn bytes_to_string(bytes: Vec<u8>) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let value = String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn statvfs(path: &str) -> Option<(u64, u64)> {
    let c_path = CString::new(path).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if result != 0 {
        return None;
    }
    let total = stat.f_blocks as u64 * stat.f_frsize as u64;
    let available = stat.f_bfree as u64 * stat.f_frsize as u64;
    Some((total, available))
}

fn compute_sorting_priority(removable: bool, mount_point: Option<&str>) -> i32 {
    if removable {
        return -1;
    }
    let Some(mount_point) = mount_point else {
        return i32::MAX;
    };

    match mount_point {
        "/" => return 0,
        "/home" => return 1,
        "/usr" | "/data" => return 2,
        "/var" => return 3,
        _ => {}
    }

    for (regex, priority) in PRIORITY_REGEXES.iter() {
        if regex.is_match(mount_point) {
            return *priority;
        }
    }

    i32::MAX
}

fn icon_from_media(media: udisks2::drive::MediaCompatibility) -> Option<String> {
    use udisks2::drive::MediaCompatibility as Media;

    let icon = match media {
        Media::Thumb => DEVICE_ICON_REMOVABLE_DRIVE,
        Media::Flash
        | Media::FlashCf
        | Media::FlashMs
        | Media::FlashSm
        | Media::FlashSd
        | Media::FlashSdhc
        | Media::FlashSdxc
        | Media::FlashSdio
        | Media::FlashSdCombo
        | Media::FlashMmc => DEVICE_ICON_REMOVABLE_FLASH,
        Media::Floppy | Media::FloppyZip | Media::FloppyJaz => DEVICE_ICON_REMOVABLE_FLOPPY,
        Media::Optical
        | Media::OpticalCd
        | Media::OpticalCdR
        | Media::OpticalCdRw
        | Media::OpticalDvd
        | Media::OpticalDvdR
        | Media::OpticalDvdRw
        | Media::OpticalDvdRam
        | Media::OpticalDvdPlusR
        | Media::OpticalDvdPlusRw
        | Media::OpticalDvdPlusRDl
        | Media::OpticalDvdPlusRwDl
        | Media::OpticalBd
        | Media::OpticalBdR
        | Media::OpticalBdRe
        | Media::OpticalHddvd
        | Media::OpticalHddvdR
        | Media::OpticalHddvdRw
        | Media::OpticalMo
        | Media::OpticalMrw
        | Media::OpticalMrwW => DEVICE_ICON_OPTICAL,
        Media::Unknown => return None,
        _ => return None,
    };

    Some(icon.to_string())
}
