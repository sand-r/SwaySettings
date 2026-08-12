//! Audio service backed by libpulse (served by pipewire-pulse).
//!
//! Runs entirely on the GLib main loop: no worker thread, no channels. The
//! PulseAudio introspection API provides devices, ports (with availability),
//! card profiles, and per-application streams as first-class objects, so no
//! SPA param parsing or route matching happens here.
//!
//! Sinks and sources (and sink-inputs and source-outputs) live in separate
//! PulseAudio index namespaces, so public ids encode the kind in the lowest
//! bit. The UI treats ids as opaque.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use libpulse_binding as pulse;
use libpulse_glib_binding as pulse_glib;

use pulse::callbacks::ListResult;
use pulse::context::introspect::{
    CardInfo, ServerInfo, SinkInfo, SinkInputInfo, SourceInfo, SourceOutputInfo,
};
use pulse::context::subscribe::{Facility, InterestMaskSet, Operation as SubscribeOp};
use pulse::context::{Context, FlagSet as ContextFlags, State as ContextState};
use pulse::def::{BufferAttr, PortAvailable};
use pulse::proplist::{properties, Proplist};
use pulse::sample::{Format, Spec};
use pulse::stream::{FlagSet as StreamFlags, PeekResult, Stream};
use pulse::volume::{ChannelVolumes, Volume};

const APPLICATION_ID: &str = "org.erikreider.swaysettings";

/// Type of audio device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Sink,
    Source,
}

impl DeviceType {
    pub fn name(&self) -> &'static str {
        match self {
            DeviceType::Sink => "sink",
            DeviceType::Source => "source",
        }
    }
}

/// Information about an audio device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub icon_name: Option<String>,
    pub volume: f64,
    pub is_muted: bool,
    pub is_default: bool,
    pub device_type: DeviceType,
    /// Card index the device belongs to (profile handling)
    pub device_id: Option<u32>,
    /// Unused with the PulseAudio backend; kept for the UI model
    pub profile_device_index: Option<u32>,
}

/// Information about a card profile
#[derive(Debug, Clone)]
pub struct ProfileInfo {
    /// Position within the card's profile list
    pub index: u32,
    pub name: String,
    pub description: String,
    pub priority: u32,
    pub available: bool,
    pub n_sinks: u32,
    pub n_sources: u32,
}

impl ProfileInfo {
    pub fn display_name(&self) -> &str {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn supports_device(&self, device_type: DeviceType, _device_index: Option<u32>) -> bool {
        match device_type {
            DeviceType::Sink => self.n_sinks > 0,
            DeviceType::Source => self.n_sources > 0,
        }
    }
}

/// Information about an application audio stream
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub id: u32,
    pub name: String,
    pub app_name: String,
    pub icon_name: Option<String>,
    pub volume: f64,
    pub is_muted: bool,
    pub is_output: bool,
}

/// Events delivered to the UI callback (on the GLib main loop)
#[derive(Debug)]
pub enum AudioEvent {
    Ready,
    DeviceAdded(DeviceInfo),
    DeviceRemoved(u32),
    DeviceChanged(DeviceInfo),
    DefaultChanged(DeviceType, Option<u32>),
    ProfilesUpdated(u32, Vec<ProfileInfo>, Option<u32>),
    /// Peak level update (device_type, level 0.0-1.0)
    PeakLevel(DeviceType, f32),
    StreamAdded(StreamInfo),
    StreamRemoved(u32),
    StreamChanged(StreamInfo),
    Error(String),
}

/// Convert linear volume (0.0-1.0) to cubic scale for UI display
pub fn linear_to_cubic(v: f64) -> f64 {
    v.cbrt()
}

/// Convert cubic scale (from UI) back to linear volume
pub fn cubic_to_linear(v: f64) -> f64 {
    v.powi(3)
}

// pipewire-pulse maps PulseAudio volume to cubic node volume, so
// pa_volume / PA_VOLUME_NORM already corresponds to the UI's cubic scale.
//
// Read the maximum channel, not the average: writes use ChannelVolumes::scale,
// which is also relative to the maximum. Mixing the two makes an unbalanced
// device lose volume on every UI round trip.
fn channel_volumes_to_linear(cv: &ChannelVolumes) -> f64 {
    let cubic = cv.max().0 as f64 / Volume::NORMAL.0 as f64;
    cubic.powi(3)
}

fn linear_to_volume(linear: f64) -> Volume {
    let cubic = linear.max(0.0).cbrt();
    let raw = (cubic * Volume::NORMAL.0 as f64).round();
    Volume((raw as u32).clamp(Volume::MUTED.0, Volume::MAX.0))
}

const KIND_SINK: u32 = 0;
const KIND_SOURCE: u32 = 1;

fn encode_id(kind: u32, index: u32) -> u32 {
    (index << 1) | kind
}

fn decode_id(id: u32) -> (u32, u32) {
    (id & 1, id >> 1)
}

struct DeviceEntry {
    info: DeviceInfo,
    volume: ChannelVolumes,
    visible: bool,
    /// Sinks only: index of the sink's monitor source (peak metering)
    monitor_source: u32,
}

struct StreamEntry {
    info: StreamInfo,
    volume: ChannelVolumes,
}

struct PeakStream {
    stream: Rc<RefCell<Stream>>,
    /// Pulse index of the device this stream records from
    target: u32,
}

