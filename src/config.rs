// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! Configuration of the systemd compute driver.

use std::path::PathBuf;

use openshell_core::resource_admission::ResourceAdmissionConfig;

/// Where the helper listens unless configured otherwise.
pub const DEFAULT_HELPER_SOCKET: &str = "/run/openshell-systemd-helper/helper.sock";

/// Configuration of the systemd compute driver.
#[derive(Clone, Debug)]
pub struct SystemdComputeConfig {
    /// Whether sandbox requests may carry driver-specific configuration.
    pub allow_driver_config: bool,
    /// The resource admission policy the driver acknowledges.
    pub resource_admission: ResourceAdmissionConfig,
    /// The privileged helper's socket.
    pub helper_socket: PathBuf,
    /// The gateway endpoint supervisors dial.
    pub grpc_endpoint: String,
    /// The gateway's name, for the supervisor's telemetry and logs.
    pub gateway_name: Option<String>,
    /// The CA supervisors verify the gateway with. Required when the
    /// endpoint is HTTPS and the gateway's certificate is not publicly
    /// trusted.
    pub gateway_tls_ca: Option<PathBuf>,
    /// How often the units' state is read for the watch stream.
    pub poll_interval: std::time::Duration,
}

impl Default for SystemdComputeConfig {
    fn default() -> Self {
        Self {
            allow_driver_config: false,
            resource_admission: ResourceAdmissionConfig::default(),
            helper_socket: PathBuf::from(DEFAULT_HELPER_SOCKET),
            grpc_endpoint: String::new(),
            gateway_name: None,
            gateway_tls_ca: None,
            poll_interval: std::time::Duration::from_secs(2),
        }
    }
}
