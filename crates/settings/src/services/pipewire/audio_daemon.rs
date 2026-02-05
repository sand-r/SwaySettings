use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use pipewire as pw;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::metadata::Metadata;
use pipewire::properties::properties;
use pipewire::registry::GlobalObject;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Pod, Property, PropertyFlags, Value, ValueArray};
use pipewire::spa::sys::{
    // Availability values: unknown=0, no=1, yes=2
    SPA_PARAM_AVAILABILITY_no,
    SPA_PARAM_PROFILE_available,
    SPA_PARAM_PROFILE_classes,
    SPA_PARAM_PROFILE_description,
    SPA_PARAM_PROFILE_index,
    SPA_PARAM_PROFILE_name,
    SPA_PARAM_PROFILE_priority,
    SPA_PARAM_PROFILE_save,
    SPA_PARAM_Profile,
    SPA_PARAM_Props,
    SPA_PARAM_ROUTE_available,
    SPA_PARAM_ROUTE_description,
    SPA_PARAM_ROUTE_device,
    SPA_PARAM_ROUTE_devices,
    // Route param constants
    SPA_PARAM_ROUTE_direction,
    SPA_PARAM_ROUTE_index,
    SPA_PARAM_ROUTE_name,
    SPA_PARAM_ROUTE_props,
    SPA_PARAM_ROUTE_save,
    SPA_PARAM_Route,
    SPA_PROP_channelVolumes,
    SPA_PROP_mute,
    SPA_PROP_volume,
    SPA_TYPE_OBJECT_ParamProfile,
    SPA_TYPE_OBJECT_ParamRoute,
    SPA_TYPE_OBJECT_Props,
    // Direction values
    SPA_DIRECTION_INPUT,
    SPA_DIRECTION_OUTPUT,
};
use pipewire::spa::utils::Direction;
use pipewire::stream::{StreamBox, StreamFlags, StreamListener};
use pipewire::types::ObjectType;

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

fn pw_debug_enabled(flag: &str) -> bool {
    env::var("SWAYSETTINGS_PW_DEBUG")
        .ok()
        .map(|value| value.split(',').any(|item| item.trim() == flag))
        .unwrap_or(false)
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
    pub device_id: Option<u32>,
    pub profile_device_index: Option<u32>,
}

/// Information about a device profile
#[derive(Debug, Clone)]
pub struct ProfileInfo {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub priority: i32,
    pub available: u32,
    pub sink_devices: Vec<u32>,
    pub source_devices: Vec<u32>,
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
        self.available != SPA_PARAM_AVAILABILITY_no
    }

    pub fn supports_device(&self, device_type: DeviceType, device_index: Option<u32>) -> bool {
        if let Some(device_index) = device_index {
            if self.sink_devices.is_empty() && self.source_devices.is_empty() {
                return true;
            }
            return match device_type {
                DeviceType::Sink => self.sink_devices.iter().any(|d| *d == device_index),
                DeviceType::Source => self.source_devices.iter().any(|d| *d == device_index),
            };
        }

        // If we don't know the device index, fall back to class presence.
        if self.sink_devices.is_empty() && self.source_devices.is_empty() {
            return true;
        }

        match device_type {
            DeviceType::Sink => !self.sink_devices.is_empty(),
            DeviceType::Source => !self.source_devices.is_empty(),
        }
    }
}

/// Events sent from the PipeWire thread to the main GTK thread
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
    Error(String),
}

/// Commands sent from the main GTK thread to the PipeWire thread
#[derive(Debug)]
enum AudioCommand {
    SetVolume(u32, f64),
    SetMute(u32, bool),
    SetDefaultSink(u32),
    SetDefaultSource(u32),
    SetProfile(u32, u32),
    Shutdown,
}

/// Convert linear volume (0.0-1.0) to cubic scale for UI display
pub fn linear_to_cubic(v: f64) -> f64 {
    v.cbrt()
}

/// Convert cubic scale (from UI) back to linear volume
pub fn cubic_to_linear(v: f64) -> f64 {
    v.powi(3)
}

/// The audio daemon manages PipeWire connections and device enumeration
pub struct AudioDaemon {
    command_tx: mpsc::Sender<AudioCommand>,
    _thread_handle: thread::JoinHandle<()>,
}

impl AudioDaemon {
    /// Start a new AudioDaemon with a callback for receiving events
    pub fn new<F>(event_callback: F) -> Self
    where
        F: Fn(AudioEvent) + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::channel::<AudioCommand>();

        let thread_handle = thread::spawn(move || {
            if let Err(e) = run_pipewire_loop(command_rx, event_callback) {
                log::error!("PipeWire loop error: {}", e);
            }
        });

        Self {
            command_tx,
            _thread_handle: thread_handle,
        }
    }

    /// Set volume for a device (0.0 to 1.5 for 150%)
    pub fn set_volume(&self, id: u32, volume: f64) {
        let _ = self.command_tx.send(AudioCommand::SetVolume(id, volume));
    }

    /// Set mute state for a device
    pub fn set_mute(&self, id: u32, muted: bool) {
        let _ = self.command_tx.send(AudioCommand::SetMute(id, muted));
    }

    /// Set the default sink (output device)
    pub fn set_default_sink(&self, id: u32) {
        let _ = self.command_tx.send(AudioCommand::SetDefaultSink(id));
    }

    /// Set the default source (input device)
    pub fn set_default_source(&self, id: u32) {
        let _ = self.command_tx.send(AudioCommand::SetDefaultSource(id));
    }

    /// Set the active profile on a device (card)
    pub fn set_profile(&self, device_id: u32, profile_index: u32) {
        let _ = self
            .command_tx
            .send(AudioCommand::SetProfile(device_id, profile_index));
    }
}

impl Drop for AudioDaemon {
    fn drop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
    }
}

/// Internal state for tracking devices
struct DeviceState {
    info: DeviceInfo,
    node: pipewire::node::Node,
    _listener: pipewire::node::NodeListener,
    channel_count: u32,
    /// Whether this node is allowed to be shown in the GUI (filters like bluetooth loopback)
    show_in_ui: bool,
    /// Whether this node is currently visible in the GUI (based on route availability)
    visible: bool,
    /// Whether this node is an internal node (Audio/*/Internal)
    is_internal: bool,
    /// Driver node id (used for Bluetooth loopback nodes)
    driver_id: Option<u32>,
    /// Route match key for availability matching (profile description or nick)
    route_key: String,
    /// Route device index for availability matching (card.profile.device)
    route_device_index: Option<u32>,
    /// Parent device ID (for matching with route availability)
    parent_device_id: Option<u32>,
}

/// State for metadata (default device tracking)
struct MetadataState {
    metadata: Metadata,
    _listener: pipewire::metadata::MetadataListener,
}

/// Route availability info from a PipeWire Device
#[derive(Debug, Clone)]
struct RouteInfo {
    index: i32,
    device: i32,
    name: String,
    #[allow(dead_code)]
    description: String,
    direction: u32, // SPA_DIRECTION_INPUT or SPA_DIRECTION_OUTPUT
    available: u32, // SPA_PARAM_AVAILABILITY_*
    devices: Vec<u32>,
}

/// State for a PipeWire Device (sound card) - tracks route availability
struct PwDeviceState {
    #[allow(dead_code)]
    device: pipewire::device::Device,
    _listener: pipewire::device::DeviceListener,
}

/// Profile state for a PipeWire device
#[derive(Default, Clone)]
struct ProfileState {
    profiles: HashMap<u32, ProfileInfo>,
    active_index: Option<u32>,
    pending_index: Option<u32>,
    pending_since: Option<Instant>,
}

const PROFILE_PENDING_TIMEOUT: Duration = Duration::from_secs(2);

/// State for peak level monitoring stream
struct PeakMonitorState<F: Fn(AudioEvent) + 'static> {
    #[allow(dead_code)]
    stream: StreamBox<'static>,
    #[allow(dead_code)]
    listener: StreamListener<PeakUserData<F>>,
}

/// User data for peak monitoring callback
struct PeakUserData<F: Fn(AudioEvent) + 'static> {
    device_type: DeviceType,
    event_callback: Rc<F>,
}