struct Inner {
    context: RefCell<Option<Context>>,
    sinks: RefCell<HashMap<u32, DeviceEntry>>,
    sources: RefCell<HashMap<u32, DeviceEntry>>,
    sink_inputs: RefCell<HashMap<u32, StreamEntry>>,
    source_outputs: RefCell<HashMap<u32, StreamEntry>>,
    /// Card index -> profile list (positions are the public profile indices)
    cards: RefCell<HashMap<u32, Vec<ProfileInfo>>>,
    default_sink: RefCell<Option<String>>,
    default_source: RefCell<Option<String>>,
    metering: Cell<bool>,
    sink_peak: RefCell<Option<PeakStream>>,
    source_peak: RefCell<Option<PeakStream>>,
    callback: Box<dyn Fn(AudioEvent)>,
}

impl Inner {
    fn emit(&self, event: AudioEvent) {
        (self.callback)(event);
    }
}

/// The audio daemon. Create on the GLib main thread; all callbacks are
/// delivered on the same thread.
pub struct AudioDaemon {
    inner: Rc<Inner>,
    _mainloop: pulse_glib::Mainloop,
}

impl AudioDaemon {
    pub fn new<F>(event_callback: F) -> Self
    where
        F: Fn(AudioEvent) + 'static,
    {
        let mainloop =
            pulse_glib::Mainloop::new(None).expect("Failed to create PulseAudio GLib mainloop");

        let mut proplist = Proplist::new().expect("Failed to create proplist");
        let _ = proplist.set_str(properties::APPLICATION_NAME, "SwaySettings");
        let _ = proplist.set_str(properties::APPLICATION_ID, APPLICATION_ID);

        let context = Context::new_with_proplist(&mainloop, "SwaySettings", &proplist)
            .expect("Failed to create PulseAudio context");

        let inner = Rc::new(Inner {
            context: RefCell::new(Some(context)),
            sinks: RefCell::new(HashMap::new()),
            sources: RefCell::new(HashMap::new()),
            sink_inputs: RefCell::new(HashMap::new()),
            source_outputs: RefCell::new(HashMap::new()),
            cards: RefCell::new(HashMap::new()),
            default_sink: RefCell::new(None),
            default_source: RefCell::new(None),
            metering: Cell::new(false),
            sink_peak: RefCell::new(None),
            source_peak: RefCell::new(None),
            callback: Box::new(event_callback),
        });

        {
            let weak = Rc::downgrade(&inner);
            let mut context = inner.context.borrow_mut();
            let context = context.as_mut().unwrap();
            context.set_state_callback(Some(Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    on_state_changed(&inner);
                }
            })));

            if let Err(e) = context.connect(None, ContextFlags::NOFAIL | ContextFlags::NOAUTOSPAWN, None)
            {
                log::error!("Failed to connect to PulseAudio: {e}");
            }
        }

        Self {
            inner,
            _mainloop: mainloop,
        }
    }

    /// Set volume for a device (linear 0.0-1.0). Preserves channel balance.
    pub fn set_volume(&self, id: u32, volume: f64) {
        let (kind, index) = decode_id(id);
        let target = linear_to_volume(volume);

        let devices = if kind == KIND_SINK {
            &self.inner.sinks
        } else {
            &self.inner.sources
        };
        let Some(mut cv) = devices.borrow().get(&index).map(|entry| entry.volume) else {
            log::warn!("set_volume: device {id} not found");
            return;
        };

        // Scaling an all-zero volume can't recover ratios; reset to uniform.
        if cv.max().0 == 0 {
            let channels = cv.len().max(1);
            cv.set(channels, target);
        } else {
            cv.scale(target);
        }

        let mut context = self.inner.context.borrow_mut();
        let Some(context) = context.as_mut() else { return };
        let mut introspect = context.introspect();
        if kind == KIND_SINK {
            introspect.set_sink_volume_by_index(index, &cv, None);
        } else {
            introspect.set_source_volume_by_index(index, &cv, None);
        }
    }

    /// Set mute state for a device
    pub fn set_mute(&self, id: u32, muted: bool) {
        let (kind, index) = decode_id(id);
        let mut context = self.inner.context.borrow_mut();
        let Some(context) = context.as_mut() else { return };
        let mut introspect = context.introspect();
        if kind == KIND_SINK {
            introspect.set_sink_mute_by_index(index, muted, None);
        } else {
            introspect.set_source_mute_by_index(index, muted, None);
        }
    }

    /// Set the default sink (output device)
    pub fn set_default_sink(&self, id: u32) {
        let (_, index) = decode_id(id);
        let Some(name) = self
            .inner
            .sinks
            .borrow()
            .get(&index)
            .map(|entry| entry.info.name.clone())
        else {
            return;
        };
        let mut context = self.inner.context.borrow_mut();
        if let Some(context) = context.as_mut() {
            context.set_default_sink(&name, |_| {});
        }
    }

    /// Set the default source (input device)
    pub fn set_default_source(&self, id: u32) {
        let (_, index) = decode_id(id);
        let Some(name) = self
            .inner
            .sources
            .borrow()
            .get(&index)
            .map(|entry| entry.info.name.clone())
        else {
            return;
        };
        let mut context = self.inner.context.borrow_mut();
        if let Some(context) = context.as_mut() {
            context.set_default_source(&name, |_| {});
        }
    }

    /// Set the active profile on a card
    pub fn set_profile(&self, device_id: u32, profile_index: u32) {
        let Some(name) = self
            .inner
            .cards
            .borrow()
            .get(&device_id)
            .and_then(|profiles| profiles.get(profile_index as usize))
            .map(|profile| profile.name.clone())
        else {
            log::warn!("set_profile: card {device_id} profile {profile_index} not found");
            return;
        };
        let mut context = self.inner.context.borrow_mut();
        let Some(context) = context.as_mut() else { return };
        context
            .introspect()
            .set_card_profile_by_index(device_id, &name, None);
    }

    /// Set volume for an application stream (linear 0.0-1.0)
    pub fn set_stream_volume(&self, id: u32, volume: f64) {
        let (kind, index) = decode_id(id);
        let target = linear_to_volume(volume);

        let streams = if kind == KIND_SINK {
            &self.inner.sink_inputs
        } else {
            &self.inner.source_outputs
        };
        let Some(mut cv) = streams.borrow().get(&index).map(|entry| entry.volume) else {
            log::warn!("set_stream_volume: stream {id} not found");
            return;
        };

        if cv.max().0 == 0 {
            let channels = cv.len().max(1);
            cv.set(channels, target);
        } else {
            cv.scale(target);
        }

        let mut context = self.inner.context.borrow_mut();
        let Some(context) = context.as_mut() else { return };
        let mut introspect = context.introspect();
        if kind == KIND_SINK {
            introspect.set_sink_input_volume(index, &cv, None);
        } else {
            introspect.set_source_output_volume(index, &cv, None);
        }
    }

    /// Set mute state for an application stream
    pub fn set_stream_mute(&self, id: u32, muted: bool) {
        let (kind, index) = decode_id(id);
        let mut context = self.inner.context.borrow_mut();
        let Some(context) = context.as_mut() else { return };
        let mut introspect = context.introspect();
        if kind == KIND_SINK {
            introspect.set_sink_input_mute(index, muted, None);
        } else {
            introspect.set_source_output_mute(index, muted, None);
        }
    }

    /// Enable or disable peak metering. While enabled, capture streams follow
    /// the default sink monitor and default source. Call with `false` when the
    /// page is unmapped so no capture stream (microphone!) stays open.
    pub fn set_metering(&self, enabled: bool) {
        if self.inner.metering.replace(enabled) == enabled {
            return;
        }
        if enabled {
            refresh_peak_streams(&self.inner);
        } else {
            stop_peak_stream(&self.inner.sink_peak);
            stop_peak_stream(&self.inner.source_peak);
            self.inner.emit(AudioEvent::PeakLevel(DeviceType::Sink, 0.0));
            self.inner.emit(AudioEvent::PeakLevel(DeviceType::Source, 0.0));
        }
    }
}

