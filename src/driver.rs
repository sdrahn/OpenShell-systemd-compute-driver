// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! The systemd compute driver.
//!
//! A sandbox is two system units on the gateway's host: the stock
//! supervisor on the host's network, and the stock `openshell-sandbox` in a
//! network namespace with loopback only, both as one unprivileged host
//! user. The driver builds a launch's files and hands them to a privileged
//! helper, which writes them and starts the units from fixed templates.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::Stream;
use openshell_core::ComputeDriverError;
use openshell_core::driver_utils::{CONDITION_EXITED, CONDITION_STOPPED};
use openshell_core::proto::compute::v1::{
    DriverCondition, DriverSandbox, DriverSandboxStatus, GetCapabilitiesResponse,
    ResourceCapabilities, WatchSandboxesDeletedEvent, WatchSandboxesEvent,
    WatchSandboxesSandboxEvent, watch_sandboxes_event,
};
use openshell_isolation_interface::contract::ResolvedWorkloadIdentity;
use tracing::{debug, info, warn};

use crate::config::SystemdComputeConfig;
use crate::helper::HelperClient;
use crate::protocol::{Request, Response, SandboxRecord, UnitState, valid_sandbox_id};

/// The driver's name in the gateway's configuration.
pub const DRIVER_NAME: &str = "systemd";

/// The only image a sandbox of this driver may name: none. The workload
/// runs on the host's file system.
pub const HOST_IMAGE: &str = "host";

const CONDITION_RUNNING: &str = "UnitsActive";
const CONDITION_STARTING: &str = "UnitsStarting";

pub type WatchStream =
    Pin<Box<dyn Stream<Item = Result<WatchSandboxesEvent, ComputeDriverError>> + Send>>;

/// The systemd compute driver.
#[derive(Clone)]
pub struct SystemdComputeDriver {
    config: Arc<SystemdComputeConfig>,
    helper: HelperClient,
    identity: ResolvedWorkloadIdentity,
    events: tokio::sync::broadcast::Sender<WatchSandboxesEvent>,
    /// What the watch last reported for each sandbox.
    observed: Arc<Mutex<HashMap<String, SandboxRecord>>>,
}

impl std::fmt::Debug for SystemdComputeDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemdComputeDriver")
            .field("helper", &self.helper)
            .field("grpc_endpoint", &self.config.grpc_endpoint)
            .finish_non_exhaustive()
    }
}

fn sandbox_event(sandbox: DriverSandbox) -> WatchSandboxesEvent {
    WatchSandboxesEvent {
        payload: Some(watch_sandboxes_event::Payload::Sandbox(
            WatchSandboxesSandboxEvent {
                sandbox: Some(sandbox),
            },
        )),
    }
}

fn deleted_event(sandbox_id: String) -> WatchSandboxesEvent {
    WatchSandboxesEvent {
        payload: Some(watch_sandboxes_event::Payload::Deleted(
            WatchSandboxesDeletedEvent { sandbox_id },
        )),
    }
}

/// The units' state as the gateway's `Ready` condition.
fn condition(record: &SandboxRecord) -> DriverCondition {
    let (status, reason, message) = match record.state {
        UnitState::Active => ("True", CONDITION_RUNNING, "The sandbox's units are active"),
        UnitState::Transitioning => ("False", CONDITION_STARTING, record.detail.as_str()),
        UnitState::Inactive => ("False", CONDITION_STOPPED, ""),
        UnitState::Failed => ("False", CONDITION_EXITED, record.detail.as_str()),
    };
    DriverCondition {
        r#type: "Ready".to_string(),
        status: status.to_string(),
        reason: reason.to_string(),
        message: message.to_string(),
        transition_time: None,
    }
}

