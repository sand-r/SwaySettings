//! BluezAgentManager - D-Bus proxy for org.bluez.AgentManager1 interface.

use gio::prelude::*;
use glib::variant::ObjectPath;

/// D-Bus proxy for BlueZ AgentManager1 interface.
///
/// Used to register pairing agents with BlueZ.
pub struct BluezAgentManager {
    proxy: gio::DBusProxy,
}

impl BluezAgentManager {
    /// Create a proxy for the agent manager.
    pub async fn new() -> Result<Self, glib::Error> {
        let proxy = gio::DBusProxy::for_bus_future(
            gio::BusType::System,
            gio::DBusProxyFlags::NONE,
            None::<&gio::DBusInterfaceInfo>,
            "org.bluez",
            "/org/bluez",
            "org.bluez.AgentManager1",
        )
        .await?;

        Ok(Self { proxy })
    }

    /// Register an agent at the given path with the specified capability.
    ///
    /// Capabilities can be:
    /// - "DisplayOnly" - Only display passkeys/pincodes
    /// - "DisplayYesNo" - Display and accept/reject
    /// - "KeyboardOnly" - Input passkey/pincode
    /// - "NoInputNoOutput" - Just works pairing
    /// - "KeyboardDisplay" - Full capabilities
    pub async fn register_agent(
        &self,
        agent_path: &str,
        capability: &str,
    ) -> Result<(), glib::Error> {
        let path = ObjectPath::try_from(agent_path)
            .map_err(|e| glib::Error::new(gio::IOErrorEnum::InvalidArgument, &e.to_string()))?;
        self.proxy
            .call_future(
                "RegisterAgent",
                Some(&(path, capability).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Unregister a previously registered agent.
    pub async fn unregister_agent(&self, agent_path: &str) -> Result<(), glib::Error> {
        let path = ObjectPath::try_from(agent_path)
            .map_err(|e| glib::Error::new(gio::IOErrorEnum::InvalidArgument, &e.to_string()))?;
        self.proxy
            .call_future(
                "UnregisterAgent",
                Some(&(path,).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }

    /// Request that the specified agent becomes the default agent.
    pub async fn request_default_agent(&self, agent_path: &str) -> Result<(), glib::Error> {
        let path = ObjectPath::try_from(agent_path)
            .map_err(|e| glib::Error::new(gio::IOErrorEnum::InvalidArgument, &e.to_string()))?;
        self.proxy
            .call_future(
                "RequestDefaultAgent",
                Some(&(path,).to_variant()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await?;
        Ok(())
    }
}