impl Drop for AudioDaemon {
    fn drop(&mut self) {
        self.inner.metering.set(false);
        stop_peak_stream(&self.inner.sink_peak);
        stop_peak_stream(&self.inner.source_peak);
        if let Some(mut context) = self.inner.context.borrow_mut().take() {
            context.disconnect();
        }
    }
}

fn on_state_changed(inner: &Rc<Inner>) {
    // PulseAudio invokes this callback synchronously from inside
    // Context::connect, while the caller still holds the context borrow.
    // Retry on the next main loop iteration rather than panicking through
    // libpulse's C frames.
    let state = match inner.context.try_borrow() {
        Ok(context) => match context.as_ref() {
            Some(context) => context.get_state(),
            None => return,
        },
        Err(_) => {
            let weak = Rc::downgrade(inner);
            glib::idle_add_local_once(move || {
                if let Some(inner) = weak.upgrade() {
                    on_state_changed(&inner);
                }
            });
            return;
        }
    };

    match state {
        ContextState::Ready => on_ready(inner),
        ContextState::Failed => {
            inner.emit(AudioEvent::Error("PulseAudio connection failed".into()));
        }
        ContextState::Terminated => {
            log::debug!("PulseAudio connection terminated");
        }
        _ => {}
    }
}

fn on_ready(inner: &Rc<Inner>) {
    log::info!("Connected to PulseAudio (pipewire-pulse)");

    {
        let weak = Rc::downgrade(inner);
        let mut context = inner.context.borrow_mut();
        let Some(context) = context.as_mut() else { return };

        context.set_subscribe_callback(Some(Box::new(move |facility, operation, index| {
            if let Some(inner) = weak.upgrade() {
                on_subscribe_event(&inner, facility, operation, index);
            }
        })));

        let mask = InterestMaskSet::SINK
            | InterestMaskSet::SOURCE
            | InterestMaskSet::SINK_INPUT
            | InterestMaskSet::SOURCE_OUTPUT
            | InterestMaskSet::CARD
            | InterestMaskSet::SERVER;
        context.subscribe(mask, |_| {});
    }

    inner.emit(AudioEvent::Ready);

    query_server_info(inner);

    let introspect = {
        let context = inner.context.borrow();
        match context.as_ref() {
            Some(context) => context.introspect(),
            None => return,
        }
    };

    let weak = Rc::downgrade(inner);
    introspect.get_sink_info_list(move |result| {
        if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
            apply_sink(&inner, info);
        }
    });

    let weak = Rc::downgrade(inner);
    introspect.get_source_info_list(move |result| {
        if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
            apply_source(&inner, info);
        }
    });

    let weak = Rc::downgrade(inner);
    introspect.get_card_info_list(move |result| {
        if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
            apply_card(&inner, info);
        }
    });

    let weak = Rc::downgrade(inner);
    introspect.get_sink_input_info_list(move |result| {
        if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
            apply_sink_input(&inner, info);
        }
    });

    let weak = Rc::downgrade(inner);
    introspect.get_source_output_info_list(move |result| {
        if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
            apply_source_output(&inner, info);
        }
    });
}