/// Create a peak monitoring stream for a device type
fn create_peak_monitor<F>(
    core: &'static pipewire::core::CoreRc,
    device_type: DeviceType,
    event_callback: Rc<F>,
) -> Option<PeakMonitorState<F>>
where
    F: Fn(AudioEvent) + 'static,
{
    let stream_name = match device_type {
        DeviceType::Sink => "output-peak-detect",
        DeviceType::Source => "input-peak-detect",
    };

    // Create stream properties for peak detection
    // For sinks, we use stream.capture.sink=true to capture from the sink's monitor
    let props = match device_type {
        DeviceType::Sink => properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "DSP",
            *pw::keys::NODE_NAME => stream_name,
            // Capture from sink's monitor (what's being played)
            "stream.capture.sink" => "true",
            // Use peak detection instead of actual audio
            "resample.peaks" => "true",
            // Don't show in volume controls
            "stream.monitor" => "true",
        },
        DeviceType::Source => properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "DSP",
            *pw::keys::NODE_NAME => stream_name,
            // Use peak detection instead of actual audio
            "resample.peaks" => "true",
            // Don't show in volume controls
            "stream.monitor" => "true",
        },
    };

    // CoreRc derefs to Core, but StreamBox needs &Core with 'static lifetime
    // Since core is &'static CoreRc, we can safely get a 'static reference to Core
    let core_ref: &'static pipewire::core::Core = unsafe {
        // SAFETY: core is 'static, so the Core inside is also valid for 'static
        &*(core as &pipewire::core::Core as *const pipewire::core::Core)
    };

    let stream = match StreamBox::new(core_ref, stream_name, props) {
        Ok(s) => s,
        Err(e) => {
            log::warn!(
                "Failed to create peak monitor stream for {:?}: {}",
                device_type,
                e
            );
            return None;
        }
    };

    let user_data = PeakUserData {
        device_type,
        event_callback,
    };

    // Set up listener for the stream
    let listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_, user_data, old_state, new_state| {
            log::debug!(
                "Peak stream {:?} state: {:?} -> {:?}",
                user_data.device_type,
                old_state,
                new_state
            );
        })
        .process(|stream, user_data| {
            // Get the buffer with peak data
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut() {
                    let chunk = data.chunk();
                    if chunk.size() >= 4 {
                        if let Some(slice) = data.data() {
                            if slice.len() >= 4 {
                                // Peak data is a single f32 value
                                let peak =
                                    f32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]);
                                log::trace!("Peak {:?}: {}", user_data.device_type, peak);
                                (user_data.event_callback)(AudioEvent::PeakLevel(
                                    user_data.device_type,
                                    peak.abs().min(1.0),
                                ));
                            }
                        }
                    }
                }
            } else {
                log::trace!("No buffer for {:?}", user_data.device_type);
            }
        })
        .register();

    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            log::warn!("Failed to register peak monitor listener: {}", e);
            return None;
        }
    };

    // Build audio format params for the stream - mono F32 at 25Hz for peak detection
    use pw::spa::param::audio::AudioFormat;
    use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use pw::spa::param::ParamType;
    use pw::spa::pod::{object, property};
    use pw::spa::utils::SpaTypes;

    let format_obj = object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        property!(FormatProperties::MediaType, Id, MediaType::Audio),
        property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        property!(FormatProperties::AudioFormat, Id, AudioFormat::F32LE),
        property!(FormatProperties::AudioRate, Int, 25),
        property!(FormatProperties::AudioChannels, Int, 1),
    );

    let format_bytes: Vec<u8> =
        PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(format_obj))
            .map(|r| r.0.into_inner())
            .unwrap_or_default();

    let format_pod = match Pod::from_bytes(&format_bytes) {
        Some(p) => p,
        None => {
            log::warn!("Failed to create format pod");
            return None;
        }
    };

    let mut params = [format_pod];

    if let Err(e) = stream.connect(
        Direction::Input,
        None, // Let PipeWire find the target
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        &mut params,
    ) {
        log::warn!("Failed to connect peak monitor stream: {}", e);
        return None;
    }

    log::info!(
        "Created peak monitor for {:?}, stream state: {:?}",
        device_type,
        stream.state()
    );

    Some(PeakMonitorState { stream, listener })
}

/// Run the PipeWire main loop
fn run_pipewire_loop<F>(
    command_rx: mpsc::Receiver<AudioCommand>,
    event_callback: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: Fn(AudioEvent) + 'static,
{
    pw::init();

    let main_loop = MainLoopRc::new(None)?;
    let context = ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;

    // Leak the core to make it 'static - this is intentional for long-running daemon
    let core: &'static _ = Box::leak(Box::new(core));
    let registry = Rc::new(core.get_registry()?);

    // Shared state
    let nodes: Rc<RefCell<HashMap<u32, DeviceState>>> = Rc::new(RefCell::new(HashMap::new()));
    let default_sink_name: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let default_source_name: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let metadata_state: Rc<RefCell<Option<MetadataState>>> = Rc::new(RefCell::new(None));
    // PipeWire Device objects (sound cards)
    let pw_devices: Rc<RefCell<HashMap<u32, PwDeviceState>>> =
        Rc::new(RefCell::new(HashMap::new()));
    // Route availability info: device_id -> Vec<RouteInfo>
    let route_availability: Rc<RefCell<HashMap<u32, Vec<RouteInfo>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    // Profile info per device (card)
    let profiles: Rc<RefCell<HashMap<u32, ProfileState>>> = Rc::new(RefCell::new(HashMap::new()));

    // Clone for closures - each closure that captures with `move` needs its own clone
    let nodes_for_global = nodes.clone();
    let nodes_for_remove = nodes.clone();
    let nodes_for_commands = nodes.clone();
    let default_sink_name_for_global = default_sink_name.clone();
    let default_source_name_for_global = default_source_name.clone();
    let metadata_state_for_global = metadata_state.clone();
    let metadata_state_for_commands = metadata_state.clone();
    let pw_devices_for_commands = pw_devices.clone();
    let pw_devices_for_global = pw_devices.clone();
    let route_availability_for_global = route_availability.clone();
    let route_availability_for_nodes = route_availability.clone();
    let route_availability_for_commands = route_availability.clone();
    let nodes_for_route_check = nodes.clone();
    let profiles_for_global = profiles.clone();
    let profiles_for_commands = profiles.clone();
    let registry_for_global = registry.clone();

    // Wrap callback in Rc for sharing
    let event_callback = Rc::new(event_callback);
    let event_callback_for_global = event_callback.clone();
    let event_callback_for_remove = event_callback.clone();
    let event_callback_for_params = event_callback.clone();
    let event_callback_for_route_check = event_callback.clone();
    let event_callback_for_commands = event_callback.clone();

    // Create peak monitoring streams for output and input
    let _sink_peak_monitor = create_peak_monitor(core, DeviceType::Sink, event_callback.clone());
    let _source_peak_monitor =
        create_peak_monitor(core, DeviceType::Source, event_callback.clone());

    // Registry listener for global objects
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            // Handle metadata objects
            if global.type_ == ObjectType::Metadata {
                handle_metadata_added(
                    global,
                    &registry_for_global,
                    &metadata_state_for_global,
                    &nodes_for_global,
                    &default_sink_name_for_global,
                    &default_source_name_for_global,
                    event_callback_for_global.clone(),
                );
                return;
            }

            // Handle PipeWire Device objects (sound cards) to get route availability
            if global.type_ == ObjectType::Device {
                handle_pw_device_added(
                    global,
                    &registry_for_global,
                    &pw_devices_for_global,
                    &route_availability_for_global,
                    &profiles_for_global,
                    &nodes_for_route_check,
                    event_callback_for_route_check.clone(),
                );
                return;
            }

            handle_node_added(
                global,
                &registry_for_global,
                &nodes_for_global,
                &default_sink_name_for_global,
                &default_source_name_for_global,
                &route_availability_for_nodes,
                event_callback_for_global.as_ref(),
                event_callback_for_params.clone(),
            );
        })
        .global_remove(move |id| {
            let mut devs = nodes_for_remove.borrow_mut();
            if devs.remove(&id).is_some() {
                event_callback_for_remove(AudioEvent::DeviceRemoved(id));
            }
        })
        .register();

    event_callback(AudioEvent::Ready);

    // Set up command channel
    let (pw_sender, pw_receiver) = pw::channel::channel::<AudioCommand>();

    // Spawn thread to forward commands from std channel to pw channel
    let _command_forwarder = thread::spawn(move || {
        while let Ok(cmd) = command_rx.recv() {
            if pw_sender.send(cmd).is_err() {
                break;
            }
        }
    });

    // Track whether we should quit
    let should_quit = Rc::new(RefCell::new(false));
    let should_quit_clone = should_quit.clone();

    // Attach receiver to the loop
    let loop_ = main_loop.loop_();
    let _receiver_handle = pw_receiver.attach(&loop_, move |cmd| match cmd {
        AudioCommand::Shutdown => {
            *should_quit_clone.borrow_mut() = true;
        }
        AudioCommand::SetVolume(id, volume) => {
            set_node_volume(
                &nodes_for_commands,
                &pw_devices_for_commands,
                &route_availability_for_commands,
                id,
                volume,
            );
        }
        AudioCommand::SetMute(id, muted) => {
            set_node_mute(
                &nodes_for_commands,
                &pw_devices_for_commands,
                &route_availability_for_commands,
                id,
                muted,
            );
        }
        AudioCommand::SetDefaultSink(id) => {
            set_default_device(
                &nodes_for_commands,
                &pw_devices_for_commands,
                &route_availability_for_commands,
                &metadata_state_for_commands,
                id,
                DeviceType::Sink,
            );
        }
        AudioCommand::SetDefaultSource(id) => {
            set_default_device(
                &nodes_for_commands,
                &pw_devices_for_commands,
                &route_availability_for_commands,
                &metadata_state_for_commands,
                id,
                DeviceType::Source,
            );
        }
        AudioCommand::SetProfile(device_id, profile_index) => {
            set_device_profile(
                &pw_devices_for_commands,
                &profiles_for_commands,
                device_id,
                profile_index,
                event_callback_for_commands.as_ref(),
            );
        }
    });

    // Run the loop
    loop {
        loop_.iterate(std::time::Duration::from_millis(100));
        if *should_quit.borrow() {
            break;
        }
    }

    Ok(())
}

