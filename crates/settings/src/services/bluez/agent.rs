//! BluezAgent - D-Bus service implementing org.bluez.Agent1 interface.
//!
//! This agent handles pairing requests from BlueZ, showing confirmation
//! dialogs to the user when devices want to pair.

use std::cell::RefCell;
use std::rc::Rc;

use glib::variant::ObjectPath;

/// The object path where our agent is registered.
pub const AGENT_PATH: &str = "/org/swaysettings/agent";

/// Agent capability - we can display a passkey and get yes/no confirmation.
pub const AGENT_CAPABILITY: &str = "DisplayYesNo";

/// Request types the agent can handle.
#[derive(Debug, Clone)]
pub enum AgentRequest {
    /// Request confirmation of a passkey (user sees passkey, confirms on both devices).
    RequestConfirmation { device_path: String, passkey: u32 },
    /// Display a passkey that the user should enter on the remote device.
    DisplayPasskey {
        device_path: String,
        passkey: u32,
        entered: u16,
    },
    /// Display a PIN code that the user should enter on the remote device.
    DisplayPinCode {
        device_path: String,
        pincode: String,
    },
    /// Request authorization for pairing (no passkey).
    RequestAuthorization { device_path: String },
    /// Authorize a service UUID.
    AuthorizeService { device_path: String, uuid: String },
    /// Cancel the current request.
    Cancel,
}

/// Callback type for agent requests.
pub type AgentCallback = Rc<RefCell<Option<Box<dyn Fn(AgentRequest) -> Option<bool>>>>>;

/// BlueZ Agent1 D-Bus service.
///
/// Exports an object that BlueZ can call to request user confirmation
/// during pairing operations.
pub struct BluezAgent {
    connection: gio::DBusConnection,
    registration_id: Option<gio::RegistrationId>,
    callback: AgentCallback,
}

impl BluezAgent {
    /// Create and register a new agent on the system bus.
    pub async fn new() -> Result<Self, glib::Error> {
        let connection = gio::bus_get_future(gio::BusType::System).await?;
        let callback: AgentCallback = Rc::new(RefCell::new(None));

        let registration_id = Self::register_object(&connection, callback.clone())?;

        Ok(Self {
            connection,
            registration_id: Some(registration_id),
            callback,
        })
    }

    /// Get the agent's object path.
    pub fn object_path(&self) -> &str {
        AGENT_PATH
    }

    /// Get the agent's capability string.
    pub fn capability(&self) -> &str {
        AGENT_CAPABILITY
    }

