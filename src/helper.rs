// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! The driver's side of the helper protocol.

use std::path::{Path, PathBuf};

use openshell_core::ComputeDriverError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::protocol::{MAX_MESSAGE_BYTES, Request, Response};

/// Reaches the privileged helper on its Unix socket.
#[derive(Clone, Debug)]
pub struct HelperClient {
    socket: PathBuf,
}

fn unavailable(socket: &Path, error: impl std::fmt::Display) -> ComputeDriverError {
    ComputeDriverError::Message(format!("systemd helper at {}: {error}", socket.display()))
}

impl HelperClient {
    #[must_use]
    pub const fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    /// Sends one request and reads its response.
    pub async fn call(&self, request: &Request) -> Result<Response, ComputeDriverError> {
        let encoded =
            serde_json::to_vec(request).map_err(|error| unavailable(&self.socket, error))?;
        let mut stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|error| unavailable(&self.socket, error))?;
        stream
            .write_all(&encoded)
            .await
            .map_err(|error| unavailable(&self.socket, error))?;
        stream
            .shutdown()
            .await
            .map_err(|error| unavailable(&self.socket, error))?;
        let mut answer = Vec::new();
        stream
            .take(MAX_MESSAGE_BYTES)
            .read_to_end(&mut answer)
            .await
            .map_err(|error| unavailable(&self.socket, error))?;
        match serde_json::from_slice(&answer)
            .map_err(|error| unavailable(&self.socket, format!("unreadable answer: {error}")))?
        {
            Response::Error { message } => Err(ComputeDriverError::Precondition(message)),
            response => Ok(response),
        }
    }

    /// A request whose only good answer is `Done`.
    pub async fn done(&self, request: &Request) -> Result<(), ComputeDriverError> {
        match self.call(request).await? {
            Response::Done => Ok(()),
            Response::NotFound => Err(ComputeDriverError::NotFound),
            other => Err(unavailable(
                &self.socket,
                format!("unexpected answer {other:?}"),
            )),
        }
    }
}
