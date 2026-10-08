use anyhow::{Context, Result};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub fn log_filter(verbose: u8, diagnostics: bool) -> EnvFilter {
    // Transport TRACE output includes OAuth-bearing frames. Only this package
    // may increase verbosity; ambient RUST_LOG cannot enable dependency traces.
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let diagnostics = if diagnostics { "debug" } else { "off" };
    EnvFilter::new(format!(
        "warn,twitch_drops_miner={level},twitch_drops_miner_core={level},twitch_drops_miner_desktop={level},tdm_diagnostics={diagnostics}"
    ))
}

pub fn initialize(
    directory: &std::path::Path,
    verbose: u8,
    diagnostics: bool,
) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(directory).context("could not create log directory")?;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("TDM")
        .filename_suffix("log")
        .max_log_files(5)
        .build(directory)
        .context("could not open log directory")?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::registry()
        .with(log_filter(verbose, diagnostics))
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(writer),
        )
        .try_init()
        .context("could not initialize logging")?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn maximal_verbosity_cannot_log_transport_frames_or_request_credentials() {
        let output = Arc::new(Mutex::new(vec![]));
        let writer = Writer(output.clone());
        let subscriber = tracing_subscriber::registry()
            .with(log_filter(255, false))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || writer.clone()),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target:"tungstenite::protocol","LISTEN auth_token=secret");
            tracing::debug!(target:"reqwest::connect","proxy password=secret");
            tracing::trace!(target:"twitch_drops_miner","safe application diagnostic");
            tracing::debug!(target:"tdm_diagnostics","advanced response capture");
        });
        let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(text.contains("safe application diagnostic"));
        assert!(!text.contains("secret"));
        assert!(!text.contains("advanced response capture"));
    }

    #[test]
    fn diagnostics_requires_explicit_opt_in_and_keeps_dependency_traces_disabled() {
        let output = Arc::new(Mutex::new(vec![]));
        let writer = Writer(output.clone());
        let subscriber = tracing_subscriber::registry()
            .with(log_filter(0, true))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || writer.clone()),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target:"tdm_diagnostics", "advanced response capture");
            tracing::trace!(target:"tungstenite::protocol", "auth_token=secret");
            tracing::debug!(target:"reqwest::connect", "proxy password=secret");
        });
        let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(text.contains("advanced response capture"));
        assert!(!text.contains("secret"));
    }
}
