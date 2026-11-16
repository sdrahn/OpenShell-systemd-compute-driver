// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! A compute driver that runs sandboxes as systemd units.
//!
//! Runs a sandbox as system units on the gateway's host. See the crate's
//! `README.md`.

pub mod config;
pub mod driver;
pub mod envfile;
pub mod grpc;
mod helper;
mod isolation;
pub mod protocol;

pub use config::SystemdComputeConfig;
pub use driver::SystemdComputeDriver;
pub use grpc::ComputeDriverService;

/// This driver's own version, not that of the `OpenShell` release it is
/// built against.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
