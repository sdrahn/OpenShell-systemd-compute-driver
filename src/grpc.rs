// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! The `ComputeDriver` gRPC service of the systemd compute driver.

#![allow(clippy::result_large_err)] // gRPC handlers return Result<_, tonic::Status>

use std::pin::Pin;

use futures::{Stream, StreamExt};
use openshell_core::proto::compute::v1::{
    AuthenticateSandboxRequest, AuthenticateSandboxResponse, CreateSandboxRequest,
    CreateSandboxResponse, DeleteSandboxRequest, DeleteSandboxResponse, DeleteWorkspaceRequest,
    DeleteWorkspaceResponse, EnsureWorkspaceRequest, EnsureWorkspaceResponse,
    GetCapabilitiesRequest, GetCapabilitiesResponse, GetSandboxRequest, GetSandboxResponse,
    ListSandboxesRequest, ListSandboxesResponse, StartSandboxRequest, StartSandboxResponse,
    StopSandboxRequest, StopSandboxResponse, SuspendSandboxRequest, SuspendSandboxResponse,
    ValidateSandboxCreateRequest, ValidateSandboxCreateResponse, WatchSandboxesEvent,
    WatchSandboxesRequest, compute_driver_server::ComputeDriver,
};
use tonic::{Request, Response, Status};

use crate::SystemdComputeDriver;
use crate::driver::DRIVER_NAME;

type ComputeDriverWatchStream =
    Pin<Box<dyn Stream<Item = Result<WatchSandboxesEvent, Status>> + Send + 'static>>;

/// Serves a [`SystemdComputeDriver`] to the gateway.
#[derive(Debug, Clone)]
pub struct ComputeDriverService {
    driver: SystemdComputeDriver,
}

impl ComputeDriverService {
    #[must_use]
    pub const fn new(driver: SystemdComputeDriver) -> Self {
        Self { driver }
    }
}

#[tonic::async_trait]
impl ComputeDriver for ComputeDriverService {
    async fn authenticate_sandbox(
        &self,
        _request: Request<AuthenticateSandboxRequest>,
    ) -> Result<Response<AuthenticateSandboxResponse>, Status> {
        Err(Status::unimplemented(
            "the systemd compute driver does not authenticate sandbox credentials",
        ))
    }

    async fn get_capabilities(
        &self,
        request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<GetCapabilitiesResponse>, Status> {
        let capabilities = self.driver.capabilities();
        openshell_core::extension_protocol::validate_gateway_metadata(
            openshell_core::extension_protocol::ExtensionFamily::Compute,
            DRIVER_NAME,
            capabilities.extension.as_ref(),
            request.into_inner().gateway,
        )
        .map_err(|error| Status::failed_precondition(error.to_string()))?;
        Ok(Response::new(capabilities))
    }

    async fn validate_sandbox_create(
        &self,
        request: Request<ValidateSandboxCreateRequest>,
    ) -> Result<Response<ValidateSandboxCreateResponse>, Status> {
        let sandbox = request
            .into_inner()
            .sandbox
            .ok_or_else(|| Status::invalid_argument("sandbox is required"))?;
        self.driver
            .validate_sandbox_create(&sandbox)
            .map_err(Status::from)?;
        Ok(Response::new(ValidateSandboxCreateResponse {}))
    }

    async fn get_sandbox(
        &self,
        request: Request<GetSandboxRequest>,
    ) -> Result<Response<GetSandboxResponse>, Status> {
        let request = request.into_inner();
        if request.sandbox_id.is_empty() {
            return Err(Status::invalid_argument("sandbox_id is required"));
        }
        let sandbox = self
            .driver
            .get_sandbox(&request.sandbox_id)
            .await
            .map_err(Status::from)?
            .ok_or_else(|| Status::not_found("sandbox not found"))?;
        Ok(Response::new(GetSandboxResponse {
            sandbox: Some(sandbox),
        }))
    }

    async fn list_sandboxes(
        &self,
        _request: Request<ListSandboxesRequest>,
    ) -> Result<Response<ListSandboxesResponse>, Status> {
        let sandboxes = self.driver.list_sandboxes().await.map_err(Status::from)?;
        Ok(Response::new(ListSandboxesResponse { sandboxes }))
    }

    async fn create_sandbox(
        &self,
        request: Request<CreateSandboxRequest>,
    ) -> Result<Response<CreateSandboxResponse>, Status> {
        let sandbox = request
            .into_inner()
            .sandbox
            .ok_or_else(|| Status::invalid_argument("sandbox is required"))?;
        self.driver
            .create_sandbox(&sandbox)
            .await
            .map_err(Status::from)?;
        Ok(Response::new(CreateSandboxResponse::default()))
    }

    async fn suspend_sandbox(
        &self,
        _request: Request<SuspendSandboxRequest>,
    ) -> Result<Response<SuspendSandboxResponse>, Status> {
        Err(Status::unimplemented(
            "the systemd compute driver does not support sandbox suspend",
        ))
    }

    async fn stop_sandbox(
        &self,
        request: Request<StopSandboxRequest>,
    ) -> Result<Response<StopSandboxResponse>, Status> {
        let request = request.into_inner();
        if request.sandbox_id.is_empty() {
            return Err(Status::invalid_argument("sandbox_id is required"));
        }
        self.driver
            .stop_sandbox(&request.sandbox_id)
            .await
            .map_err(Status::from)?;
        Ok(Response::new(StopSandboxResponse {}))
    }

    async fn start_sandbox(
        &self,
        request: Request<StartSandboxRequest>,
    ) -> Result<Response<StartSandboxResponse>, Status> {
        let request = request.into_inner();
        if request.sandbox_id.is_empty() {
            return Err(Status::invalid_argument("sandbox_id is required"));
        }
        if request.resume {
            return Err(Status::failed_precondition(
                "the systemd compute driver retains no suspended sandbox state",
            ));
        }
        self.driver
            .start_sandbox(
                &request.sandbox_id,
                &request.launch_authentication,
                request.sandbox.as_ref(),
            )
            .await
            .map_err(Status::from)?;
        Ok(Response::new(StartSandboxResponse::default()))
    }

    async fn delete_sandbox(
        &self,
        request: Request<DeleteSandboxRequest>,
    ) -> Result<Response<DeleteSandboxResponse>, Status> {
        let request = request.into_inner();
        if request.sandbox_id.is_empty() {
            return Err(Status::invalid_argument("sandbox_id is required"));
        }
        let deleted = self
            .driver
            .delete_sandbox(&request.sandbox_id)
            .await
            .map_err(Status::from)?;
        Ok(Response::new(DeleteSandboxResponse { deleted }))
    }

    type WatchSandboxesStream = ComputeDriverWatchStream;

    async fn watch_sandboxes(
        &self,
        _request: Request<WatchSandboxesRequest>,
    ) -> Result<Response<Self::WatchSandboxesStream>, Status> {
        let stream = self.driver.watch_sandboxes().await.map_err(Status::from)?;
        let stream: ComputeDriverWatchStream =
            Box::pin(stream.map(|item| item.map_err(|error| Status::internal(error.to_string()))));
        Ok(Response::new(stream))
    }

    async fn ensure_workspace(
        &self,
        _request: Request<EnsureWorkspaceRequest>,
    ) -> Result<Response<EnsureWorkspaceResponse>, Status> {
        Ok(Response::new(EnsureWorkspaceResponse {}))
    }

    async fn delete_workspace(
        &self,
        _request: Request<DeleteWorkspaceRequest>,
    ) -> Result<Response<DeleteWorkspaceResponse>, Status> {
        Ok(Response::new(DeleteWorkspaceResponse {}))
    }
}
