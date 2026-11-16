// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! The launch files of one sandbox generation.
//!
//! The same bootstrap file, runtime descriptor and auth bundle the
//! container drivers provision, for a workload that is a systemd unit on
//! the host: its identity is a host user, and its outer fence is a network
//! namespace with loopback only.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use openshell_core::ComputeDriverError;
use openshell_core::proto::compute::v1::DriverSandbox;
use openshell_isolation_interface::contract::{ProvisioningEvidence, ResolvedWorkloadIdentity};
use openshell_sandbox_backend::boundary_protocol::{
    BoundaryConfig, BoundaryListener, GatewayVerificationKey, SandboxRuntimeDescriptor,
    SandboxTlsClientConfig, SandboxTlsServerConfig, SandboxTransport,
    generate_sandbox_tls_material,
};
use serde::Serialize;

use crate::config::SystemdComputeConfig;
use crate::protocol::Provision;

/// Paths as the two units see them. The unit templates bind each
/// sandbox's directory to these.
pub const SERVER_CERTIFICATE_PATH: &str = "/.openshell/channel/sandbox/server.crt";
pub const SERVER_KEY_PATH: &str = "/.openshell/channel/sandbox/server.key";
pub const CONTROL_SOCKET_PATH: &str = "/.openshell/channel/control/control.sock";
pub const SANDBOX_TOKEN_PATH: &str = "/etc/openshell/auth/sandbox.jwt";
pub const GATEWAY_CA_PATH: &str = "/etc/openshell/tls/client/ca.crt";

/// The supervisor's runtime directory, which its unit creates for it and
/// the workload's unit hides.
pub fn supervisor_runtime_dir(sandbox_id: &str) -> String {
    format!("/run/openshell-sandbox/{sandbox_id}")
}

/// The unit of a sandbox's workload; a resource claim of its launch.
pub fn workload_unit(sandbox_id: &str) -> String {
    format!("openshell-sandbox-workload@{sandbox_id}.service")
}

/// The unit that holds the network namespace of a sandbox's workload.
pub fn network_namespace_unit(sandbox_id: &str) -> String {
    format!("openshell-sandbox-netns@{sandbox_id}.service")
}

fn invalid(error: impl std::fmt::Display) -> ComputeDriverError {
    ComputeDriverError::Precondition(error.to_string())
}

/// The fence the unit templates give every workload, as the evidence the
/// launch is bound to.
#[derive(Serialize)]
struct SystemdOuterFenceEvidence<'a> {
    unit: &'a str,
    network_namespace_unit: &'a str,
    interfaces: &'static [&'static str],
}