fn handle_node_added<F>(
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    registry: &pipewire::registry::Registry,
    nodes: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    default_sink_name: &Rc<RefCell<Option<String>>>,
    default_source_name: &Rc<RefCell<Option<String>>>,
    route_availability: &Rc<RefCell<HashMap<u32, Vec<RouteInfo>>>>,
    event_callback: &F,
    event_callback_rc: Rc<F>,
) where
    F: Fn(AudioEvent) + 'static,
{
    if global.type_ != ObjectType::Node {
        return;
    }

    let props = match global.props {
        Some(props) => props,
        None => return,
    };

    let media_class = match props.get("media.class") {
        Some(mc) => mc,
        None => return,
    };

    let is_internal = media_class.contains("/Internal");

    let prop_is_true = |key: &str| matches!(props.get(key), Some("true") | Some("1"));
    let bluez_loopback =
        prop_is_true("bluez5.sink-loopback") || prop_is_true("bluez5.source-loopback");
    let bluez_loopback_target = prop_is_true("bluez5.sink-loopback-target")
        || prop_is_true("bluez5.source-loopback-target");

    let device_type = if media_class.starts_with("Audio/Sink") {
        DeviceType::Sink
    } else if media_class.starts_with("Audio/Source") {
        if media_class.contains("Monitor") {
            return;
        }
        DeviceType::Source
    } else {
        return;
    };

    let name = props.get("node.name").unwrap_or("").to_string();
    if name.ends_with(".monitor") {
        return;
    }

    let is_bluez_output = name.starts_with("bluez_output.");
    let is_bluez_input = name.starts_with("bluez_input.");
    let is_bluez_output_internal = name.starts_with("bluez_output_internal.");
    let is_bluez_input_internal = name.starts_with("bluez_input_internal.");
    let is_bluez_wrapper = is_bluez_output || is_bluez_input;
    let is_bluez_internal = is_bluez_output_internal || is_bluez_input_internal;

    // Prefer profile description or node nick for route matching
    // (these correspond to PipeWire route descriptions like "HDMI / DisplayPort 1 Output")
    let profile_desc = props.get("device.profile.description");
    let nick = props.get("node.nick");
    let card_desc = props.get("node.description");

    let description = match (nick, card_desc) {
        (Some(n), Some(c)) if !n.is_empty() => format!("{} - {}", n, c),
        (Some(n), _) if !n.is_empty() => n.to_string(),
        (_, Some(c)) if !c.is_empty() => c.to_string(),
        _ => name.clone(),
    };
    let description_for_match = description.clone();

    // Get icon name from PipeWire (e.g., "audio-speakers", "audio-headphones", "video-display")
    let icon_name = props.get("device.icon_name").map(|s| s.to_string());

    // Log all properties to understand what PipeWire provides
    if pw_debug_enabled("props") {
        log::info!("PW-PROPS Node {} properties:", name);
        for (key, value) in props.iter() {
            log::info!("  {}: {}", key, value);
        }
    }

    // Get parent device ID for route availability matching
    let parent_device_id = props.get("device.id").and_then(|s| s.parse::<u32>().ok());

    let driver_id = props
        .get("node.driver-id")
        .and_then(|s| s.parse::<u32>().ok());

    // Store the route match key for availability matching
    let route_key = profile_desc
        .filter(|s| !s.is_empty())
        .or_else(|| nick.filter(|s| !s.is_empty()))
        .or_else(|| card_desc.filter(|s| !s.is_empty()))
        .unwrap_or("")
        .to_string();

    // Route device index (used for matching route availability by index)
    let route_device_index = props
        .get("card.profile.device")
        .and_then(|s| s.parse::<u32>().ok());

    // Decide whether this node should ever be shown in the UI.
    // Bluetooth devices expose loopback + internal nodes; prefer the loopback target (internal)
    // and hide the loopback wrapper nodes.
    let show_in_ui = if bluez_loopback {
        false
    } else if is_internal {
        bluez_loopback_target
    } else {
        true
    };

    log::debug!(
        "Node {} visibility decision: internal={} bluez_loopback={} target={} show_in_ui={}",
        name,
        is_internal,
        bluez_loopback,
        bluez_loopback_target,
        show_in_ui
    );

    // Check initial route availability
    let mut initially_visible = show_in_ui;
    if let Some(device_id) = parent_device_id {
        let routes = route_availability.borrow();
        if let Some(device_routes) = routes.get(&device_id) {
            let expected_direction = match device_type {
                DeviceType::Sink => SPA_DIRECTION_OUTPUT,
                DeviceType::Source => SPA_DIRECTION_INPUT,
            };

            let route_info = route_device_index
                .and_then(|idx| {
                    device_routes.iter().find(|r| {
                        r.direction == expected_direction
                            && (r.device == idx as i32 || r.devices.iter().any(|d| *d == idx))
                    })
                })
                .or_else(|| find_matching_route(device_routes, &route_key, expected_direction));

            if let Some(route_info) = route_info {
                if route_info.available == SPA_PARAM_AVAILABILITY_no {
                    initially_visible = false;
                    log::debug!(
                        "Node {} initially hidden: route '{}' not available",
                        name,
                        route_info.name,
                    );
                }
            }
        }
    }

    let id = global.id;

    // Check if this device is the current default
    let is_default = match device_type {
        DeviceType::Sink => default_sink_name
            .borrow()
            .as_ref()
            .map_or(false, |n| n == &name),
        DeviceType::Source => default_source_name
            .borrow()
            .as_ref()
            .map_or(false, |n| n == &name),
    };

    let info = DeviceInfo {
        id,
        name: name.clone(),
        description,
        icon_name,
        volume: 1.0,
        is_muted: false,
        is_default,
        device_type,
        device_id: parent_device_id,
        profile_device_index: route_device_index,
    };

    let node: pipewire::node::Node = match registry.bind(global) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to bind node {}: {}", id, e);
            return;
        }
    };

    let nodes_for_listener = nodes.clone();
    let default_sink_name_for_listener = default_sink_name.clone();
    let default_source_name_for_listener = default_source_name.clone();
    let name_for_listener = name.clone();

    let nodes_for_info = nodes.clone();
    let listener = node
        .add_listener_local()
        .info(move |info| {
            let Some(props) = info.props() else { return };

            let mut nodes_mut = nodes_for_info.borrow_mut();
            let Some(state) = nodes_mut.get_mut(&id) else { return };

            if state.route_device_index.is_none() {
                if let Some(idx) = props
                    .get("card.profile.device")
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    state.route_device_index = Some(idx);
                    state.info.profile_device_index = Some(idx);
                    if pw_debug_enabled("volume") {
                        log::info!(
                            "VOL-DBG node {} updated route_device_index={}",
                            state.info.id,
                            idx
                        );
                    }
                }
            }

            let is_bluez_node = state.info.name.starts_with("bluez_");
            if is_bluez_node && state.driver_id.is_none() {
                if let Some(driver_id) = props
                    .get("node.driver-id")
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    state.driver_id = Some(driver_id);
                    if pw_debug_enabled("volume") {
                        log::info!(
                            "VOL-DBG node {} updated driver_id={}",
                            state.info.id,
                            driver_id
                        );
                    }
                }
            }

            if state.parent_device_id.is_none() {
                if let Some(device_id) = props.get("device.id").and_then(|s| s.parse::<u32>().ok()) {
                    state.parent_device_id = Some(device_id);
                    state.info.device_id = Some(device_id);
                    if pw_debug_enabled("volume") {
                        log::info!(
                            "VOL-DBG node {} updated parent_device_id={}",
                            state.info.id,
                            device_id
                        );
                    }
                }
            }

            if state.route_key.is_empty() {
                let profile_desc = props.get("device.profile.description");
                let nick = props.get("node.nick");
                let card_desc = props.get("node.description");
                let route_key = profile_desc
                    .filter(|s| !s.is_empty())
                    .or_else(|| nick.filter(|s| !s.is_empty()))
                    .or_else(|| card_desc.filter(|s| !s.is_empty()))
                    .unwrap_or("")
                    .to_string();

                if !route_key.is_empty() {
                    state.route_key = route_key.clone();
                    if pw_debug_enabled("volume") {
                        log::info!(
                            "VOL-DBG node {} updated route_key='{}'",
                            state.info.id,
                            route_key
                        );
                    }
                }
            }
        })
        .param(move |_seq, param_type, _index, _next, param| {
            if param_type != ParamType::Props {
                return;
            }

            let Some(pod) = param else { return };

            if let Some((volume, muted, channel_count)) = parse_audio_props(pod) {
                let mut changed_infos: Vec<DeviceInfo> = Vec::new();
                let mut devs = nodes_for_listener.borrow_mut();
                if let Some(state) = devs.get_mut(&id) {
                    log::debug!(
                        "Props for node {} ({}): vol={:.3} muted={} channels={} internal={} driver_id={:?}",
                        id,
                        state.info.description,
                        volume,
                        muted,
                        channel_count,
                        state.is_internal,
                        state.driver_id
                    );

                    state.channel_count = channel_count;

                    // Skip volume updates from follower nodes (Bluetooth loopbacks)
                    let ignore_volume = !state.is_internal && state.driver_id.is_some();
                    if ignore_volume {
                        log::debug!(
                            "Ignoring volume update for follower node {} (driver_id {:?})",
                            id,
                            state.driver_id
                        );
                    }

                    if !ignore_volume {
                        let volume_delta = (state.info.volume - volume).abs();
                        let near_zero = state.info.volume <= 0.001 || volume <= 0.001;
                        let changed = volume_delta > 0.001
                            || (near_zero && volume_delta > 0.0)
                            || state.info.is_muted != muted;

                        state.info.volume = volume;
                        state.info.is_muted = muted;

                        if changed {
                            changed_infos.push(state.info.clone());
                        }
                    }

                    // Update is_default based on name comparison
                    let is_default = match state.info.device_type {
                        DeviceType::Sink => default_sink_name_for_listener
                            .borrow()
                            .as_ref()
                            .map_or(false, |n| n == &name_for_listener),
                        DeviceType::Source => default_source_name_for_listener
                            .borrow()
                            .as_ref()
                            .map_or(false, |n| n == &name_for_listener),
                    };
                    state.info.is_default = is_default;

                    // If this is an internal node, propagate volume/mute to follower nodes
                    if state.is_internal {
                        let driver_id = state.info.id;
                        log::debug!(
                            "Propagating internal volume from node {} to followers",
                            driver_id
                        );
                        for follower in devs.values_mut() {
                            if follower.driver_id == Some(driver_id) {
                                let volume_delta = (follower.info.volume - volume).abs();
                                let near_zero =
                                    follower.info.volume <= 0.001 || volume <= 0.001;
                                let changed = volume_delta > 0.001
                                    || (near_zero && volume_delta > 0.0)
                                    || follower.info.is_muted != muted;
                                follower.info.volume = volume;
                                follower.info.is_muted = muted;
                                if changed {
                                    changed_infos.push(follower.info.clone());
                                }
                            }
                        }
                    }
                }
                drop(devs);

                for info in changed_infos {
                    log::debug!(
                        "Device {} ({}) changed: vol={:.2}, muted={}",
                        info.description,
                        info.device_type.name(),
                        info.volume,
                        info.is_muted
                    );
                    event_callback_rc(AudioEvent::DeviceChanged(info));
                }
            }
        })
        .register();

    node.subscribe_params(&[ParamType::Props]);
    // Fetch initial volume/mute values (some nodes don't emit Props immediately)
    node.enum_params(0, Some(ParamType::Props), 0, u32::MAX);

    log::debug!(
        "Added {} device: {} (id={}, default={}, visible={}, show_in_ui={})",
        device_type.name(),
        info.description,
        id,
        is_default,
        initially_visible,
        show_in_ui
    );

    let state = DeviceState {
        info: info.clone(),
        node,
        _listener: listener,
        channel_count: 2,
        show_in_ui,
        visible: initially_visible,
        is_internal,
        driver_id,
        route_key,
        route_device_index,
        parent_device_id,
    };
    let mut nodes_mut = nodes.borrow_mut();
    nodes_mut.insert(id, state);

    // For Bluetooth devices, map the visible wrapper node to the internal node that carries volume.
    if is_bluez_internal {
        let driver_id = id;
        for other in nodes_mut.values_mut() {
            if other.is_internal {
                continue;
            }

            if other.parent_device_id != parent_device_id {
                continue;
            }

            if other.info.device_type != device_type {
                continue;
            }

            let other_name = &other.info.name;
            let is_wrapper =
                other_name.starts_with("bluez_output.") || other_name.starts_with("bluez_input.");

            if !is_wrapper {
                continue;
            }

            if other.info.description != description_for_match {
                continue;
            }

            other.driver_id = Some(driver_id);
            log::debug!(
                "Linked BlueZ wrapper {} -> internal node {} for volume control",
                other.info.name,
                driver_id
            );
        }
    } else if is_bluez_wrapper && driver_id.is_none() {
        if let Some(internal_id) = nodes_mut.values().find_map(|state| {
            if !state.is_internal {
                return None;
            }

            if state.parent_device_id != parent_device_id {
                return None;
            }

            if state.info.device_type != device_type {
                return None;
            }

            let internal_name = &state.info.name;
            let is_internal_bluez = internal_name.starts_with("bluez_output_internal.")
                || internal_name.starts_with("bluez_input_internal.");
            if !is_internal_bluez {
                return None;
            }

            if state.info.description != description_for_match {
                return None;
            }

            Some(state.info.id)
        }) {
            if let Some(wrapper_state) = nodes_mut.get_mut(&id) {
                wrapper_state.driver_id = Some(internal_id);
                log::debug!(
                    "Linked BlueZ wrapper {} -> internal node {} for volume control",
                    wrapper_state.info.name,
                    internal_id
                );
            }
        }
    }

    // Only notify GUI if initially visible
    if initially_visible {
        event_callback(AudioEvent::DeviceAdded(info));
    }
}

