use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

use pipewire as pw;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::registry::GlobalObject;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Pod, Property, PropertyFlags, Value, ValueArray};
use pipewire::spa::sys::{
    SPA_PARAM_Props, SPA_PROP_channelVolumes, SPA_PROP_mute, SPA_PROP_volume,
    SPA_TYPE_OBJECT_Props,
};
use pipewire::types::ObjectType;

/// Type of audio device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Sink,
    Source,
}

/// Information about an audio device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub volume: f64,
    pub is_muted: bool,
    pub is_default: bool,
    pub device_type: DeviceType,
}

/// Events sent from the PipeWire thread to the main GTK thread
#[derive(Debug)]
pub enum AudioEvent {
    Ready,
    DeviceAdded(DeviceInfo),
    DeviceRemoved(u32),
    DeviceChanged(DeviceInfo),
    DefaultChanged(DeviceType, Option<u32>),
    Error(String),
}

/// Commands sent from the main GTK thread to the PipeWire thread
#[derive(Debug)]
enum AudioCommand {
    SetVolume(u32, f64),
    SetMute(u32, bool),
    SetDefaultSink(u32),
    SetDefaultSource(u32),
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
    let devices: Rc<RefCell<HashMap<u32, DeviceState>>> = Rc::new(RefCell::new(HashMap::new()));
    let default_sink: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(None));
    let default_source: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(None));

    // Clone for closures
    let devices_clone = devices.clone();
    let devices_for_remove = devices.clone();
    let devices_for_commands = devices.clone();
    let default_sink_clone = default_sink.clone();
    let default_source_clone = default_source.clone();
    let registry_for_bind = registry.clone();
    let registry_clone = registry.clone();

    // Wrap callback in Rc for sharing
    let event_callback = Rc::new(event_callback);
    let event_callback_for_global = event_callback.clone();
    let event_callback_for_remove = event_callback.clone();
    let event_callback_for_params = event_callback.clone();

    // Registry listener for global objects
    let _registry_listener = registry_for_bind
        .add_listener_local()
        .global(move |global| {
            handle_global_added(
                global,
                &registry_clone,
                &devices_clone,
                &default_sink_clone,
                &default_source_clone,
                event_callback_for_global.as_ref(),
                event_callback_for_params.clone(),
            );
        })
        .global_remove(move |id| {
            let mut devs = devices_for_remove.borrow_mut();
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
    let _receiver_handle = pw_receiver.attach(&loop_, move |cmd| {
        match cmd {
            AudioCommand::Shutdown => {
                *should_quit_clone.borrow_mut() = true;
            }
            AudioCommand::SetVolume(id, volume) => {
                set_device_volume(&devices_for_commands, id, volume);
            }
            AudioCommand::SetMute(id, muted) => {
                set_device_mute(&devices_for_commands, id, muted);
            }
            AudioCommand::SetDefaultSink(_id) => {
                // Changing default requires WirePlumber metadata - not implemented yet
                log::debug!("SetDefaultSink not yet implemented");
            }
            AudioCommand::SetDefaultSource(_id) => {
                log::debug!("SetDefaultSource not yet implemented");
            }
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

fn handle_global_added<F>(
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    registry: &pipewire::registry::Registry,
    devices: &Rc<RefCell<HashMap<u32, DeviceState>>>,
    default_sink: &Rc<RefCell<Option<u32>>>,
    default_source: &Rc<RefCell<Option<u32>>>,
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

    let description = props
        .get("node.description")
        .or_else(|| props.get("node.nick"))
        .unwrap_or(&name)
        .to_string();

    if let Some(available) = props.get("port.available") {
        if available == "no" {
            return;
        }
    }

    let id = global.id;

    let info = DeviceInfo {
        id,
        name: name.clone(),
        description,
        volume: 1.0,
        is_muted: false,
        is_default: false,
        device_type,
    };

    let node: pipewire::node::Node = match registry.bind(global) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to bind node {}: {}", id, e);
            return;
        }
    };

    let devices_for_listener = devices.clone();
    let default_sink_for_listener = default_sink.clone();
    let default_source_for_listener = default_source.clone();

    let listener = node
        .add_listener_local()
        .param(move |_seq, param_type, _index, _next, param| {
            if param_type != ParamType::Props {
                return;
            }

            let Some(pod) = param else { return };

            if let Some((volume, muted, channel_count)) = parse_audio_props(pod) {
                let mut devs = devices_for_listener.borrow_mut();
                if let Some(state) = devs.get_mut(&id) {
                    let changed = (state.info.volume - volume).abs() > 0.001
                        || state.info.is_muted != muted;

                    state.info.volume = volume;
                    state.info.is_muted = muted;
                    state.channel_count = channel_count;

                    let is_default = match state.info.device_type {
                        DeviceType::Sink => *default_sink_for_listener.borrow() == Some(id),
                        DeviceType::Source => *default_source_for_listener.borrow() == Some(id),
                    };
                    state.info.is_default = is_default;

                    if changed {
                        event_callback_rc(AudioEvent::DeviceChanged(state.info.clone()));
                    }
                }
            }
        })
        .register();

    node.subscribe_params(&[ParamType::Props]);

    let state = DeviceState {
        info: info.clone(),
        node,
        _listener: listener,
        channel_count: 2,
    };

    devices.borrow_mut().insert(id, state);
    event_callback(AudioEvent::DeviceAdded(info));
}

/// Parse volume and mute from Props pod
/// Returns None if no volume/mute properties found (to avoid overwriting with defaults)
fn parse_audio_props(pod: &Pod) -> Option<(f64, bool, u32)> {
    let mut volume: Option<f64> = None;
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
                        volume = Some(v as f64);
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
                            volume = Some(avg as f64);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Only return if we found at least volume or mute data
    if volume.is_some() || muted.is_some() {
        Some((volume.unwrap_or(1.0), muted.unwrap_or(false), channel_count))
    } else {
        None
    }
}

/// Set volume for a device via PipeWire
fn set_device_volume(devices: &Rc<RefCell<HashMap<u32, DeviceState>>>, id: u32, volume: f64) {
    let devs = devices.borrow();
    let Some(state) = devs.get(&id) else { return };

    let volume_f32 = volume as f32;
    let channel_count = state.channel_count.max(1) as usize;
    let volumes: Vec<f32> = vec![volume_f32; channel_count];

    // Build Props object with channelVolumes
    let obj = Object {
        type_: SPA_TYPE_OBJECT_Props,
        id: SPA_PARAM_Props,
        properties: vec![Property {
            key: SPA_PROP_channelVolumes,
            flags: PropertyFlags::empty(),
            value: Value::ValueArray(ValueArray::Float(volumes)),
        }],
    };

    // Serialize to bytes
    let mut buffer = vec![0u8; 512];
    let result = PodSerializer::serialize(Cursor::new(&mut buffer), &Value::Object(obj));

    match result {
        Ok((_, len)) => {
            buffer.truncate(len as usize);
            if let Some(pod) = Pod::from_bytes(&buffer) {
                state.node.set_param(ParamType::Props, 0, pod);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize volume pod: {:?}", e);
        }
    }
}

/// Set mute state for a device via PipeWire
fn set_device_mute(devices: &Rc<RefCell<HashMap<u32, DeviceState>>>, id: u32, muted: bool) {
    let devs = devices.borrow();
    let Some(state) = devs.get(&id) else { return };

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
                state.node.set_param(ParamType::Props, 0, pod);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize mute pod: {:?}", e);
        }
    }
}