/// A sandbox as the gateway is told of it.
fn driver_sandbox(record: &SandboxRecord) -> DriverSandbox {
    DriverSandbox {
        id: record.sandbox_id.clone(),
        name: record.name.clone(),
        namespace: String::new(),
        spec: None,
        status: Some(DriverSandboxStatus {
            name: crate::isolation::workload_unit(&record.sandbox_id),
            instance_id: record.sandbox_id.chars().take(12).collect(),
            agent_fd: String::new(),
            sandbox_fd: String::new(),
            conditions: vec![condition(record)],
            deleting: false,
            ..Default::default()
        }),
        workspace: record.workspace.clone(),
    }
}

/// Refuses a requested workload user or group other than the one every
/// workload of this host runs as. `sandbox` names it, as it names the
/// image's sandbox account for the container drivers.
fn check_identity_request(
    user: &str,
    group: &str,
    identity: &ResolvedWorkloadIdentity,
) -> Result<(), ComputeDriverError> {
    let matches = |requested: &str, id: u32| {
        let requested = requested.trim();
        requested.is_empty() || requested == "sandbox" || requested.parse() == Ok(id)
    };
    if matches(user, identity.uid) && matches(group, identity.gid) {
        Ok(())
    } else {
        Err(ComputeDriverError::InvalidArgument(format!(
            "the systemd compute driver runs every workload as {}:{} and cannot run one as {user:?}:{group:?}",
            identity.uid, identity.gid
        )))
    }
}

fn check_sandbox_id(sandbox_id: &str) -> Result<(), ComputeDriverError> {
    if valid_sandbox_id(sandbox_id) {
        Ok(())
    } else {
        Err(ComputeDriverError::InvalidArgument(
            "the sandbox ID is not a canonical UUID".to_string(),
        ))
    }
}

impl SystemdComputeDriver {
    /// Connects to the helper and learns the identity workloads run as.
    pub async fn new(config: SystemdComputeConfig) -> Result<Self, ComputeDriverError> {
        let helper = HelperClient::new(config.helper_socket.clone());
        let (uid, gid) = match helper.call(&Request::Describe).await? {
            Response::Described { version, uid, gid } => {
                info!(helper_version = %version, uid, gid, "Connected to the systemd helper");
                (uid, gid)
            }
            other => {
                return Err(ComputeDriverError::Message(format!(
                    "systemd helper: unexpected answer {other:?}"
                )));
            }
        };
        let identity = crate::isolation::workload_identity(uid, gid)?;
        let (events, _) = tokio::sync::broadcast::channel(256);
        let driver = Self {
            config: Arc::new(config),
            helper,
            identity,
            events,
            observed: Arc::new(Mutex::new(HashMap::new())),
        };
        tokio::spawn(driver.clone().poll());
        Ok(driver)
    }

    pub fn capabilities(&self) -> GetCapabilitiesResponse {
        #[allow(deprecated)]
        GetCapabilitiesResponse {
            resource_admission_policy: openshell_core::resource_admission::DriverAdmissionConfig {
                allow_driver_config: self.config.allow_driver_config,
                resource_admission: self.config.resource_admission.clone(),
            }
            .acknowledgement(),
            driver_name: DRIVER_NAME.to_string(),
            driver_version: crate::VERSION.to_string(),
            default_image: HOST_IMAGE.to_string(),
            gateway_manages_lifecycle: true,
            supports_sandbox_authentication: false,
            driver_reports_runtime_readiness: false,
            // No CPU, memory or GPU request is honored yet.
            resource_capabilities: Some(ResourceCapabilities::default()),
            rootfs_tar_staging_dir: String::new(),
            rootfs_tar_max_bytes: 0,
            extension: Some(openshell_core::extension_protocol::extension_metadata(
                openshell_core::extension_protocol::ExtensionFamily::Compute,
                "openshell-driver-systemd",
                crate::VERSION,
                [],
            )),
        }
    }