/// Find a matching route for a node nick
fn find_matching_route<'a>(
    routes: &'a [RouteInfo],
    route_key: &str,
    expected_direction: u32,
) -> Option<&'a RouteInfo> {
    if route_key.is_empty() {
        return None;
    }

    routes
        .iter()
        .find(|r| r.direction == expected_direction && route_matches_key(r, route_key))
}

fn find_route_for_state<'a>(state: &DeviceState, routes: &'a [RouteInfo]) -> Option<&'a RouteInfo> {
    let expected_direction = match state.info.device_type {
        DeviceType::Sink => SPA_DIRECTION_OUTPUT,
        DeviceType::Source => SPA_DIRECTION_INPUT,
    };

    let mut route = None;

    if let Some(idx) = state.route_device_index {
        route = routes.iter().find(|r| {
            r.direction == expected_direction
                && (r.device == idx as i32 || r.devices.iter().any(|d| *d == idx))
        });
    }

    if route.is_none() && !state.route_key.is_empty() {
        route = find_matching_route(routes, &state.route_key, expected_direction);
    }

    route
}

/// Update node visibility based on route availability and emit events
fn update_node_visibility<F>(
    nodes: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    device_id: u32,
    route_info: &RouteInfo,
    event_callback: &F,
) where
    F: Fn(AudioEvent),
{
    let is_available = route_info.available != SPA_PARAM_AVAILABILITY_no;

    let mut nodes_mut = nodes.borrow_mut();
    for state in nodes_mut.values_mut() {
        if !state.show_in_ui {
            continue;
        }
        // Only check nodes belonging to this device
        if state.parent_device_id != Some(device_id) {
            continue;
        }

        let node_direction = match state.info.device_type {
            DeviceType::Sink => SPA_DIRECTION_OUTPUT,
            DeviceType::Source => SPA_DIRECTION_INPUT,
        };

        if node_direction != route_info.direction {
            continue;
        }

        // Check if this node matches the route
        let matches = if let Some(idx) = state.route_device_index {
            route_info.device == idx as i32 || route_info.devices.iter().any(|d| *d == idx)
        } else if !state.route_key.is_empty() {
            route_matches_key(route_info, &state.route_key)
        } else {
            false
        };

        if !matches {
            continue;
        }

        // Update visibility if changed
        if state.visible != is_available {
            state.visible = is_available;
            if is_available {
                log::debug!(
                    "Node {} now available: route '{}'",
                    state.info.name,
                    route_info.name
                );
                event_callback(AudioEvent::DeviceAdded(state.info.clone()));
            } else {
                log::debug!(
                    "Node {} now unavailable: route '{}'",
                    state.info.name,
                    route_info.name
                );
                event_callback(AudioEvent::DeviceRemoved(state.info.id));
            }
        }
    }
}

/// Normalize and compare a node route key to a route definition
fn route_matches_key(route_info: &RouteInfo, route_key: &str) -> bool {
    let normalize = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    };

    let key_normalized = normalize(route_key);

    let mut candidates = Vec::new();
    if !route_info.description.is_empty() {
        candidates.push(normalize(&route_info.description));
    }
    candidates.push(normalize(&route_info.name));

    candidates.iter().any(|route_normalized| {
        key_normalized == *route_normalized
            || key_normalized.starts_with(route_normalized)
            || route_normalized.starts_with(&key_normalized)
            || key_normalized.ends_with(route_normalized)
    })
}

