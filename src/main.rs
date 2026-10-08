use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use twitch_drops_miner::{
    logging,
    runtime::Runtime,
    web::{self, App},
};

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: IpAddr,
    #[arg(long, env = "PORT", default_value_t = 8080)]
    port: u16,
    #[arg(long, env = "DATA_DIR", default_value = "data")]
    data_dir: PathBuf,
    #[arg(long, env = "LOG_DIR", default_value = "logs")]
    log_dir: PathBuf,
    #[arg(long, env = "PUBLIC_BASE_URL", default_value = "")]
    public_base_url: String,
    #[arg(short,long,action=clap::ArgAction::Count)]
    verbose: u8,
    /// Write bounded, redacted upstream response and transport diagnostics to server logs.
    #[arg(long, env = "TDM_DIAGNOSTICS", default_value_t = false, value_parser = clap::builder::BoolishValueParser::new())]
    diagnostics: bool,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    Healthcheck,
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {_=terminate.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn run(args: Args) -> Result<()> {
    if matches!(args.command, Some(Action::Healthcheck)) {
        let host = if args.host.is_unspecified() {
            "127.0.0.1".parse().unwrap()
        } else {
            args.host
        };
        let url = format!("http://{}/healthz", SocketAddr::new(host, args.port));
        let response = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()?
            .get(url)
            .send()
            .await
            .context("health check failed")?;
        anyhow::ensure!(
            response.status() == reqwest::StatusCode::OK,
            "health check failed"
        );
        return Ok(());
    }
    let _logs = logging::initialize(&args.log_dir, args.verbose, args.diagnostics)?;
    if args.diagnostics {
        tracing::info!("Advanced upstream diagnostics enabled (redacted and bounded)");
    }
    let (app, commands) = App::open(args.data_dir, &args.public_base_url)?;
    let address = SocketAddr::new(args.host, args.port);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .context("could not bind dashboard address")?;
    tracing::info!(version=env!("CARGO_PKG_VERSION"),%address,"Starting Twitch Drops Miner");
    let shutdown = app.shutdown.clone();
    let router = web::router(app.clone());
    let mut server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
    });
    let mut runtime = Runtime::start(app.application.clone(), commands);
    let mut server_finished = false;
    let mut failure = None;
    tokio::select! {
        _=shutdown_signal()=>{},
        _=app.shutdown.cancelled()=>{},
        result=runtime.wait()=>{if result.is_err(){failure=Some("mining task failed");}},
        result=&mut server=>{server_finished=true;if !matches!(result,Ok(Ok(()))){failure=Some("dashboard server failed");}},
    }
    app.shutdown.cancel();
    if runtime.shutdown().await.is_err() {
        failure = Some("mining task failed");
    }
    app.sockets.close().await;
    if !server_finished {
        match tokio::time::timeout(Duration::from_secs(10), &mut server).await {
            Ok(Ok(Ok(()))) => {}
            Ok(_) => failure = Some("dashboard server failed"),
            Err(_) => {
                server.abort();
                let _ = server.await;
            }
        }
    }
    tracing::info!("Shutdown complete");
    if let Some(failure) = failure {
        anyhow::bail!(failure);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Twitch Drops Miner: {error}");
            ExitCode::FAILURE
        }
    }
}