fn on_subscribe_event(
    inner: &Rc<Inner>,
    facility: Option<Facility>,
    operation: Option<SubscribeOp>,
    index: u32,
) {
    let Some(facility) = facility else { return };
    let Some(operation) = operation else { return };

    match (facility, operation) {
        (Facility::Server, _) => query_server_info(inner),
        (Facility::Sink, SubscribeOp::Removed) => {
            remove_device(inner, DeviceType::Sink, index);
        }
        (Facility::Sink, _) => {
            let weak = Rc::downgrade(inner);
            with_introspect(inner, |introspect| {
                introspect.get_sink_info_by_index(index, move |result| {
                    if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
                        apply_sink(&inner, info);
                    }
                });
            });
        }
        (Facility::Source, SubscribeOp::Removed) => {
            remove_device(inner, DeviceType::Source, index);
        }
        (Facility::Source, _) => {
            let weak = Rc::downgrade(inner);
            with_introspect(inner, |introspect| {
                introspect.get_source_info_by_index(index, move |result| {
                    if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
                        apply_source(&inner, info);
                    }
                });
            });
        }
        (Facility::Card, SubscribeOp::Removed) => {
            if inner.cards.borrow_mut().remove(&index).is_some() {
                inner.emit(AudioEvent::ProfilesUpdated(index, Vec::new(), None));
            }
        }
        (Facility::Card, _) => {
            let weak = Rc::downgrade(inner);
            with_introspect(inner, |introspect| {
                introspect.get_card_info_by_index(index, move |result| {
                    if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
                        apply_card(&inner, info);
                    }
                });
            });
        }
        (Facility::SinkInput, SubscribeOp::Removed) => {
            remove_stream(inner, true, index);
        }
        (Facility::SinkInput, _) => {
            let weak = Rc::downgrade(inner);
            with_introspect(inner, |introspect| {
                introspect.get_sink_input_info(index, move |result| {
                    if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
                        apply_sink_input(&inner, info);
                    }
                });
            });
        }
        (Facility::SourceOutput, SubscribeOp::Removed) => {
            remove_stream(inner, false, index);
        }
        (Facility::SourceOutput, _) => {
            let weak = Rc::downgrade(inner);
            with_introspect(inner, |introspect| {
                introspect.get_source_output_info(index, move |result| {
                    if let (Some(inner), ListResult::Item(info)) = (weak.upgrade(), result) {
                        apply_source_output(&inner, info);
                    }
                });
            });
        }
        _ => {}
    }
}

fn with_introspect(
    inner: &Rc<Inner>,
    f: impl FnOnce(&mut pulse::context::introspect::Introspector),
) {
    // Take an owned Introspector and drop the borrow before running `f`, so a
    // callback that reaches back into the daemon can never hit a live borrow.
    let introspect = {
        let context = inner.context.borrow();
        context.as_ref().map(|context| context.introspect())
    };
    if let Some(mut introspect) = introspect {
        f(&mut introspect);
    }
}

fn query_server_info(inner: &Rc<Inner>) {
    let weak = Rc::downgrade(inner);
    with_introspect(inner, |introspect| {
        introspect.get_server_info(move |info| {
            if let Some(inner) = weak.upgrade() {
                apply_server_info(&inner, info);
            }
        });
    });
}

fn apply_server_info(inner: &Rc<Inner>, info: &ServerInfo) {
    let new_sink = info.default_sink_name.as_ref().map(|n| n.to_string());
    let new_source = info.default_source_name.as_ref().map(|n| n.to_string());

    if *inner.default_sink.borrow() != new_sink {
        *inner.default_sink.borrow_mut() = new_sink.clone();
        let default_id = update_default_flags(&inner.sinks, new_sink.as_deref(), KIND_SINK);
        inner.emit(AudioEvent::DefaultChanged(DeviceType::Sink, default_id));
        if inner.metering.get() {
            refresh_peak_streams(inner);
        }
    }

    if *inner.default_source.borrow() != new_source {
        *inner.default_source.borrow_mut() = new_source.clone();
        let default_id = update_default_flags(&inner.sources, new_source.as_deref(), KIND_SOURCE);
        inner.emit(AudioEvent::DefaultChanged(DeviceType::Source, default_id));
        if inner.metering.get() {
            refresh_peak_streams(inner);
        }
    }
}

fn update_default_flags(
    devices: &RefCell<HashMap<u32, DeviceEntry>>,
    default_name: Option<&str>,
    kind: u32,
) -> Option<u32> {
    let mut default_id = None;
    let mut devices = devices.borrow_mut();
    for (index, entry) in devices.iter_mut() {
        let is_default = Some(entry.info.name.as_str()) == default_name;
        entry.info.is_default = is_default;
        if is_default {
            default_id = Some(encode_id(kind, *index));
        }
    }
    default_id
}

fn proplist_str(proplist: &Proplist, key: &str) -> Option<String> {
    proplist.get_str(key).filter(|s| !s.is_empty())
}

fn apply_sink(inner: &Rc<Inner>, info: &SinkInfo) {
    let name = match info.name.as_ref() {
        Some(name) => name.to_string(),
        None => return,
    };
    let visible = info
        .active_port
        .as_ref()
        .map(|port| port.available != PortAvailable::No)
        .unwrap_or(true);

    let device_info = DeviceInfo {
        id: encode_id(KIND_SINK, info.index),
        name: name.clone(),
        description: info
            .description
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_else(|| name.clone()),
        icon_name: proplist_str(&info.proplist, properties::DEVICE_ICON_NAME),
        volume: channel_volumes_to_linear(&info.volume),
        is_muted: info.mute,
        is_default: inner.default_sink.borrow().as_deref() == Some(name.as_str()),
        device_type: DeviceType::Sink,
        device_id: info.card,
        profile_device_index: None,
    };

    let entry = DeviceEntry {
        info: device_info.clone(),
        volume: info.volume,
        visible,
        monitor_source: info.monitor_source,
    };

    // Server info can arrive before the device list, so DefaultChanged may
    // have fired without an id. Re-announce when the default device appears.
    let became_default = device_info.is_default
        && visible
        && !inner
            .sinks
            .borrow()
            .get(&info.index)
            .map(|old| old.info.is_default && old.visible)
            .unwrap_or(false);

    let events = upsert_device(&inner.sinks, info.index, entry);
    for event in events {
        inner.emit(event);
    }
    if became_default {
        inner.emit(AudioEvent::DefaultChanged(
            DeviceType::Sink,
            Some(device_info.id),
        ));
    }

    if inner.metering.get() {
        refresh_peak_streams(inner);
    }
}

