mod audio_device;
mod content;

use content::SoundContent;
use gtk4::prelude::*;

pub fn build_page() -> gtk4::Widget {
    SoundContent::new().upcast()
}
