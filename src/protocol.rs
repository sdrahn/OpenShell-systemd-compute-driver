// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! The protocol between the driver and its privileged helper.
//!
//! One request and one response per connection on a Unix socket, each a
//! JSON document. The driver writes the request and closes its half of the
//! connection; the helper answers and closes. The helper serves only the
//! user it was started for, checked from the socket's peer credentials.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Largest request or response either side reads.
pub const MAX_MESSAGE_BYTES: u64 = 4 * 1024 * 1024;

/// What the driver asks of the helper.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// The helper's version and the identity its workloads run as.
    Describe,
    /// Write one sandbox's launch files. Its units are stopped first.
    Provision(Box<Provision>),
    /// Start the sandbox's units.
    Start { sandbox_id: String },
    /// Stop the sandbox's units and keep its files.
    Stop { sandbox_id: String },
    /// Stop the sandbox's units and remove its files, the workspace too.
    Delete { sandbox_id: String },
    /// The state of one sandbox's units.
    Status { sandbox_id: String },
    /// Every sandbox the helper holds files for.
    List,
}

/// The launch files of one sandbox generation, as their contents.
///
/// The helper writes each to its fixed place and gives it its owner and
/// mode. Nothing here names a path or a unit.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct Provision {
    pub sandbox_id: String,
    pub name: String,
    pub workspace: String,
    /// `openshell-sandbox`'s bootstrap file.
    pub bootstrap: String,
    /// The sandbox's TLS identity for the Sandbox Protocol.
    pub server_certificate: String,
    pub server_key: String,
    /// The supervisor's runtime descriptor.
    pub runtime_descriptor: String,
    /// The supervisor's gateway-issued credentials for this launch.
    pub auth_bundle: String,
    /// The supervisor's gateway token. Empty for a launch that keeps the
    /// token of the launch before it: the gateway sends a sandbox's token
    /// when it creates the sandbox, not each time it starts it.
    pub sandbox_token: String,
    /// The CA the supervisor verifies the gateway with; empty for none.
    pub gateway_ca: String,
    /// The supervisor's environment.
    pub environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for Provision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Provision")
            .field("sandbox_id", &self.sandbox_id)
            .field("name", &self.name)
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

/// The state of a sandbox's two units together.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitState {
    /// Both units are active.
    Active,
    /// Neither unit is active and none failed.
    Inactive,
    /// A unit is starting or stopping.
    Transitioning,
    /// A unit failed, or one runs without the other.
    Failed,
}

/// One sandbox as the helper knows it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SandboxRecord {
    pub sandbox_id: String,
    pub name: String,
    pub workspace: String,
    pub state: UnitState,
    /// What systemd reports for the supervisor's and the workload's unit.
    pub detail: String,
}

/// The helper's answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Done,
    Described {
        version: String,
        /// The host user and group every workload and supervisor runs as.
        uid: u32,
        gid: u32,
    },
    Status(SandboxRecord),
    Listed {
        sandboxes: Vec<SandboxRecord>,
    },
    /// The sandbox is not known to the helper.
    NotFound,
    Error {
        message: String,
    },
}

/// A sandbox ID as the helper accepts one: a UUID in its canonical form.
///
/// It becomes a directory name and a unit instance, so nothing else is let
/// through.
pub fn valid_sandbox_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.hyphenated().to_string() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_ids_are_canonical_uuids_only() {
        assert!(valid_sandbox_id("d6ac9d5f-60aa-4fcb-b050-9a0a4b134205"));
        for id in [
            "",
            "D6AC9D5F-60AA-4FCB-B050-9A0A4B134205",
            "d6ac9d5f60aa4fcbb0509a0a4b134205",
            "{d6ac9d5f-60aa-4fcb-b050-9a0a4b134205}",
            "../d6ac9d5f-60aa-4fcb-b050-9a0a4b134205",
            "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205/..",
            "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205\n",
            "sandbox",
        ] {
            assert!(!valid_sandbox_id(id), "{id:?} must be refused");
        }
    }

    #[test]
    fn requests_round_trip_as_json() {
        for request in [
            Request::Describe,
            Request::List,
            Request::Start {
                sandbox_id: "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205".into(),
            },
        ] {
            let encoded = serde_json::to_string(&request).expect("encode");
            let decoded: Request = serde_json::from_str(&encoded).expect("decode");
            assert_eq!(decoded, request);
        }
    }

    #[test]
    fn provision_debug_shows_no_credentials() {
        let provision = Provision {
            sandbox_id: "d6ac9d5f-60aa-4fcb-b050-9a0a4b134205".into(),
            name: "demo".into(),
            workspace: "default".into(),
            bootstrap: "bootstrap-secret".into(),
            server_certificate: String::new(),
            server_key: "key-secret".into(),
            runtime_descriptor: String::new(),
            auth_bundle: "auth-secret".into(),
            sandbox_token: "token-secret".into(),
            gateway_ca: String::new(),
            environment: BTreeMap::new(),
        };
        let shown = format!("{provision:?}");
        assert!(shown.contains("demo"));
        assert!(!shown.contains("secret"), "{shown}");
    }
}