    /// Refuses what this driver cannot give a sandbox, before anything is
    /// provisioned.
    pub fn validate_sandbox_create(
        &self,
        sandbox: &DriverSandbox,
    ) -> Result<(), ComputeDriverError> {
        check_sandbox_id(&sandbox.id)?;
        let image = sandbox
            .spec
            .as_ref()
            .and_then(|spec| spec.template.as_ref())
            .map_or("", |template| template.image.as_str());
        if !image.is_empty() && image != HOST_IMAGE {
            return Err(ComputeDriverError::InvalidArgument(format!(
                "the systemd compute driver runs workloads on the host's file system and takes no image (got {image:?})"
            )));
        }
        sandbox
            .spec
            .as_ref()
            .and_then(|spec| spec.workload_identity.as_ref())
            .map_or(Ok(()), |request| {
                check_identity_request(&request.user, &request.group, &self.identity)
            })
    }

    async fn launch(
        &self,
        sandbox: &DriverSandbox,
        launch_authentication: &[u8],
    ) -> Result<(), ComputeDriverError> {
        self.validate_sandbox_create(sandbox)?;
        if launch_authentication.is_empty() {
            return Err(ComputeDriverError::Precondition(
                "the sandbox has no launch authentication".to_string(),
            ));
        }
        let launch_authentication = serde_json::from_slice::<
            openshell_core::jwt::SandboxLaunchAuthentication,
        >(launch_authentication)
        .map_err(|error| {
            ComputeDriverError::Precondition(format!(
                "decode systemd sandbox launch authentication: {error}"
            ))
        })?;
        let gateway_ca = match &self.config.gateway_tls_ca {
            Some(path) => tokio::fs::read_to_string(path).await.map_err(|error| {
                ComputeDriverError::Precondition(format!(
                    "read the gateway CA {}: {error}",
                    path.display()
                ))
            })?,
            None => String::new(),
        };
        let provision = crate::isolation::provision(
            sandbox,
            &self.config,
            &self.identity,
            &launch_authentication,
            gateway_ca,
        )?;
        self.helper
            .done(&Request::Provision(Box::new(provision)))
            .await?;
        self.helper
            .done(&Request::Start {
                sandbox_id: sandbox.id.clone(),
            })
            .await
    }

    pub async fn create_sandbox(&self, sandbox: &DriverSandbox) -> Result<(), ComputeDriverError> {
        check_sandbox_id(&sandbox.id)?;
        if self.record(&sandbox.id).await?.is_some() {
            return Err(ComputeDriverError::AlreadyExists);
        }
        let launch_authentication = sandbox
            .spec
            .as_ref()
            .map(|spec| spec.launch_authentication.clone())
            .unwrap_or_default();
        info!(sandbox_id = %sandbox.id, sandbox_name = %sandbox.name, "Creating sandbox units");
        if let Err(error) = self.launch(sandbox, &launch_authentication).await {
            // Nothing of a launch that failed is left behind.
            let _ = self
                .helper
                .call(&Request::Delete {
                    sandbox_id: sandbox.id.clone(),
                })
                .await;
            return Err(error);
        }
        self.observe(&sandbox.id).await;
        Ok(())
    }

    pub async fn start_sandbox(
        &self,
        sandbox_id: &str,
        launch_authentication: &[u8],
        sandbox: Option<&DriverSandbox>,
    ) -> Result<(), ComputeDriverError> {
        check_sandbox_id(sandbox_id)?;
        if launch_authentication.is_empty() {
            // No new launch: starting what already runs is idempotent.
            return match self.record(sandbox_id).await? {
                Some(record) if record.state == UnitState::Active => Ok(()),
                Some(_) => Err(ComputeDriverError::Precondition(
                    "starting stopped sandbox units needs a launch authentication".to_string(),
                )),
                None => Err(ComputeDriverError::NotFound),
            };
        }
        let sandbox = sandbox.ok_or_else(|| {
            ComputeDriverError::Precondition(
                "starting a sandbox needs its specification for the new launch".to_string(),
            )
        })?;
        if sandbox.id != sandbox_id {
            return Err(ComputeDriverError::InvalidArgument(
                "the sandbox specification is for another sandbox".to_string(),
            ));
        }
        info!(%sandbox_id, "Starting sandbox units");
        self.launch(sandbox, launch_authentication).await?;
        self.observe(sandbox_id).await;
        Ok(())
    }

