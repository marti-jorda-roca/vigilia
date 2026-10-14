//! The Vigilia service.
//!
//! ```text
//! transports/local ──► ServiceHandle ──► Service loop ──► dispatch ──► session · journal · platform
//!                                          │    ▲
//!                                     bus ◄┘    └── one deadline timer
//! ```
//!
//! The service starts on demand (launchd socket activation, or spawned by a client) and exits
//! after `[energy] idle_exit_minutes` with no session and no subscribers.

pub mod bus;
pub mod dispatch;
pub mod install;
pub mod lock;
pub mod recover;
pub mod service;
pub mod transports;

pub use recover::{RecoverOutcome, recover_blocking};

#[cfg(unix)]
mod activation;

pub use service::{Deps, Exit, Service, ServiceHandle};

use std::io::IsTerminal;

use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use vigilia_core::{Config, Paths};

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the service is not supported on this OS yet")]
    Unsupported,
}

/// Runs the service in the foreground until it exits. Returns `Ok` when another instance is
/// already running.
pub fn run_blocking() -> Result<(), DaemonError> {
    let paths = Paths::for_user()?;
    paths.create_dirs()?;
    let (config, config_error) = match Config::load(&paths.config) {
        Ok(config) => (config, None),
        Err(e) => (Config::default(), Some(e)),
    };
    let _log_guard = init_logging(&paths, &config);
    if let Some(error) = config_error {
        tracing::error!(%error, "using default config");
    }
    run(paths, config)
}

#[cfg(unix)]
fn run(paths: Paths, config: Config) -> Result<(), DaemonError> {
    use std::sync::Arc;

    use vigilia_core::{Journal, SystemClock};

    let Some(_lock) = lock::try_acquire(&paths.lock)? else {
        tracing::info!("another instance is running; exiting");
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(async move {
        let (std_listener, socket_activated) = match activation::inherited_listener() {
            Some(listener) => (listener, true),
            None => (transports::local::bind(&paths.socket)?, false),
        };
        std_listener.set_nonblocking(true)?;
        let listener = tokio::net::UnixListener::from_std(std_listener)?;

        let (timer, fired) = vigilia_platform::native_timer();
        let service = Service::start(Deps {
            platform: vigilia_platform::native(),
            clock: Arc::new(SystemClock),
            timer,
            config,
            journal: Journal::open(&paths.journal)?,
            config_path: Some(paths.config.clone()),
            socket_activated,
        });
        let (handle, rx) = ServiceHandle::channel();
        let accept = tokio::spawn(transports::local::serve(listener, handle));
        tracing::info!(
            pid = std::process::id(),
            socket = %paths.socket.display(),
            socket_activated,
            "service started"
        );

        let exit = service.run(rx, fired, shutdown_signal()).await;
        accept.abort();
        if !socket_activated {
            // launchd owns its socket; ours we remove so the next client starts a fresh service.
            let _ = std::fs::remove_file(&paths.socket);
        }
        tracing::info!(?exit, "service stopped");
        Ok(())
    })
}

#[cfg(not(unix))]
fn run(_paths: Paths, _config: Config) -> Result<(), DaemonError> {
    Err(DaemonError::Unsupported)
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

/// Buffered, non-blocking file logging; also stderr when it is a terminal (`vigilia daemon` run
/// by hand). Nothing is fsynced.
fn init_logging(
    paths: &Paths,
    config: &Config,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::fmt::writer::BoxMakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    let filter: Targets = config
        .log
        .level
        .parse()
        .unwrap_or_else(|_| Targets::new().with_default(tracing::Level::INFO));
    // A small line buffer: the default (128k lines) pre-allocates ~4 MB in the channel,
    // which alone would break the daemon's 10 MB memory budget. Logging is lossy beyond
    // this; the journal, not the log, is the record that matters.
    let (file, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(512)
        .finish(tracing_appender::rolling::never(
            &paths.log_dir,
            "vigilia.log",
        ));
    let writer = if std::io::stderr().is_terminal() {
        BoxMakeWriter::new(file.and(std::io::stderr))
    } else {
        BoxMakeWriter::new(file)
    };
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false);
    let subscriber = tracing_subscriber::registry().with(fmt).with(filter);
    tracing::subscriber::set_global_default(subscriber).ok()?;
    Some(guard)
}