    /// Set the callback for handling agent requests.
    ///
    /// The callback receives an AgentRequest and should return:
    /// - Some(true) to accept/confirm the request
    /// - Some(false) to reject the request
    /// - None for requests that don't need a response (DisplayPasskey, Cancel)
    pub fn set_callback<F>(&self, callback: F)
    where
        F: Fn(AgentRequest) -> Option<bool> + 'static,
    {
        *self.callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Clear the callback.
    pub fn clear_callback(&self) {
        *self.callback.borrow_mut() = None;
    }

    fn register_object(
        connection: &gio::DBusConnection,
        callback: AgentCallback,
    ) -> Result<gio::RegistrationId, glib::Error> {
        let introspection_xml = r#"
            <node>
                <interface name="org.bluez.Agent1">
                    <method name="Release"/>
                    <method name="RequestPinCode">
                        <arg type="o" name="device" direction="in"/>
                        <arg type="s" name="pincode" direction="out"/>
                    </method>
                    <method name="DisplayPinCode">
                        <arg type="o" name="device" direction="in"/>
                        <arg type="s" name="pincode" direction="in"/>
                    </method>
                    <method name="RequestPasskey">
                        <arg type="o" name="device" direction="in"/>
                        <arg type="u" name="passkey" direction="out"/>
                    </method>
                    <method name="DisplayPasskey">
                        <arg type="o" name="device" direction="in"/>
                        <arg type="u" name="passkey" direction="in"/>
                        <arg type="q" name="entered" direction="in"/>
                    </method>
                    <method name="RequestConfirmation">
                        <arg type="o" name="device" direction="in"/>
                        <arg type="u" name="passkey" direction="in"/>
                    </method>
                    <method name="RequestAuthorization">
                        <arg type="o" name="device" direction="in"/>
                    </method>
                    <method name="AuthorizeService">
                        <arg type="o" name="device" direction="in"/>
                        <arg type="s" name="uuid" direction="in"/>
                    </method>
                    <method name="Cancel"/>
                </interface>
            </node>
        "#;

        let node_info = gio::DBusNodeInfo::for_xml(introspection_xml)?;
        let interface_info = node_info
            .lookup_interface("org.bluez.Agent1")
            .expect("Agent1 interface not found in introspection data");

        let registration_id = connection
            .register_object(AGENT_PATH, &interface_info)
            .method_call(
                move |_conn, _sender, _path, _iface, method, params, invocation| {
                    Self::handle_method_call(&callback, &method, params, invocation);
                },
            )
            .build()?;

        Ok(registration_id)
    }

    fn handle_method_call(
        callback: &AgentCallback,
        method: &str,
        params: glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        log::debug!("Agent method called: {} with params: {:?}", method, params);

        match method {
            "Release" => {
                // Agent is being released, nothing to do
                invocation.return_value(None);
            }
            "RequestPinCode" => {
                // We don't support PIN code input, reject
                invocation.return_error(
                    gio::IOErrorEnum::NotSupported,
                    "PIN code input not supported",
                );
            }
            "RequestPasskey" => {
                // We don't support passkey input, reject
                invocation.return_error(
                    gio::IOErrorEnum::NotSupported,
                    "Passkey input not supported",
                );
            }
            "DisplayPinCode" => {
                if let Some((device_path, pincode)) = params.get::<(ObjectPath, String)>() {
                    let request = AgentRequest::DisplayPinCode {
                        device_path: device_path.to_string(),
                        pincode,
                    };
                    if let Some(ref cb) = *callback.borrow() {
                        cb(request);
                    }
                }
                invocation.return_value(None);
            }
            "DisplayPasskey" => {
                if let Some((device_path, passkey, entered)) =
                    params.get::<(ObjectPath, u32, u16)>()
                {
                    let request = AgentRequest::DisplayPasskey {
                        device_path: device_path.to_string(),
                        passkey,
                        entered,
                    };
                    if let Some(ref cb) = *callback.borrow() {
                        cb(request);
                    }
                }
                invocation.return_value(None);
            }
            "RequestConfirmation" => {
                if let Some((device_path, passkey)) = params.get::<(ObjectPath, u32)>() {
                    let request = AgentRequest::RequestConfirmation {
                        device_path: device_path.to_string(),
                        passkey,
                    };
                    let accepted = callback
                        .borrow()
                        .as_ref()
                        .and_then(|cb| cb(request))
                        .unwrap_or(false);

                    if accepted {
                        invocation.return_value(None);
                    } else {
                        invocation
                            .return_error(gio::IOErrorEnum::Cancelled, "Pairing rejected by user");
                    }
                } else {
                    invocation.return_error(gio::IOErrorEnum::InvalidArgument, "Invalid arguments");
                }
            }
            "RequestAuthorization" => {
                if let Some((device_path,)) = params.get::<(ObjectPath,)>() {
                    let request = AgentRequest::RequestAuthorization {
                        device_path: device_path.to_string(),
                    };
                    let accepted = callback
                        .borrow()
                        .as_ref()
                        .and_then(|cb| cb(request))
                        .unwrap_or(false);

                    if accepted {
                        invocation.return_value(None);
                    } else {
                        invocation.return_error(
                            gio::IOErrorEnum::Cancelled,
                            "Authorization rejected by user",
                        );
                    }
                } else {
                    invocation.return_error(gio::IOErrorEnum::InvalidArgument, "Invalid arguments");
                }
            }
            "AuthorizeService" => {
                if let Some((device_path, uuid)) = params.get::<(ObjectPath, String)>() {
                    let request = AgentRequest::AuthorizeService {
                        device_path: device_path.to_string(),
                        uuid,
                    };
                    let accepted = callback
                        .borrow()
                        .as_ref()
                        .and_then(|cb| cb(request))
                        .unwrap_or(true); // Auto-accept service authorization

                    if accepted {
                        invocation.return_value(None);
                    } else {
                        invocation.return_error(
                            gio::IOErrorEnum::Cancelled,
                            "Service authorization rejected",
                        );
                    }
                } else {
                    invocation.return_error(gio::IOErrorEnum::InvalidArgument, "Invalid arguments");
                }
            }
            "Cancel" => {
                let request = AgentRequest::Cancel;
                if let Some(ref cb) = *callback.borrow() {
                    cb(request);
                }
                invocation.return_value(None);
            }
            _ => {
                invocation.return_error(
                    gio::IOErrorEnum::NotSupported,
                    &format!("Unknown method: {}", method),
                );
            }
        }
    }
}

impl Drop for BluezAgent {
    fn drop(&mut self) {
        if let Some(registration_id) = self.registration_id.take() {
            let _ = self.connection.unregister_object(registration_id);
        }
    }
}