/// Handle PipeWire Device objects (sound cards) to get route availability info
fn handle_pw_device_added<F>(
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    registry: &pipewire::registry::Registry,
    pw_devices: &Rc<RefCell<HashMap<u32, PwDeviceState>>>,
    route_availability: &Rc<RefCell<HashMap<u32, Vec<RouteInfo>>>>,
    profiles: &Rc<RefCell<HashMap<u32, ProfileState>>>,
    nodes: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    event_callback: Rc<F>,
) where
    F: Fn(AudioEvent) + 'static,
{
    let props = match global.props {
        Some(props) => props,
        None => return,
    };

    let device_name = props.get("device.name").unwrap_or("");

    log::debug!("Found PipeWire Device: {} (id={})", device_name, global.id);

    let device: pipewire::device::Device = match registry.bind(global) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to bind device {}: {}", global.id, e);
            return;
        }
    };

    let device_id = global.id;
    let route_availability_for_listener = route_availability.clone();
    let profiles_for_listener = profiles.clone();
    let nodes_for_listener = nodes.clone();
    let event_callback_for_listener = event_callback;

    // Subscribe to Route params to get availability info
    let listener = device
        .add_listener_local()
        .info(|_info| {
            // Device info received, routes will come via param callback
        })
        .param(move |_seq, param_type, _index, _next, param| {
            if let Some(pod) = param {
                if param_type == ParamType::EnumProfile {
                    if let Some(profile) = parse_profile_param(pod) {
                        let profile_index = profile.index;
                        let mut profiles_map = profiles_for_listener.borrow_mut();
                        let entry = profiles_map.entry(device_id).or_default();
                        entry.profiles.insert(profile_index, profile);
                        if pw_debug_enabled("profile") {
                            if let Some(info) = entry.profiles.get(&profile_index) {
                                log::info!(
                                    "PROFILE-DBG enum device={} idx={} name='{}' desc='{}' avail={} prio={} sinks={:?} sources={:?}",
                                    device_id,
                                    info.index,
                                    info.name,
                                    info.description,
                                    info.available,
                                    info.priority,
                                    info.sink_devices,
                                    info.source_devices
                                );
                            }
                        }
                        maybe_clear_pending_profile(entry, entry.active_index, device_id);
                        emit_profiles_update(
                            device_id,
                            entry,
                            event_callback_for_listener.as_ref(),
                        );
                    }
                } else if param_type == ParamType::Profile {
                    let mut profiles_map = profiles_for_listener.borrow_mut();
                    let entry = profiles_map.entry(device_id).or_default();

                    if let Some(profile) = parse_profile_param(pod) {
                        let profile_index = profile.index;
                        entry.active_index = Some(profile_index);
                        entry.profiles.entry(profile_index).or_insert(profile);
                        if pw_debug_enabled("profile") {
                            if let Some(info) = entry.profiles.get(&profile_index) {
                                log::info!(
                                    "PROFILE-DBG active device={} idx={} name='{}' desc='{}' avail={} prio={} sinks={:?} sources={:?}",
                                    device_id,
                                    info.index,
                                    info.name,
                                    info.description,
                                    info.available,
                                    info.priority,
                                    info.sink_devices,
                                    info.source_devices
                                );
                            }
                        }
                        maybe_clear_pending_profile(entry, Some(profile_index), device_id);
                    } else if let Some(active_index) = parse_profile_index(pod) {
                        entry.active_index = Some(active_index);
                        if pw_debug_enabled("profile") {
                            log::info!(
                                "PROFILE-DBG active device={} idx={} (index only)",
                                device_id,
                                active_index
                            );
                        }
                        maybe_clear_pending_profile(entry, Some(active_index), device_id);
                    }

                    emit_profiles_update(device_id, entry, event_callback_for_listener.as_ref());
                } else if param_type == ParamType::EnumRoute || param_type == ParamType::Route {
                    if let Some(route_info) = parse_route_param(pod) {
                        if pw_debug_enabled("volume") {
                            log::info!(
                                "VOL-DBG route param device={} idx={} dev={} dir={} avail={} name='{}'",
                                device_id,
                                route_info.index,
                                route_info.device,
                                route_info.direction,
                                route_info.available,
                                route_info.name
                            );
                        } else {
                            log::debug!(
                                "Device {} route: name='{}', direction={}, available={}",
                                device_id,
                                route_info.name,
                                if route_info.direction == SPA_DIRECTION_OUTPUT {
                                    "output"
                                } else {
                                    "input"
                                },
                                route_info.available
                            );
                        }

                        // Update node visibility based on route availability
                        update_node_visibility(
                            &nodes_for_listener,
                            device_id,
                            &route_info,
                            event_callback_for_listener.as_ref(),
                        );

                        // Store route info for future node additions
                        let mut routes = route_availability_for_listener.borrow_mut();
                        let device_routes = routes.entry(device_id).or_insert_with(Vec::new);

                        // Update existing route or add new one
                        if let Some(existing) = device_routes.iter_mut().find(|r| {
                            r.direction == route_info.direction
                                && (r.index == route_info.index && r.device == route_info.device)
                        }) {
                            *existing = route_info;
                        } else {
                            device_routes.push(route_info);
                        }
                    }
                }
            }
        })
        .register();

    // Subscribe to profiles and routes for all devices (route volumes are used by WirePlumber)
    let mut params = vec![ParamType::EnumProfile, ParamType::Profile];
    params.push(ParamType::EnumRoute);
    params.push(ParamType::Route);
    device.subscribe_params(&params);

    // Explicitly enumerate initial params so we have route/profile data immediately
    device.enum_params(0, Some(ParamType::EnumProfile), 0, u32::MAX);
    device.enum_params(0, Some(ParamType::Profile), 0, u32::MAX);
    device.enum_params(0, Some(ParamType::EnumRoute), 0, u32::MAX);
    device.enum_params(0, Some(ParamType::Route), 0, u32::MAX);

    let state = PwDeviceState {
        device,
        _listener: listener,
    };

    pw_devices.borrow_mut().insert(global.id, state);
}

/// Parse a Route param pod to extract availability info
fn parse_route_param(pod: &Pod) -> Option<RouteInfo> {
    let value = match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
        Ok((_, val)) => val,
        Err(_) => return None,
    };

    let Value::Object(obj) = value else {
        return None;
    };

    let mut index: i32 = -1;
    let mut device: i32 = -1;
    let mut name = String::new();
    let mut description = String::new();
    let mut direction: u32 = 0;
    let mut available: u32 = 0;
    let mut devices: Vec<u32> = Vec::new();

    for prop in obj.properties {
        match prop.key {
            k if k == SPA_PARAM_ROUTE_name => {
                if let Value::String(s) = prop.value {
                    name = s;
                }
            }
            k if k == SPA_PARAM_ROUTE_index => match prop.value {
                Value::Int(i) => index = i,
                Value::Id(id) => index = id.0 as i32,
                _ => {}
            },
            k if k == SPA_PARAM_ROUTE_device => match prop.value {
                Value::Int(i) => device = i,
                Value::Id(id) => device = id.0 as i32,
                _ => {}
            },
            k if k == SPA_PARAM_ROUTE_description => {
                if let Value::String(s) = prop.value {
                    description = s;
                }
            }
            k if k == SPA_PARAM_ROUTE_direction => match prop.value {
                Value::Id(id) => direction = id.0,
                Value::Int(i) => direction = i as u32,
                _ => {}
            },
            k if k == SPA_PARAM_ROUTE_available => match prop.value {
                Value::Id(id) => available = id.0,
                Value::Int(i) => available = i as u32,
                _ => {}
            },
            k if k == SPA_PARAM_ROUTE_devices => match prop.value {
                Value::ValueArray(ValueArray::Int(vals)) => {
                    devices = vals.iter().map(|v| *v as u32).collect();
                }
                Value::ValueArray(ValueArray::Id(vals)) => {
                    devices = vals.iter().map(|v| v.0).collect();
                }
                _ => {}
            },
            _ => {}
        }
    }

    if name.is_empty() {
        name = format!("route-{}-{}", direction, device);
    }

    // Strip [Out] or [In] prefix from route names
    // e.g., "[Out] Speaker" -> "Speaker", "[In] Mic" -> "Mic"
    let name = name
        .strip_prefix("[Out] ")
        .or_else(|| name.strip_prefix("[In] "))
        .unwrap_or(&name)
        .to_string();

    Some(RouteInfo {
        index,
        device,
        name,
        description,
        direction,
        available,
        devices,
    })
}