    pub async fn stop_sandbox(&self, sandbox_id: &str) -> Result<(), ComputeDriverError> {
        check_sandbox_id(sandbox_id)?;
        info!(%sandbox_id, "Stopping sandbox units");
        match self
            .helper
            .done(&Request::Stop {
                sandbox_id: sandbox_id.to_string(),
            })
            .await
        {
            // Stopping is idempotent.
            Ok(()) | Err(ComputeDriverError::NotFound) => {}
            Err(error) => return Err(error),
        }
        self.observe(sandbox_id).await;
        Ok(())
    }

    /// Returns whether there was a sandbox to delete.
    pub async fn delete_sandbox(&self, sandbox_id: &str) -> Result<bool, ComputeDriverError> {
        check_sandbox_id(sandbox_id)?;
        info!(%sandbox_id, "Deleting sandbox units");
        let existed = match self
            .helper
            .done(&Request::Delete {
                sandbox_id: sandbox_id.to_string(),
            })
            .await
        {
            Ok(()) => true,
            Err(ComputeDriverError::NotFound) => false,
            Err(error) => return Err(error),
        };
        self.observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(sandbox_id);
        if existed {
            let _ = self.events.send(deleted_event(sandbox_id.to_string()));
        }
        Ok(existed)
    }

    async fn record(&self, sandbox_id: &str) -> Result<Option<SandboxRecord>, ComputeDriverError> {
        match self
            .helper
            .call(&Request::Status {
                sandbox_id: sandbox_id.to_string(),
            })
            .await?
        {
            Response::Status(record) => Ok(Some(record)),
            Response::NotFound => Ok(None),
            other => Err(ComputeDriverError::Message(format!(
                "systemd helper: unexpected answer {other:?}"
            ))),
        }
    }

    pub async fn get_sandbox(
        &self,
        sandbox_id: &str,
    ) -> Result<Option<DriverSandbox>, ComputeDriverError> {
        check_sandbox_id(sandbox_id)?;
        Ok(self
            .record(sandbox_id)
            .await?
            .map(|record| driver_sandbox(&record)))
    }

    async fn records(&self) -> Result<Vec<SandboxRecord>, ComputeDriverError> {
        match self.helper.call(&Request::List).await? {
            Response::Listed { sandboxes } => Ok(sandboxes),
            other => Err(ComputeDriverError::Message(format!(
                "systemd helper: unexpected answer {other:?}"
            ))),
        }
    }

    pub async fn list_sandboxes(&self) -> Result<Vec<DriverSandbox>, ComputeDriverError> {
        Ok(self.records().await?.iter().map(driver_sandbox).collect())
    }