/// The identity every workload of this host runs as: the helper's
/// configured user, which is never root.
pub fn workload_identity(
    uid: u32,
    gid: u32,
) -> Result<ResolvedWorkloadIdentity, ComputeDriverError> {
    // There is no image to pin. The digest names what the identity was
    // resolved from: the helper's user and group.
    let digest =
        openshell_crypto::sha256(format!("openshell-driver-systemd:{uid}:{gid}").as_bytes())
            .map_err(|err| {
                ComputeDriverError::Message(format!("hash the workload identity: {err}"))
            })?
            .iter()
            .fold(String::with_capacity(64), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    ResolvedWorkloadIdentity::new(uid, gid, Vec::new(), "driver".into(), digest).map_err(invalid)
}

/// The supervisor's environment, as the container drivers set it.
fn supervisor_environment(
    sandbox: &DriverSandbox,
    config: &SystemdComputeConfig,
    identity: &ResolvedWorkloadIdentity,
    has_gateway_ca: bool,
) -> Result<BTreeMap<String, String>, ComputeDriverError> {
    use openshell_core::sandbox_env as names;

    let spec = sandbox.spec.as_ref();
    let mut environment = BTreeMap::new();
    let mut user_environment = BTreeMap::new();
    if let Some(template) = spec.and_then(|spec| spec.template.as_ref()) {
        for (name, value) in &template.environment {
            user_environment.insert(name.clone(), value.clone());
        }
    }
    if let Some(spec) = spec {
        if !spec.log_level.is_empty() {
            environment.insert(names::LOG_LEVEL.to_string(), spec.log_level.clone());
        }
        for (name, value) in &spec.environment {
            user_environment.insert(name.clone(), value.clone());
        }
    }
    if !user_environment.is_empty() {
        environment.insert(
            names::USER_ENVIRONMENT.to_string(),
            serde_json::to_string(&user_environment).map_err(invalid)?,
        );
    }
    environment.insert(names::SANDBOX.to_string(), sandbox.name.clone());
    environment.insert(names::SANDBOX_ID.to_string(), sandbox.id.clone());
    environment.insert(names::ENDPOINT.to_string(), config.grpc_endpoint.clone());
    let runtime_dir = supervisor_runtime_dir(&sandbox.id);
    environment.insert(
        names::SSH_SOCKET_PATH.to_string(),
        format!("{runtime_dir}/ssh.sock"),
    );
    environment.insert(
        names::CONTAINER_IMAGE.to_string(),
        crate::driver::HOST_IMAGE.to_string(),
    );
    environment.insert(
        names::CONTAINER_IMAGE_UID.to_string(),
        identity.resource_digest.clone(),
    );
    // A command may hold any character; the unit's environment file is
    // read by lines.
    environment.insert(
        names::MAIN_PROCESS_SPEC.to_string(),
        names::MainProcessConfig::encode_driver_spec_base64url(spec).map_err(invalid)?,
    );
    environment.insert(
        names::TELEMETRY_ENABLED.to_string(),
        openshell_core::telemetry::enabled_env_value().to_string(),
    );
    if let Some(name) = &config.gateway_name {
        environment.insert(names::GATEWAY_NAME.to_string(), name.clone());
    }
    environment.insert(
        names::NETWORK_RUNTIME_CAPABILITIES.to_string(),
        names::POLICY_DNS_TRANSPARENT_TCP_CAPABILITY.to_string(),
    );
    if has_gateway_ca {
        environment.insert(names::TLS_CA.to_string(), GATEWAY_CA_PATH.to_string());
    }
    environment.insert(names::OCI_IMAGE_USER.to_string(), String::new());
    environment.insert(names::SANDBOX_UID.to_string(), identity.uid.to_string());
    environment.insert(names::SANDBOX_GID.to_string(), identity.gid.to_string());
    environment.insert(
        names::SANDBOX_TOKEN_FILE.to_string(),
        SANDBOX_TOKEN_PATH.to_string(),
    );
    environment.insert(
        names::ADMITTED_ISOLATION_BACKEND.to_string(),
        openshell_sandbox_backend::BACKEND_NAME.to_string(),
    );
    environment.insert(
        names::PROXY_TLS_DIR.to_string(),
        format!("{runtime_dir}/proxy-tls"),
    );
    Ok(environment)
}

/// The workload's environment before the sandbox's own.
fn child_environment() -> HashMap<String, String> {
    HashMap::from([(
        "PATH".to_string(),
        "/usr/local/bin:/usr/bin:/bin".to_string(),
    )])
}

/// Builds the launch files of one sandbox generation.
pub fn provision(
    sandbox: &DriverSandbox,
    config: &SystemdComputeConfig,
    identity: &ResolvedWorkloadIdentity,
    launch_authentication: &openshell_core::jwt::SandboxLaunchAuthentication,
    gateway_ca: String,
) -> Result<Provision, ComputeDriverError> {
    launch_authentication.validate().map_err(invalid)?;
    let spec = sandbox
        .spec
        .as_ref()
        .ok_or_else(|| invalid("the sandbox has no spec"))?;
    let session_id = launch_authentication.supervisor.session_id;
    let tls = generate_sandbox_tls_material(session_id).map_err(invalid)?;
    let generation = launch_authentication
        .supervisor
        .runtime_generation
        .to_string();
    let unit = workload_unit(&sandbox.id);
    let namespace = network_namespace_unit(&sandbox.id);
    let resource_claims = BTreeMap::from([
        ("systemd.unit".to_string(), unit.clone()),
        (
            "systemd.identity".to_string(),
            identity.resource_digest.clone(),
        ),
    ]);
    let evidence = serde_json::to_vec(&SystemdOuterFenceEvidence {
        unit: &unit,
        network_namespace_unit: &namespace,
        interfaces: &["lo"],
    })
    .map_err(invalid)?;
    let provisioning_evidence =
        ProvisioningEvidence::from_enforcement_evidence(generation.clone(), &evidence)
            .map_err(invalid)?;
    provisioning_evidence
        .validate(&generation)
        .map_err(invalid)?;
    let verification_keys = launch_authentication
        .verification_keys
        .iter()
        .map(|key| {
            String::from_utf8(key.public_key_pem.clone())
                .map(|public_key_pem| GatewayVerificationKey {
                    key_id: key.key_id.clone(),
                    public_key_pem,
                })
                .map_err(invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let boundary = BoundaryConfig {
        boundary_id: sandbox.id.clone(),
        generation: generation.clone(),
        session_id,
        session_rotation: launch_authentication.supervisor.session_rotation,
        auth_epoch: launch_authentication.supervisor.auth_epoch,
        gateway_id: launch_authentication.gateway_id.clone(),
        verification_keys,
        listener: BoundaryListener::RuntimeInitiatedUnix {
            socket_path: CONTROL_SOCKET_PATH.into(),
            tls: SandboxTlsServerConfig {
                certificate_chain_path: PathBuf::from(SERVER_CERTIFICATE_PATH),
                private_key_path: PathBuf::from(SERVER_KEY_PATH),
            },
        },
        resource_claims: resource_claims.clone(),
        resource_claim_files: BTreeMap::new(),
        workload_identity: identity.clone(),
        provisioning_evidence: provisioning_evidence.clone(),
        child_env: child_environment(),
    };
    let runtime_descriptor = SandboxRuntimeDescriptor {
        boundary_id: sandbox.id.clone(),
        generation,
        session_id,
        transport: SandboxTransport::RuntimeInitiatedUnix {
            socket_path: CONTROL_SOCKET_PATH.into(),
        },
        tls: SandboxTlsClientConfig {
            server_name: tls.server_name,
            trust_anchor_pem: tls.trust_anchor_pem,
        },
        // The supervisor shares the host's network: the gateway is on its
        // loopback.
        host_gateway_ip: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        resource_claims,
        workload_identity: identity.clone(),
        provisioning_evidence,
    };
    let environment = supervisor_environment(sandbox, config, identity, !gateway_ca.is_empty())?;
    Ok(Provision {
        sandbox_id: sandbox.id.clone(),
        name: sandbox.name.clone(),
        workspace: sandbox.workspace.clone(),
        bootstrap: serde_json::to_string(&boundary).map_err(invalid)?,
        server_certificate: tls.certificate_chain_pem,
        server_key: tls.private_key_pem,
        runtime_descriptor: serde_json::to_string(&runtime_descriptor).map_err(invalid)?,
        auth_bundle: serde_json::to_string(&launch_authentication.supervisor).map_err(invalid)?,
        sandbox_token: spec.sandbox_token.clone(),
        gateway_ca,
        environment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workload_identity_is_never_root() {
        assert!(workload_identity(0, 1000).is_err());
        assert!(workload_identity(1000, 0).is_err());
        let identity = workload_identity(1000, 1000).expect("a non-root identity");
        assert_eq!((identity.uid, identity.gid), (1000, 1000));
        assert!(identity.supplementary_gids.is_empty());
    }

    #[test]
    fn the_identity_digest_follows_the_user_and_group() {
        let digest = |uid, gid| {
            workload_identity(uid, gid)
                .expect("identity")
                .resource_digest
        };
        assert_eq!(digest(1000, 1000), digest(1000, 1000));
        assert_ne!(digest(1000, 1000), digest(1001, 1000));
        assert_ne!(digest(1000, 1000), digest(1000, 1001));
        assert_eq!(digest(1000, 1000).len(), 64);
    }

    #[test]
    fn units_and_namespaces_are_named_by_the_sandbox() {
        let id = "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205";
        assert_eq!(
            workload_unit(id),
            "openshell-sandbox-workload@d6ac9d5f-60aa-4fcb-b050-9a0a4b134205.service"
        );
        assert_eq!(
            network_namespace_unit(id),
            "openshell-sandbox-netns@d6ac9d5f-60aa-4fcb-b050-9a0a4b134205.service"
        );
    }

    #[test]
    fn the_supervisor_environment_names_the_host_user_and_the_token_file() {
        let sandbox = DriverSandbox {
            id: "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205".into(),
            name: "demo".into(),
            ..Default::default()
        };
        let config = SystemdComputeConfig {
            grpc_endpoint: "https://127.0.0.1:17670".into(),
            gateway_name: Some("openshell".into()),
            ..SystemdComputeConfig::default()
        };
        let identity = workload_identity(473, 474).expect("identity");
        let environment =
            supervisor_environment(&sandbox, &config, &identity, true).expect("environment");
        let get = |name: &str| environment.get(name).map(String::as_str);
        assert_eq!(get(openshell_core::sandbox_env::SANDBOX), Some("demo"));
        assert_eq!(get(openshell_core::sandbox_env::SANDBOX_UID), Some("473"));
        assert_eq!(get(openshell_core::sandbox_env::SANDBOX_GID), Some("474"));
        assert_eq!(
            get(openshell_core::sandbox_env::SANDBOX_TOKEN_FILE),
            Some(SANDBOX_TOKEN_PATH)
        );
        assert_eq!(
            get(openshell_core::sandbox_env::TLS_CA),
            Some(GATEWAY_CA_PATH)
        );
        assert_eq!(
            get(openshell_core::sandbox_env::SSH_SOCKET_PATH),
            Some("/run/openshell-sandbox/d6ac9d5f-60aa-4fcb-b050-9a0a4b134205/ssh.sock")
        );
        assert_eq!(
            get(openshell_core::sandbox_env::PROXY_TLS_DIR),
            Some("/run/openshell-sandbox/d6ac9d5f-60aa-4fcb-b050-9a0a4b134205/proxy-tls")
        );
        assert!(
            get(openshell_core::sandbox_env::MAIN_PROCESS_SPEC)
                .is_some_and(|value| value.starts_with("base64url:"))
        );
        // Every value fits a line of the unit's environment file.
        crate::envfile::render(&environment).expect("an environment file");

        let without_ca =
            supervisor_environment(&sandbox, &config, &identity, false).expect("environment");
        assert!(!without_ca.contains_key(openshell_core::sandbox_env::TLS_CA));
    }
}