fn apply_source(inner: &Rc<Inner>, info: &SourceInfo) {
    // Hide sink monitors; they are not real capture devices.
    if info.monitor_of_sink.is_some() {
        return;
    }

    let name = match info.name.as_ref() {
        Some(name) => name.to_string(),
        None => return,
    };
    let visible = info
        .active_port
        .as_ref()
        .map(|port| port.available != PortAvailable::No)
        .unwrap_or(true);

    let device_info = DeviceInfo {
        id: encode_id(KIND_SOURCE, info.index),
        name: name.clone(),
        description: info
            .description
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_else(|| name.clone()),
        icon_name: proplist_str(&info.proplist, properties::DEVICE_ICON_NAME),
        volume: channel_volumes_to_linear(&info.volume),
        is_muted: info.mute,
        is_default: inner.default_source.borrow().as_deref() == Some(name.as_str()),
        device_type: DeviceType::Source,
        device_id: info.card,
        profile_device_index: None,
    };

    let entry = DeviceEntry {
        info: device_info.clone(),
        volume: info.volume,
        visible,
        monitor_source: pulse::def::INVALID_INDEX,
    };

    let became_default = device_info.is_default
        && visible
        && !inner
            .sources
            .borrow()
            .get(&info.index)
            .map(|old| old.info.is_default && old.visible)
            .unwrap_or(false);

    let events = upsert_device(&inner.sources, info.index, entry);
    for event in events {
        inner.emit(event);
    }
    if became_default {
        inner.emit(AudioEvent::DefaultChanged(
            DeviceType::Source,
            Some(device_info.id),
        ));
    }

    if inner.metering.get() {
        refresh_peak_streams(inner);
    }
}

/// Insert or update a device entry, translating visibility transitions into
/// added/removed events. Returns the events to emit (after borrows are gone).
fn upsert_device(
    devices: &RefCell<HashMap<u32, DeviceEntry>>,
    index: u32,
    entry: DeviceEntry,
) -> Vec<AudioEvent> {
    let mut events = Vec::new();
    let mut devices = devices.borrow_mut();
    match devices.get(&index) {
        None => {
            if entry.visible {
                events.push(AudioEvent::DeviceAdded(entry.info.clone()));
            }
            devices.insert(index, entry);
        }
        Some(old) => {
            match (old.visible, entry.visible) {
                (false, true) => events.push(AudioEvent::DeviceAdded(entry.info.clone())),
                (true, false) => events.push(AudioEvent::DeviceRemoved(entry.info.id)),
                (true, true) => events.push(AudioEvent::DeviceChanged(entry.info.clone())),
                (false, false) => {}
            }
            devices.insert(index, entry);
        }
    }
    events
}

fn remove_device(inner: &Rc<Inner>, device_type: DeviceType, index: u32) {
    let (devices, kind) = match device_type {
        DeviceType::Sink => (&inner.sinks, KIND_SINK),
        DeviceType::Source => (&inner.sources, KIND_SOURCE),
    };
    let removed_visible = devices
        .borrow_mut()
        .remove(&index)
        .map(|entry| entry.visible)
        .unwrap_or(false);
    if removed_visible {
        inner.emit(AudioEvent::DeviceRemoved(encode_id(kind, index)));
    }
}

fn apply_card(inner: &Rc<Inner>, info: &CardInfo) {
    let profiles: Vec<ProfileInfo> = info
        .profiles
        .iter()
        .enumerate()
        .map(|(position, profile)| ProfileInfo {
            index: position as u32,
            name: profile
                .name
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_default(),
            description: profile
                .description
                .as_ref()
                .map(|d| d.to_string())
                .unwrap_or_default(),
            priority: profile.priority,
            available: profile.available,
            n_sinks: profile.n_sinks,
            n_sources: profile.n_sources,
        })
        .collect();

    let active_index = info.active_profile.as_ref().and_then(|active| {
        let active_name = active.name.as_ref()?;
        profiles
            .iter()
            .find(|profile| profile.name == active_name.as_ref())
            .map(|profile| profile.index)
    });

    inner
        .cards
        .borrow_mut()
        .insert(info.index, profiles.clone());
    inner.emit(AudioEvent::ProfilesUpdated(
        info.index,
        profiles,
        active_index,
    ));
}

fn stream_info_from_props(
    proplist: &Proplist,
    name: Option<&str>,
    id: u32,
    volume: f64,
    is_muted: bool,
    is_output: bool,
) -> Option<StreamInfo> {
    // Filter non-application streams, like GNOME Control Center: event sounds,
    // filters, and streams without an owning application.
    if let Some(role) = proplist.get_str(properties::MEDIA_ROLE) {
        if role == "event" || role == "filter" {
            return None;
        }
    }
    // Never list our own peak-detect streams.
    if proplist.get_str(properties::APPLICATION_ID).as_deref() == Some(APPLICATION_ID) {
        return None;
    }

    let app_name = proplist_str(proplist, properties::APPLICATION_NAME)
        .or_else(|| proplist_str(proplist, properties::APPLICATION_PROCESS_BINARY))?;

    Some(StreamInfo {
        id,
        name: name.map(|n| n.to_string()).unwrap_or_else(|| app_name.clone()),
        app_name,
        icon_name: proplist_str(proplist, properties::APPLICATION_ICON_NAME),
        volume,
        is_muted,
        is_output,
    })
}