    /// The current sandboxes, then every change of their units' state.
    pub async fn watch_sandboxes(&self) -> Result<WatchStream, ComputeDriverError> {
        let mut events = self.events.subscribe();
        let current = self.records().await?;
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            for record in current {
                if sender
                    .send(Ok(sandbox_event(driver_sandbox(&record))))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if sender.send(Ok(event)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        warn!(missed, "The sandbox watch fell behind");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }

    /// Reports a record to the watch if it differs from the last one
    /// reported for its sandbox.
    fn report(&self, record: SandboxRecord) {
        let changed = self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(record.sandbox_id.clone(), record.clone())
            .as_ref()
            != Some(&record);
        if changed {
            debug!(sandbox_id = %record.sandbox_id, state = ?record.state, "Sandbox units changed");
            let _ = self.events.send(sandbox_event(driver_sandbox(&record)));
        }
    }

    async fn observe(&self, sandbox_id: &str) {
        match self.record(sandbox_id).await {
            Ok(Some(record)) => self.report(record),
            Ok(None) => {}
            Err(error) => warn!(%sandbox_id, %error, "Failed to read the sandbox units' state"),
        }
    }

    /// Reads every sandbox's state at the configured interval: units fail
    /// and stop without the driver being asked.
    async fn poll(self) {
        loop {
            tokio::time::sleep(self.config.poll_interval).await;
            let records = match self.records().await {
                Ok(records) => records,
                Err(error) => {
                    warn!(%error, "Failed to list the sandboxes' units");
                    continue;
                }
            };
            let gone: Vec<String> = {
                let observed = self
                    .observed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                observed
                    .keys()
                    .filter(|id| !records.iter().any(|record| &&record.sandbox_id == id))
                    .cloned()
                    .collect()
            };
            for sandbox_id in gone {
                self.observed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&sandbox_id);
                let _ = self.events.send(deleted_event(sandbox_id));
            }
            for record in records {
                self.report(record);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: UnitState, detail: &str) -> SandboxRecord {
        SandboxRecord {
            sandbox_id: "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205".into(),
            name: "demo".into(),
            workspace: "default".into(),
            state,
            detail: detail.into(),
        }
    }

    #[test]
    fn unit_states_map_to_the_ready_condition() {
        for (state, status, reason) in [
            (UnitState::Active, "True", CONDITION_RUNNING),
            (UnitState::Transitioning, "False", CONDITION_STARTING),
            (UnitState::Inactive, "False", CONDITION_STOPPED),
            (UnitState::Failed, "False", CONDITION_EXITED),
        ] {
            let condition = condition(&record(state, "supervisor failed, workload inactive"));
            assert_eq!(condition.r#type, "Ready");
            assert_eq!(condition.status, status, "{state:?}");
            assert_eq!(condition.reason, reason, "{state:?}");
        }
        assert_eq!(
            condition(&record(UnitState::Failed, "supervisor failed")).message,
            "supervisor failed"
        );
    }

    #[test]
    fn a_sandbox_is_reported_with_its_workload_unit() {
        let sandbox = driver_sandbox(&record(UnitState::Active, ""));
        assert_eq!(sandbox.id, "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205");
        assert_eq!(sandbox.name, "demo");
        assert_eq!(sandbox.workspace, "default");
        let status = sandbox.status.expect("status");
        assert_eq!(
            status.name,
            "openshell-sandbox-workload@d6ac9d5f-60aa-4fcb-b050-9a0a4b134205.service"
        );
        assert_eq!(status.instance_id, "d6ac9d5f-60a");
        assert_eq!(status.conditions.len(), 1);
    }

    #[test]
    fn only_the_hosts_workload_identity_can_be_requested() {
        let identity = crate::isolation::workload_identity(473, 474).expect("identity");
        for (user, group) in [
            ("", ""),
            ("sandbox", "sandbox"),
            ("473", "474"),
            (" 473 ", ""),
        ] {
            assert!(
                check_identity_request(user, group, &identity).is_ok(),
                "{user:?}:{group:?}"
            );
        }
        for (user, group) in [
            ("1000", ""),
            ("", "1000"),
            ("root", ""),
            ("0", "0"),
            ("474", "473"),
        ] {
            assert!(
                matches!(
                    check_identity_request(user, group, &identity),
                    Err(ComputeDriverError::InvalidArgument(_))
                ),
                "{user:?}:{group:?}"
            );
        }
    }

    #[test]
    fn only_canonical_uuids_are_sandbox_ids() {
        assert!(check_sandbox_id("d6ac9d5f-60aa-4fcb-b050-9a0a4b134205").is_ok());
        assert!(matches!(
            check_sandbox_id("../etc"),
            Err(ComputeDriverError::InvalidArgument(_))
        ));
    }
}
