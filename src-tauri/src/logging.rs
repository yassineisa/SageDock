//! Structured logging setup.
//!
//! Logs are written as JSON lines to a daily-rotating file under the app's log
//! directory, and mirrored to stdout in debug builds. Each log call should set an
//! explicit `target` matching one of the categories from the product spec
//! (installer, launcher, wsl, runtime, jupyter, kernel, package_manager, updates,
//! repair, config) so log files can be filtered by subsystem later without
//! re-instrumenting call sites.
//!
//! Verbose (`debug`) logging is opt-in via the `SAGEDOCK_LOG` env var (e.g.
//! `SAGEDOCK_LOG=debug`), normal users run at `info` by default, per the product
//! spec's requirement that debug logs not be on by default for end users.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Initializes the global tracing subscriber. Returns a guard that must be kept
/// alive for the lifetime of the app, dropping it stops the background writer
/// thread and can silently truncate the last log lines on shutdown.
pub fn init_logging(log_dir: &Path) -> WorkerGuard {
    std::fs::create_dir_all(log_dir).ok();

    let file_appender = rolling::daily(log_dir, "sagedock.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_env("SAGEDOCK_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking);

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    #[cfg(debug_assertions)]
    {
        registry
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
            .init();
    }
    #[cfg(not(debug_assertions))]
    {
        registry.init();
    }

    guard
}