fn apply_sink_input(inner: &Rc<Inner>, info: &SinkInputInfo) {
    let Some(stream_info) = stream_info_from_props(
        &info.proplist,
        info.name.as_deref(),
        encode_id(KIND_SINK, info.index),
        channel_volumes_to_linear(&info.volume),
        info.mute,
        true,
    ) else {
        return;
    };

    let event = upsert_stream(
        &inner.sink_inputs,
        info.index,
        StreamEntry {
            info: stream_info,
            volume: info.volume,
        },
    );
    inner.emit(event);
}

fn apply_source_output(inner: &Rc<Inner>, info: &SourceOutputInfo) {
    let Some(stream_info) = stream_info_from_props(
        &info.proplist,
        info.name.as_deref(),
        encode_id(KIND_SOURCE, info.index),
        channel_volumes_to_linear(&info.volume),
        info.mute,
        false,
    ) else {
        return;
    };

    let event = upsert_stream(
        &inner.source_outputs,
        info.index,
        StreamEntry {
            info: stream_info,
            volume: info.volume,
        },
    );
    inner.emit(event);
}

fn upsert_stream(
    streams: &RefCell<HashMap<u32, StreamEntry>>,
    index: u32,
    entry: StreamEntry,
) -> AudioEvent {
    let mut streams = streams.borrow_mut();
    let existed = streams.contains_key(&index);
    let info = entry.info.clone();
    streams.insert(index, entry);
    if existed {
        AudioEvent::StreamChanged(info)
    } else {
        AudioEvent::StreamAdded(info)
    }
}

fn remove_stream(inner: &Rc<Inner>, is_output: bool, index: u32) {
    let (streams, kind) = if is_output {
        (&inner.sink_inputs, KIND_SINK)
    } else {
        (&inner.source_outputs, KIND_SOURCE)
    };
    if streams.borrow_mut().remove(&index).is_some() {
        inner.emit(AudioEvent::StreamRemoved(encode_id(kind, index)));
    }
}

// --- Peak metering ---
//
// Mirrors gnome-control-center's cc-level-bar: a 25 Hz mono F32 record stream
// with PEAK_DETECT, reading from the default sink's monitor source (output)
// or the default source (input).

fn default_sink_monitor_source(inner: &Inner) -> Option<u32> {
    let default = inner.default_sink.borrow();
    let default = default.as_deref()?;
    inner
        .sinks
        .borrow()
        .values()
        .find(|entry| entry.info.name == default)
        .map(|entry| entry.monitor_source)
        .filter(|index| *index != pulse::def::INVALID_INDEX)
}

fn default_source_index(inner: &Inner) -> Option<u32> {
    let default = inner.default_source.borrow();
    let default = default.as_deref()?;
    inner
        .sources
        .borrow()
        .iter()
        .find(|(_, entry)| entry.info.name == default)
        .map(|(index, _)| *index)
}

fn refresh_peak_streams(inner: &Rc<Inner>) {
    refresh_peak_stream(
        inner,
        DeviceType::Sink,
        default_sink_monitor_source(inner),
    );
    refresh_peak_stream(inner, DeviceType::Source, default_source_index(inner));
}

fn refresh_peak_stream(inner: &Rc<Inner>, device_type: DeviceType, target: Option<u32>) {
    let slot = match device_type {
        DeviceType::Sink => &inner.sink_peak,
        DeviceType::Source => &inner.source_peak,
    };

    if slot.borrow().as_ref().map(|peak| peak.target) == target {
        return;
    }

    stop_peak_stream(slot);
    inner.emit(AudioEvent::PeakLevel(device_type, 0.0));

    let Some(target) = target else { return };
    match create_peak_stream(inner, device_type, target) {
        Some(peak) => *slot.borrow_mut() = Some(peak),
        None => log::warn!("Failed to create {} peak stream", device_type.name()),
    }
}

fn stop_peak_stream(slot: &RefCell<Option<PeakStream>>) {
    if let Some(peak) = slot.borrow_mut().take() {
        let mut stream = peak.stream.borrow_mut();
        stream.set_read_callback(None);
        stream.set_suspended_callback(None);
        let _ = stream.disconnect();
    }
}

