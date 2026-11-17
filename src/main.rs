// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::path::PathBuf;

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use openshell_core::proto::compute::v1::compute_driver_server::ComputeDriverServer;
use openshell_driver_systemd::config::DEFAULT_HELPER_SOCKET;
use openshell_driver_systemd::{
    ComputeDriverService, SystemdComputeConfig, SystemdComputeDriver, VERSION,
};
use tracing::info;

#[derive(Parser)]
#[command(name = "openshell-driver-systemd")]
#[command(version = VERSION)]
struct Args {
    #[arg(
        long,
        env = "OPENSHELL_DRIVER_ADMISSION_CONFIG_JSON",
        default_value = "{}"
    )]
    admission_config_json: openshell_core::resource_admission::DriverAdmissionConfig,

    /// The socket the gateway reaches the driver on. Only the user the
    /// driver runs as is served there; there is no other way in.
    #[arg(long, env = "OPENSHELL_COMPUTE_DRIVER_SOCKET")]
    bind_socket: PathBuf,

    #[arg(long, env = "OPENSHELL_LOG_LEVEL", default_value = "info")]
    log_level: String,

    #[arg(long, env = "OPENSHELL_GATEWAY_NAME")]
    gateway_name: Option<String>,

    /// The gateway endpoint supervisors dial.
    #[arg(long, env = "OPENSHELL_GRPC_ENDPOINT")]
    grpc_endpoint: Option<String>,

    #[arg(
        long,
        env = "OPENSHELL_GATEWAY_PORT",
        default_value_t = openshell_core::config::DEFAULT_SERVER_PORT
    )]
    gateway_port: u16,

    /// The CA supervisors verify the gateway with.
    #[arg(long, env = "OPENSHELL_SYSTEMD_TLS_CA")]
    gateway_tls_ca: Option<PathBuf>,

    /// The privileged helper's socket.
    #[arg(long, env = "OPENSHELL_SYSTEMD_HELPER_SOCKET", default_value = DEFAULT_HELPER_SOCKET)]
    helper_socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&args.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // The supervisor shares the host's network: the gateway is on its
    // loopback. TLS when the gateway's CA is configured.
    let grpc_endpoint = args.grpc_endpoint.unwrap_or_else(|| {
        let scheme = if args.gateway_tls_ca.is_some() {
            "https"
        } else {
            "http"
        };
        format!("{scheme}://127.0.0.1:{}", args.gateway_port)
    });
    let driver = SystemdComputeDriver::new(SystemdComputeConfig {
        allow_driver_config: args.admission_config_json.allow_driver_config,
        resource_admission: args.admission_config_json.resource_admission.clone(),
        helper_socket: args.helper_socket,
        grpc_endpoint,
        gateway_name: args.gateway_name,
        gateway_tls_ca: args.gateway_tls_ca,
        ..SystemdComputeConfig::default()
    })
    .await
    .into_diagnostic()?;

    let service = ComputeDriverServer::new(ComputeDriverService::new(driver));
    let socket_path = args.bind_socket;
    let listener = openshell_core::external_driver_socket::bind_private(&socket_path)
        .map_err(|err| miette::miette!("{err}"))?;
    let _cleanup = openshell_core::external_driver_socket::SocketCleanup::new(socket_path.clone());
    info!(socket = %socket_path.display(), "Starting systemd compute driver");
    tonic::transport::Server::builder()
        .add_service(service)
        .serve_with_incoming_shutdown(
            openshell_core::external_driver_socket::SameUidUnixIncoming::new(listener),
            shutdown_signal(),
        )
        .await
        .into_diagnostic()
}

async fn select_shutdown_signal(
    ctrl_c: impl Future<Output = ()>,
    terminate: impl Future<Output = ()>,
) {
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

async fn ctrl_c_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "Failed to install Ctrl-C signal handler");
        std::future::pending::<()>().await;
    }
}

async fn terminate_signal() {
    let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        tracing::warn!("Failed to install SIGTERM signal handler");
        std::future::pending::<()>().await;
        return;
    };
    let _ = signal.recv().await;
}

async fn shutdown_signal() {
    select_shutdown_signal(ctrl_c_signal(), terminate_signal()).await;
    info!("Received shutdown signal, draining in-flight requests");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_completes_when_termination_signal_arrives() {
        select_shutdown_signal(std::future::pending(), std::future::ready(())).await;
    }

    #[test]
    fn defaults_name_the_helper_socket_and_no_tls() {
        let args = Args::try_parse_from([
            "openshell-driver-systemd",
            "--bind-socket",
            "/run/user/1000/openshell/systemd-driver.sock",
        ])
        .expect("defaults parse");
        assert_eq!(args.helper_socket, PathBuf::from(DEFAULT_HELPER_SOCKET));
        assert!(args.gateway_tls_ca.is_none());
    }

    #[test]
    fn there_is_no_way_in_but_the_socket() {
        assert!(Args::try_parse_from(["openshell-driver-systemd"]).is_err());
        assert!(
            Args::try_parse_from([
                "openshell-driver-systemd",
                "--bind-socket",
                "/run/x.sock",
                "--bind-address",
                "127.0.0.1:50062",
            ])
            .is_err()
        );
    }
}
