//! BlueZ D-Bus service integration.
//!
//! This module provides Rust wrappers for the BlueZ D-Bus API, enabling
//! Bluetooth device discovery, pairing, and connection management.

mod adapter;
mod agent;
mod agent_manager;
mod daemon;
mod device;

pub use agent::AgentRequest;
pub use daemon::BluezDaemon;
pub use device::BluezDevice;