fn create_peak_stream(
    inner: &Rc<Inner>,
    device_type: DeviceType,
    source_index: u32,
) -> Option<PeakStream> {
    let spec = Spec {
        format: Format::FLOAT32NE,
        channels: 1,
        rate: 25,
    };

    let mut proplist = Proplist::new()?;
    let _ = proplist.set_str(properties::APPLICATION_ID, APPLICATION_ID);
    let _ = proplist.set_str(properties::MEDIA_ROLE, "test");

    let stream = {
        let mut context = inner.context.borrow_mut();
        let context = context.as_mut()?;
        if context.get_state() != ContextState::Ready {
            return None;
        }
        Stream::new_with_proplist(context, "Peak detect", &spec, None, &mut proplist)?
    };
    let stream = Rc::new(RefCell::new(stream));

    let weak_inner = Rc::downgrade(inner);
    let weak_stream: Weak<RefCell<Stream>> = Rc::downgrade(&stream);
    stream
        .borrow_mut()
        .set_read_callback(Some(Box::new(move |_length| {
            let Some(stream) = weak_stream.upgrade() else { return };
            let level = {
                let mut stream = stream.borrow_mut();
                // Discard only what was actually read: dropping an empty
                // fragment is an error, while a hole must still be consumed.
                let (level, consumed) = match stream.peek() {
                    Ok(PeekResult::Data(data)) if data.len() >= 4 => {
                        let start = data.len() - 4;
                        let bytes = [data[start], data[start + 1], data[start + 2], data[start + 3]];
                        (Some(f32::from_ne_bytes(bytes)), true)
                    }
                    Ok(PeekResult::Empty) => (None, false),
                    Ok(_) => (None, true),
                    Err(_) => (None, false),
                };
                if consumed {
                    let _ = stream.discard();
                }
                level
            };
            if let (Some(inner), Some(level)) = (weak_inner.upgrade(), level) {
                inner.emit(AudioEvent::PeakLevel(device_type, level.abs().min(1.0)));
            }
        })));

    let weak_inner = Rc::downgrade(inner);
    let weak_stream: Weak<RefCell<Stream>> = Rc::downgrade(&stream);
    stream
        .borrow_mut()
        .set_suspended_callback(Some(Box::new(move || {
            let Some(stream) = weak_stream.upgrade() else { return };
            let suspended = stream.borrow_mut().is_suspended().unwrap_or(false);
            if suspended {
                if let Some(inner) = weak_inner.upgrade() {
                    inner.emit(AudioEvent::PeakLevel(device_type, 0.0));
                }
            }
        })));

    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: std::mem::size_of::<f32>() as u32,
    };
    let device = source_index.to_string();
    let flags = StreamFlags::DONT_MOVE | StreamFlags::PEAK_DETECT | StreamFlags::ADJUST_LATENCY;
    if let Err(e) = stream
        .borrow_mut()
        .connect_record(Some(&device), Some(&attr), flags)
    {
        log::warn!("Failed to connect peak stream: {e}");
        return None;
    }

    Some(PeakStream {
        stream,
        target: source_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_encoding_round_trips_and_separates_namespaces() {
        let sink = encode_id(KIND_SINK, 7);
        let source = encode_id(KIND_SOURCE, 7);
        assert_ne!(sink, source);
        assert_eq!(decode_id(sink), (KIND_SINK, 7));
        assert_eq!(decode_id(source), (KIND_SOURCE, 7));
    }

    #[test]
    fn volume_conversion_round_trips_with_pulse_scale() {
        // pulse volume / NORM is the cubic scale shown by pavucontrol;
        // a 50% pulse volume must survive the linear round trip.
        let half = Volume(Volume::NORMAL.0 / 2);
        let mut cv = ChannelVolumes::default();
        cv.set(2, half);
        let linear = channel_volumes_to_linear(&cv);
        assert!((linear_to_volume(linear).0 as f64 - half.0 as f64).abs() <= 1.0);
        // And the UI's cubic display value matches pulse percent.
        assert!((linear_to_cubic(linear) - 0.5).abs() < 0.001);
    }

    /// Read-only smoke test against a live server. Opt in with:
    ///
    ///     cargo test -p swaysettings -- --ignored --nocapture live_server
    ///
    /// Changes no audio state; it only enumerates and meters.
    #[test]
    #[ignore = "requires a running PulseAudio/pipewire-pulse server"]
    fn live_server_enumerates_and_meters() {
        use std::collections::BTreeSet;

        #[derive(Default)]
        struct Seen {
            ready: bool,
            /// Keyed by id, never by description: a Bluetooth headset's sink
            /// and source share one description.
            devices: HashMap<u32, (String, &'static str, f64, bool, bool)>,
            removed: BTreeSet<u32>,
            streams: BTreeSet<String>,
            cards: BTreeSet<(u32, usize, Option<u32>)>,
            defaults: Vec<(String, Option<u32>)>,
            peaks: HashMap<&'static str, (u32, f32)>,
            errors: Vec<String>,
        }

        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        // Panics raised inside a PulseAudio callback unwind through C frames
        // and never reach the test's assertions, so count them explicitly.
        // (Process-wide hook: run this test on its own.)
        let panics = Arc::new(AtomicUsize::new(0));
        let previous_hook = std::panic::take_hook();
        {
            let panics = panics.clone();
            std::panic::set_hook(Box::new(move |info| {
                panics.fetch_add(1, Ordering::SeqCst);
                eprintln!("panic inside a PulseAudio callback: {info}");
            }));
        }

        let seen = Rc::new(RefCell::new(Seen::default()));
        let sink = seen.clone();
        let daemon = AudioDaemon::new(move |event| {
            let mut seen = sink.borrow_mut();
            match event {
                AudioEvent::Ready => seen.ready = true,
                AudioEvent::DeviceAdded(info) | AudioEvent::DeviceChanged(info) => {
                    seen.devices.insert(
                        info.id,
                        (
                            info.description.clone(),
                            info.device_type.name(),
                            info.volume,
                            info.is_muted,
                            info.is_default,
                        ),
                    );
                }
                AudioEvent::DeviceRemoved(id) => {
                    seen.removed.insert(id);
                }
                AudioEvent::DefaultChanged(kind, id) => {
                    seen.defaults.push((kind.name().to_string(), id));
                }
                AudioEvent::ProfilesUpdated(card, profiles, active) => {
                    seen.cards.insert((card, profiles.len(), active));
                }
                AudioEvent::StreamAdded(info) | AudioEvent::StreamChanged(info) => {
                    seen.streams.insert(format!(
                        "{} ({}) vol={:.2} muted={} output={}",
                        info.app_name, info.name, info.volume, info.is_muted, info.is_output
                    ));
                }
                AudioEvent::StreamRemoved(_) => {}
                AudioEvent::PeakLevel(kind, level) => {
                    let entry = seen.peaks.entry(kind.name()).or_insert((0, 0.0));
                    entry.0 += 1;
                    entry.1 = entry.1.max(level);
                }
                AudioEvent::Error(e) => seen.errors.push(e),
            }
        });

        daemon.set_metering(true);

        let context = glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        daemon.set_metering(false);
        std::panic::set_hook(previous_hook);

        let seen = seen.borrow();
        println!("\n--- live PulseAudio smoke test ---");
        println!("ready: {}", seen.ready);
        println!("errors: {:?}", seen.errors);
        println!("devices ({}):", seen.devices.len());
        let mut listed: Vec<_> = seen.devices.values().collect();
        listed.sort_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(&b.0)));
        for (description, kind, volume, muted, is_default) in listed {
            // The UI shows the cubic scale, which is what pactl prints as %.
            println!(
                "  [{kind}] {description}  ui={:.0}% linear={volume:.4} muted={muted} default={is_default}",
                linear_to_cubic(*volume) * 100.0
            );
        }
        println!("defaults announced: {:?}", seen.defaults);
        println!("cards (id, n_profiles, active): {:?}", seen.cards);
        println!("app streams ({}):", seen.streams.len());
        for stream in &seen.streams {
            println!("  {stream}");
        }
        println!("peaks: {:?}", seen.peaks);
        println!("device-removed events: {:?}", seen.removed);

        assert_eq!(
            panics.load(Ordering::SeqCst),
            0,
            "panicked inside a PulseAudio callback"
        );
        assert!(seen.ready, "never reached the Ready state");
        assert!(seen.errors.is_empty(), "errors: {:?}", seen.errors);
        assert!(!seen.devices.is_empty(), "no devices enumerated");
        assert!(
            seen.peaks.contains_key("sink"),
            "no output peak events; metering did not attach"
        );
        // Our own peak-detect streams must never show up as applications.
        assert!(
            !seen.streams.iter().any(|s| s.contains("Peak detect")),
            "peak-detect stream leaked into the application list"
        );
    }

    /// Exercises the application-stream and volume-write paths against a live
    /// server, using a playback stream the caller starts (e.g. `paplay` on a
    /// silent file) so no real device volume is ever touched:
    ///
    ///     paplay silence.wav &
    ///     cargo test -p swaysettings -- --ignored --nocapture live_stream
    #[test]
    #[ignore = "requires a running server and an active playback stream"]
    fn live_stream_volume_round_trips() {
        let streams: Rc<RefCell<HashMap<u32, StreamInfo>>> = Rc::new(RefCell::new(HashMap::new()));
        let sink = streams.clone();
        let daemon = AudioDaemon::new(move |event| match event {
            AudioEvent::StreamAdded(info) | AudioEvent::StreamChanged(info) => {
                sink.borrow_mut().insert(info.id, info);
            }
            AudioEvent::StreamRemoved(id) => {
                sink.borrow_mut().remove(&id);
            }
            _ => {}
        });

        let context = glib::MainContext::default();
        let pump = |seconds: f32| {
            let start = std::time::Instant::now();
            while start.elapsed().as_secs_f32() < seconds {
                context.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };

        pump(1.5);

        let target = streams
            .borrow()
            .values()
            .find(|info| info.is_output)
            .cloned()
            .expect("no application playback stream found; start one first");
        println!(
            "target stream: {} ({}) vol={:.4}",
            target.app_name, target.name, target.volume
        );

        let original = target.volume;
        daemon.set_stream_volume(target.id, 0.125); // cubic 50%
        pump(1.5);

        let updated = streams
            .borrow()
            .get(&target.id)
            .cloned()
            .expect("stream vanished mid-test");
        println!(
            "after set: linear={:.4} ui={:.0}%",
            updated.volume,
            linear_to_cubic(updated.volume) * 100.0
        );
        let ui_percent = linear_to_cubic(updated.volume) * 100.0;
        assert!(
            (ui_percent - 50.0).abs() < 2.0,
            "expected ~50% after write, got {ui_percent:.1}%"
        );

        // Round trip: writing back what we read must not drift.
        daemon.set_stream_volume(target.id, updated.volume);
        pump(1.5);
        let round_tripped = streams
            .borrow()
            .get(&target.id)
            .cloned()
            .expect("stream vanished mid-test");
        println!("after round trip: linear={:.4}", round_tripped.volume);
        assert!(
            (linear_to_cubic(round_tripped.volume) - linear_to_cubic(updated.volume)).abs() < 0.01,
            "volume drifted on round trip: {:.4} -> {:.4}",
            updated.volume,
            round_tripped.volume
        );

        daemon.set_stream_volume(target.id, original);
        pump(0.5);
    }

    /// Construct, toggle metering, and tear down against whatever (if any)
    /// PulseAudio socket exists. With ContextFlags::NOFAIL this must be safe
    /// on socketless CI: the context just stays in the connecting state.
    #[test]
    fn daemon_lifecycle_is_safe_without_server() {
        let context = glib::MainContext::default();
        let daemon = AudioDaemon::new(|event| {
            if let AudioEvent::Error(e) = event {
                eprintln!("audio error event: {e}");
            }
        });
        daemon.set_metering(true);
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(300) {
            context.iteration(false);
        }
        daemon.set_metering(false);
        drop(daemon);
    }
}