/// Parse a Profile param pod to extract profile info
fn parse_profile_param(pod: &Pod) -> Option<ProfileInfo> {
    let value = match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
        Ok((_, val)) => val,
        Err(_) => return None,
    };

    let Value::Object(obj) = value else {
        return None;
    };

    let mut index: Option<u32> = None;
    let mut name = String::new();
    let mut description = String::new();
    let mut priority: i32 = 0;
    let mut available: u32 = 0;
    let mut sink_devices: Vec<u32> = Vec::new();
    let mut source_devices: Vec<u32> = Vec::new();

    for prop in obj.properties {
        match prop.key {
            k if k == SPA_PARAM_PROFILE_index => match prop.value {
                Value::Int(v) => index = Some(v as u32),
                Value::Id(id) => index = Some(id.0),
                _ => {}
            },
            k if k == SPA_PARAM_PROFILE_name => {
                if let Value::String(s) = prop.value {
                    name = s;
                }
            }
            k if k == SPA_PARAM_PROFILE_description => {
                if let Value::String(s) = prop.value {
                    description = s;
                }
            }
            k if k == SPA_PARAM_PROFILE_priority => {
                if let Value::Int(v) = prop.value {
                    priority = v;
                }
            }
            k if k == SPA_PARAM_PROFILE_available => match prop.value {
                Value::Id(id) => available = id.0,
                Value::Int(v) => available = v as u32,
                _ => {}
            },
            k if k == SPA_PARAM_PROFILE_classes => {
                let (sinks, sources) = parse_profile_classes(&prop.value);
                sink_devices = sinks;
                source_devices = sources;
            }
            _ => {}
        }
    }

    let index = index?;
    if description.is_empty() {
        description = name.clone();
    }

    Some(ProfileInfo {
        index,
        name,
        description,
        priority,
        available,
        sink_devices,
        source_devices,
    })
}

fn parse_profile_index(pod: &Pod) -> Option<u32> {
    let value = match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
        Ok((_, val)) => val,
        Err(_) => return None,
    };

    let Value::Object(obj) = value else {
        return None;
    };

    for prop in obj.properties {
        if prop.key == SPA_PARAM_PROFILE_index {
            match prop.value {
                Value::Int(v) => return Some(v as u32),
                Value::Id(id) => return Some(id.0),
                _ => {}
            }
        }
    }

    None
}

fn parse_profile_classes(value: &Value) -> (Vec<u32>, Vec<u32>) {
    let Value::Struct(items) = value else {
        return (Vec::new(), Vec::new());
    };

    let mut iter = items.iter();
    let (count, mut entries_iter): (usize, Box<dyn Iterator<Item = &Value>>) = match iter.next() {
        Some(Value::Int(v)) => (*v as usize, Box::new(iter)),
        Some(Value::String(_)) => (items.len() / 4, Box::new(items.iter())),
        _ => return (Vec::new(), Vec::new()),
    };

    let mut sink_devices = Vec::new();
    let mut source_devices = Vec::new();

    for _ in 0..count {
        let entry_values: Vec<&Value> = match entries_iter.next() {
            Some(Value::Struct(entry)) => entry.iter().collect(),
            Some(first) => {
                let mut vals = vec![first];
                for _ in 0..3 {
                    if let Some(next) = entries_iter.next() {
                        vals.push(next);
                    }
                }
                vals
            }
            None => break,
        };

        if entry_values.len() < 4 {
            continue;
        }

        let class_name = match entry_values[0] {
            Value::String(s) => s.as_str(),
            _ => continue,
        };

        let device_indices: Vec<u32> = match entry_values[3] {
            Value::ValueArray(ValueArray::Int(vals)) => vals.iter().map(|v| *v as u32).collect(),
            Value::ValueArray(ValueArray::Id(vals)) => vals.iter().map(|v| v.0).collect(),
            _ => Vec::new(),
        };

        match class_name {
            "Audio/Sink" => sink_devices.extend(device_indices),
            "Audio/Source" => source_devices.extend(device_indices),
            _ => {}
        }
    }

    (sink_devices, source_devices)
}

fn maybe_clear_pending_profile(entry: &mut ProfileState, new_active: Option<u32>, device_id: u32) {
    let Some(pending) = entry.pending_index else {
        return;
    };

    if Some(pending) == new_active {
        if pw_debug_enabled("profile") {
            log::info!(
                "PROFILE-DBG pending cleared device={} idx={} (matched active)",
                device_id,
                pending
            );
        }
        entry.pending_index = None;
        entry.pending_since = None;
        return;
    }

    if let Some(since) = entry.pending_since {
        if since.elapsed() > PROFILE_PENDING_TIMEOUT {
            if pw_debug_enabled("profile") {
                log::info!(
                    "PROFILE-DBG pending cleared device={} idx={} (timeout)",
                    device_id,
                    pending
                );
            }
            entry.pending_index = None;
            entry.pending_since = None;
        }
    }
}

fn emit_profiles_update<F>(device_id: u32, state: &ProfileState, callback: &F)
where
    F: Fn(AudioEvent),
{
    let mut profiles: Vec<ProfileInfo> = state.profiles.values().cloned().collect();
    profiles.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.display_name().cmp(b.display_name()))
    });

    let active_index = state.pending_index.or(state.active_index);
    callback(AudioEvent::ProfilesUpdated(
        device_id,
        profiles,
        active_index,
    ));
}

/// Parse volume and mute from Props pod
/// Returns None if no volume/mute properties found (to avoid overwriting with defaults)
fn parse_audio_props(pod: &Pod) -> Option<(f64, bool, u32)> {
    let mut volume_prop: Option<f64> = None;
    let mut channel_avg: Option<f64> = None;
    let mut muted: Option<bool> = None;
    let mut channel_count: u32 = 2;

    // Deserialize the pod to a Value
    let value = match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
        Ok((_, val)) => val,
        Err(_) => return None,
    };

    if let Value::Object(obj) = value {
        for prop in obj.properties {
            match prop.key {
                k if k == SPA_PROP_volume => {
                    if let Value::Float(v) = prop.value {
                        volume_prop = Some(v as f64);
                    }
                }
                k if k == SPA_PROP_mute => {
                    if let Value::Bool(m) = prop.value {
                        muted = Some(m);
                    }
                }
                k if k == SPA_PROP_channelVolumes => {
                    if let Value::ValueArray(ValueArray::Float(vols)) = prop.value {
                        channel_count = vols.len() as u32;
                        if !vols.is_empty() {
                            let avg: f32 = vols.iter().sum::<f32>() / vols.len() as f32;
                            channel_avg = Some(avg as f64);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let volume = channel_avg.or(volume_prop);

    // Only return if we found at least volume or mute data
    if volume.is_some() || muted.is_some() {
        Some((volume.unwrap_or(1.0), muted.unwrap_or(false), channel_count))
    } else {
        None
    }
}

/// Set volume for a node via PipeWire
fn set_node_volume(
    nodes: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    pw_devices: &Rc<RefCell<HashMap<u32, PwDeviceState>>>,
    route_availability: &Rc<RefCell<HashMap<u32, Vec<RouteInfo>>>>,
    id: u32,
    volume: f64,
) {
    let devs = nodes.borrow();
    let Some(state) = devs.get(&id) else {
        log::warn!("set_node_volume: node {} not found", id);
        return;
    };

    let driver_state = state.driver_id.and_then(|driver| devs.get(&driver));

    let target_state = driver_state.unwrap_or(state);
    let volume_f32 = volume as f32;
    let vol_debug = pw_debug_enabled("volume");

    if vol_debug {
        log::info!(
            "VOL-DBG request id={} name='{}' target={} parent_device={:?} route_device_index={:?} route_key='{}' type={:?}",
            state.info.id,
            state.info.name,
            target_state.info.id,
            target_state.parent_device_id,
            target_state.route_device_index,
            target_state.route_key,
            target_state.info.device_type
        );
    }

    // Try to set volume via device Route param (WirePlumber does this)
    if let Some(device_id) = target_state.parent_device_id {
        if let Some(routes) = route_availability.borrow().get(&device_id) {
            if vol_debug {
                log::info!("VOL-DBG cached routes for device {}:", device_id);
                for route in routes {
                    log::info!(
                        "VOL-DBG cached route idx={} dev={} dir={} avail={} name='{}'",
                        route.index,
                        route.device,
                        route.direction,
                        route.available,
                        route.name
                    );
                }
            }

            if let Some(route_info) = find_route_for_state(target_state, routes) {
                if route_info.index >= 0 {
                    if let Some(device_state) = pw_devices.borrow().get(&device_id) {
                        let channel_count = target_state.channel_count.max(1) as usize;
                        let volumes: Vec<f32> = vec![volume_f32; channel_count];

                        let route_device = if route_info.device >= 0 {
                            route_info.device
                        } else {
                            target_state
                                .route_device_index
                                .map(|v| v as i32)
                                .unwrap_or(-1)
                        };

                        if route_device < 0 {
                            if vol_debug {
                                log::info!(
                                    "VOL-DBG route info missing device id: idx={} dev={}",
                                    route_info.index,
                                    route_info.device
                                );
                            }
                        } else {
                            if vol_debug {
                                log::info!(
                                    "VOL-DBG using route idx={} dev={} dir={} name='{}'",
                                    route_info.index,
                                    route_device,
                                    route_info.direction,
                                    route_info.name
                                );
                            } else {
                                log::debug!(
                                    "Setting volume via route for device {} (route index {}, device {}): {:.3}",
                                    device_id,
                                    route_info.index,
                                    route_device,
                                    volume
                                );
                            }

                            let props_obj = Object {
                                type_: SPA_TYPE_OBJECT_Props,
                                id: SPA_PARAM_Props,
                                properties: vec![
                                    Property {
                                        key: SPA_PROP_channelVolumes,
                                        flags: PropertyFlags::empty(),
                                        value: Value::ValueArray(ValueArray::Float(volumes)),
                                    },
                                    Property {
                                        key: SPA_PROP_volume,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Float(volume_f32),
                                    },
                                ],
                            };

                            let route_obj = Object {
                                type_: SPA_TYPE_OBJECT_ParamRoute,
                                id: SPA_PARAM_Route,
                                properties: vec![
                                    Property {
                                        key: SPA_PARAM_ROUTE_index,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Int(route_info.index),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_device,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Int(route_device),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_props,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Object(props_obj),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_save,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Bool(true),
                                    },
                                ],
                            };

                            let mut buffer = vec![0u8; 512];
                            let result = PodSerializer::serialize(
                                Cursor::new(&mut buffer),
                                &Value::Object(route_obj),
                            );

                            match result {
                                Ok((_, len)) => {
                                    buffer.truncate(len as usize);
                                    if let Some(pod) = Pod::from_bytes(&buffer) {
                                        device_state.device.set_param(ParamType::Route, 0, pod);
                                        if vol_debug {
                                            device_state.device.enum_params(
                                                0,
                                                Some(ParamType::Route),
                                                0,
                                                u32::MAX,
                                            );
                                        }
                                        return;
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to serialize route volume pod: {:?}", e);
                                }
                            }
                        }
                    } else if vol_debug {
                        log::info!(
                            "VOL-DBG no PipeWire device state cached for device {}",
                            device_id
                        );
                    }
                } else if vol_debug {
                    log::info!(
                        "VOL-DBG route info missing index: idx={} dev={}",
                        route_info.index,
                        route_info.device
                    );
                }
            } else if vol_debug {
                log::info!(
                    "VOL-DBG no matching route for target {} (route_device_index={:?}, route_key='{}')",
                    target_state.info.id,
                    target_state.route_device_index,
                    target_state.route_key
                );
            }
        } else if vol_debug {
            log::info!("VOL-DBG no cached routes for device {}", device_id);
        }
    } else if vol_debug {
        log::info!(
            "VOL-DBG target {} has no parent device id (cannot set route)",
            target_state.info.id
        );
    }

    // Fallback: set node props directly
    let apply_volume = |target_state: &DeviceState| {
        let channel_count = target_state.channel_count.max(1) as usize;
        let volumes: Vec<f32> = vec![volume_f32; channel_count];

        let obj = Object {
            type_: SPA_TYPE_OBJECT_Props,
            id: SPA_PARAM_Props,
            properties: vec![
                Property {
                    key: SPA_PROP_channelVolumes,
                    flags: PropertyFlags::empty(),
                    value: Value::ValueArray(ValueArray::Float(volumes)),
                },
                Property {
                    key: SPA_PROP_volume,
                    flags: PropertyFlags::empty(),
                    value: Value::Float(volume_f32),
                },
            ],
        };

        let mut buffer = vec![0u8; 512];
        let result = PodSerializer::serialize(Cursor::new(&mut buffer), &Value::Object(obj));

        match result {
            Ok((_, len)) => {
                buffer.truncate(len as usize);
                if let Some(pod) = Pod::from_bytes(&buffer) {
                    target_state.node.set_param(ParamType::Props, 0, pod);
                }
            }
            Err(e) => {
                log::error!("Failed to serialize volume pod: {:?}", e);
            }
        }
    };

    if let Some(driver) = driver_state {
        log::debug!(
            "Setting volume for device {} (target {}, {} channels): {:.3}",
            id,
            driver.info.id,
            driver.channel_count.max(1),
            volume
        );
        apply_volume(driver);

        if driver.info.id != state.info.id {
            log::debug!(
                "Mirroring volume to wrapper device {} ({} channels): {:.3}",
                state.info.id,
                state.channel_count.max(1),
                volume
            );
            apply_volume(state);
        }
    } else {
        log::debug!(
            "Setting volume for device {} ({} channels): {:.3}",
            id,
            state.channel_count.max(1),
            volume
        );
        apply_volume(state);
    }
}

/// Set mute state for a node via PipeWire
fn set_node_mute(
    nodes: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    pw_devices: &Rc<RefCell<HashMap<u32, PwDeviceState>>>,
    route_availability: &Rc<RefCell<HashMap<u32, Vec<RouteInfo>>>>,
    id: u32,
    muted: bool,
) {
    let devs = nodes.borrow();
    let Some(state) = devs.get(&id) else {
        log::warn!("set_node_mute: node {} not found", id);
        return;
    };

    let driver_state = state.driver_id.and_then(|driver| devs.get(&driver));

    let target_state = driver_state.unwrap_or(state);

    // Try to set mute via device Route param (WirePlumber does this)
    if let Some(device_id) = target_state.parent_device_id {
        if let Some(routes) = route_availability.borrow().get(&device_id) {
            if let Some(route_info) = find_route_for_state(target_state, routes) {
                if route_info.index >= 0 {
                    if let Some(device_state) = pw_devices.borrow().get(&device_id) {
                        let route_device = if route_info.device >= 0 {
                            route_info.device
                        } else {
                            target_state
                                .route_device_index
                                .map(|v| v as i32)
                                .unwrap_or(-1)
                        };

                        if route_device >= 0 {
                            log::debug!(
                                "Setting mute via route for device {} (route index {}, device {}): {}",
                                device_id,
                                route_info.index,
                                route_device,
                                muted
                            );

                            let props_obj = Object {
                                type_: SPA_TYPE_OBJECT_Props,
                                id: SPA_PARAM_Props,
                                properties: vec![Property {
                                    key: SPA_PROP_mute,
                                    flags: PropertyFlags::empty(),
                                    value: Value::Bool(muted),
                                }],
                            };

                            let route_obj = Object {
                                type_: SPA_TYPE_OBJECT_ParamRoute,
                                id: SPA_PARAM_Route,
                                properties: vec![
                                    Property {
                                        key: SPA_PARAM_ROUTE_index,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Int(route_info.index),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_device,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Int(route_device),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_props,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Object(props_obj),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_save,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Bool(true),
                                    },
                                ],
                            };

                            let mut buffer = vec![0u8; 256];
                            let result = PodSerializer::serialize(
                                Cursor::new(&mut buffer),
                                &Value::Object(route_obj),
                            );

                            match result {
                                Ok((_, len)) => {
                                    buffer.truncate(len as usize);
                                    if let Some(pod) = Pod::from_bytes(&buffer) {
                                        device_state.device.set_param(ParamType::Route, 0, pod);
                                        return;
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to serialize route mute pod: {:?}", e);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let apply_mute = |target_state: &DeviceState| {
        let obj = Object {
            type_: SPA_TYPE_OBJECT_Props,
            id: SPA_PARAM_Props,
            properties: vec![Property {
                key: SPA_PROP_mute,
                flags: PropertyFlags::empty(),
                value: Value::Bool(muted),
            }],
        };

        let mut buffer = vec![0u8; 128];
        let result = PodSerializer::serialize(Cursor::new(&mut buffer), &Value::Object(obj));

        match result {
            Ok((_, len)) => {
                buffer.truncate(len as usize);
                if let Some(pod) = Pod::from_bytes(&buffer) {
                    target_state.node.set_param(ParamType::Props, 0, pod);
                }
            }
            Err(e) => {
                log::error!("Failed to serialize mute pod: {:?}", e);
            }
        }
    };

    if let Some(driver) = driver_state {
        log::debug!(
            "Setting mute for device {} (target {}): {}",
            id,
            driver.info.id,
            muted
        );
        apply_mute(driver);

        if driver.info.id != state.info.id {
            log::debug!(
                "Mirroring mute to wrapper device {}: {}",
                state.info.id,
                muted
            );
            apply_mute(state);
        }
    } else {
        log::debug!("Setting mute for device {}: {}", id, muted);
        apply_mute(state);
    }
}

/// Set active profile for a device (card)
fn set_device_profile<F>(
    pw_devices: &Rc<RefCell<HashMap<u32, PwDeviceState>>>,
    profiles: &Rc<RefCell<HashMap<u32, ProfileState>>>,
    device_id: u32,
    profile_index: u32,
    event_callback: &F,
) where
    F: Fn(AudioEvent),
{
    let devices = pw_devices.borrow();
    let Some(state) = devices.get(&device_id) else {
        log::warn!("set_device_profile: device {} not found", device_id);
        return;
    };

    log::debug!(
        "Setting profile for device {} to index {}",
        device_id,
        profile_index
    );

    let updated_state = {
        let mut profiles_map = profiles.borrow_mut();
        let entry = profiles_map.entry(device_id).or_default();
        entry.pending_index = Some(profile_index);
        entry.pending_since = Some(Instant::now());
        entry.clone()
    };
    if pw_debug_enabled("profile") {
        log::info!(
            "PROFILE-DBG pending device={} idx={} (user requested)",
            device_id,
            profile_index
        );
    }
    emit_profiles_update(device_id, &updated_state, event_callback);

    let obj = Object {
        type_: SPA_TYPE_OBJECT_ParamProfile,
        id: SPA_PARAM_Profile,
        properties: vec![
            Property {
                key: SPA_PARAM_PROFILE_index,
                flags: PropertyFlags::empty(),
                value: Value::Int(profile_index as i32),
            },
            Property {
                key: SPA_PARAM_PROFILE_save,
                flags: PropertyFlags::empty(),
                value: Value::Bool(true),
            },
        ],
    };

    let mut buffer = vec![0u8; 256];
    let result = PodSerializer::serialize(Cursor::new(&mut buffer), &Value::Object(obj));

    match result {
        Ok((_, len)) => {
            buffer.truncate(len as usize);
            if let Some(pod) = Pod::from_bytes(&buffer) {
                state.device.set_param(ParamType::Profile, 0, pod);
                // Refresh profile information after change (Bluetooth codecs may update availability)
                state
                    .device
                    .enum_params(0, Some(ParamType::Profile), 0, u32::MAX);
                state
                    .device
                    .enum_params(0, Some(ParamType::EnumProfile), 0, u32::MAX);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize profile pod: {:?}", e);
        }
    }
}

/// Handle metadata object added (for tracking default devices)
fn handle_metadata_added<F>(
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    registry: &pipewire::registry::Registry,
    metadata_state: &Rc<RefCell<Option<MetadataState>>>,
    devices: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    default_sink_name: &Rc<RefCell<Option<String>>>,
    default_source_name: &Rc<RefCell<Option<String>>>,
    event_callback: Rc<F>,
) where
    F: Fn(AudioEvent) + 'static,
{
    let props = match global.props {
        Some(props) => props,
        None => return,
    };

    // Only interested in the "default" metadata
    let metadata_name = props.get("metadata.name").unwrap_or("");
    if metadata_name != "default" {
        return;
    }

    log::debug!("Found default metadata object (id={})", global.id);

    let metadata: Metadata = match registry.bind(global) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Failed to bind metadata: {}", e);
            return;
        }
    };

    let default_sink_name_for_listener = default_sink_name.clone();
    let default_source_name_for_listener = default_source_name.clone();
    let devices_for_listener = devices.clone();
    let event_callback_for_listener = event_callback.clone();

    let listener = metadata
        .add_listener_local()
        .property(move |_subject, key, _type, value| {
            let Some(key) = key else { return 0 };

            match key {
                "default.audio.sink" => {
                    let new_name = value.and_then(parse_metadata_name);
                    log::debug!("Default sink changed: {:?}", new_name);

                    let old_name = default_sink_name_for_listener.borrow().clone();
                    if old_name != new_name {
                        *default_sink_name_for_listener.borrow_mut() = new_name.clone();

                        // Find device ID by name and emit event
                        let devs = devices_for_listener.borrow();
                        let mut default_id = None;
                        for state in devs.values() {
                            if state.info.device_type == DeviceType::Sink {
                                if new_name.as_ref() == Some(&state.info.name) {
                                    default_id = Some(state.info.id);
                                    break;
                                }
                            }
                        }
                        drop(devs);
                        event_callback_for_listener(AudioEvent::DefaultChanged(
                            DeviceType::Sink,
                            default_id,
                        ));
                    }
                }
                "default.audio.source" => {
                    let new_name = value.and_then(parse_metadata_name);
                    log::debug!("Default source changed: {:?}", new_name);

                    let old_name = default_source_name_for_listener.borrow().clone();
                    if old_name != new_name {
                        *default_source_name_for_listener.borrow_mut() = new_name.clone();

                        let devs = devices_for_listener.borrow();
                        let mut default_id = None;
                        for state in devs.values() {
                            if state.info.device_type == DeviceType::Source {
                                if new_name.as_ref() == Some(&state.info.name) {
                                    default_id = Some(state.info.id);
                                    break;
                                }
                            }
                        }
                        drop(devs);
                        event_callback_for_listener(AudioEvent::DefaultChanged(
                            DeviceType::Source,
                            default_id,
                        ));
                    }
                }
                _ => {}
            }
            0
        })
        .register();

    *metadata_state.borrow_mut() = Some(MetadataState {
        metadata,
        _listener: listener,
    });
}

/// Parse device name from metadata JSON value (e.g., {"name":"alsa_output.pci..."})
fn parse_metadata_name(json: &str) -> Option<String> {
    // Simple JSON parsing for {"name":"value"} format
    let json = json.trim();
    if !json.starts_with('{') || !json.ends_with('}') {
        return None;
    }

    // Find "name" key
    let name_key = "\"name\"";
    let pos = json.find(name_key)?;
    let rest = &json[pos + name_key.len()..];

    // Skip whitespace and colon
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();

    // Extract string value
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Set default device via metadata
fn set_default_device(
    devices: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    pw_devices: &Rc<RefCell<HashMap<u32, PwDeviceState>>>,
    route_availability: &Rc<RefCell<HashMap<u32, Vec<RouteInfo>>>>,
    metadata_state: &Rc<RefCell<Option<MetadataState>>>,
    id: u32,
    device_type: DeviceType,
) {
    let devs = devices.borrow();
    let Some(state) = devs.get(&id) else {
        log::warn!("set_default_device: device {} not found", id);
        return;
    };

    let device_name = state.info.name.clone();
    let route_debug = pw_debug_enabled("route");

    // Ensure the device route is activated so WirePlumber considers this node
    // available when honoring default.configured.*
    if let Some(device_id) = state.parent_device_id {
        if let Some(routes) = route_availability.borrow().get(&device_id) {
            if let Some(route_info) = find_route_for_state(state, routes) {
                if route_info.index >= 0 {
                    if let Some(device_state) = pw_devices.borrow().get(&device_id) {
                        let route_device = if route_info.device >= 0 {
                            route_info.device
                        } else {
                            state.route_device_index.map(|v| v as i32).unwrap_or(-1)
                        };

                        if route_device >= 0 {
                            if route_debug {
                                log::info!(
                                    "ROUTE-DBG selecting route for device {}: idx={} dev={} name='{}'",
                                    device_id,
                                    route_info.index,
                                    route_device,
                                    route_info.name
                                );
                            }

                            let route_obj = Object {
                                type_: SPA_TYPE_OBJECT_ParamRoute,
                                id: SPA_PARAM_Route,
                                properties: vec![
                                    Property {
                                        key: SPA_PARAM_ROUTE_index,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Int(route_info.index),
                                    },
                                    Property {
                                        key: SPA_PARAM_ROUTE_device,
                                        flags: PropertyFlags::empty(),
                                        value: Value::Int(route_device),
                                    },
                                ],
                            };

                            let mut buffer = vec![0u8; 256];
                            if let Ok((_, len)) = PodSerializer::serialize(
                                Cursor::new(&mut buffer),
                                &Value::Object(route_obj),
                            ) {
                                buffer.truncate(len as usize);
                                if let Some(pod) = Pod::from_bytes(&buffer) {
                                    device_state.device.set_param(ParamType::Route, 0, pod);
                                }
                            }
                        } else if route_debug {
                            log::info!(
                                "ROUTE-DBG missing route device for id {} (route idx={})",
                                device_id,
                                route_info.index
                            );
                        }
                    } else if route_debug {
                        log::info!("ROUTE-DBG no PipeWire device for id {}", device_id);
                    }
                } else if route_debug {
                    log::info!(
                        "ROUTE-DBG no route index for device {} (route dev={})",
                        device_id,
                        route_info.device
                    );
                }
            } else if route_debug {
                log::info!(
                    "ROUTE-DBG no matching route for node {} (device={}, key='{}')",
                    state.info.id,
                    device_id,
                    state.route_key
                );
            }
        } else if route_debug {
            log::info!("ROUTE-DBG no cached routes for device {}", device_id);
        }
    }
    drop(devs);

    let meta_state = metadata_state.borrow();
    let Some(meta) = meta_state.as_ref() else {
        log::warn!("set_default_device: no metadata object available");
        return;
    };

    // WirePlumber expects configured defaults to be written to the
    // default.configured.* keys, and then updates default.audio.* itself.
    let key = match device_type {
        DeviceType::Sink => "default.configured.audio.sink",
        DeviceType::Source => "default.configured.audio.source",
    };

    let json_value = format!(r#"{{"name":"{}"}}"#, device_name);
    log::debug!("Setting {} to {}", key, json_value);

    meta.metadata
        .set_property(0, key, Some("Spa:String:JSON"), Some(&json_value));
}
