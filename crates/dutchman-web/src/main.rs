//! `dutchman-web`: Dutchman as a web page, for a self-hosted Dutchman. See `docs/web.md`.

use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use dutchman::worker::Limits;
use dutchman_web::upload::ZipLimits;
use dutchman_web::Config;

/// Converts CAD designs uploaded from a browser.
///
/// Set DUTCHMAN_WEB_PASSWORD to ask every browser for a user name and password.
#[derive(Parser)]
#[command(name = "dutchman-web", version)]
struct Args {
    /// The address and port to listen on. 0.0.0.0 listens on every network.
    #[arg(long, env = "DUTCHMAN_WEB_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// The folder uploads and designs are kept in while their job is kept (default: a
    /// `dutchman-web` folder in the system's temporary folder).
    #[arg(long, env = "DUTCHMAN_WEB_DATA", value_name = "DIR")]
    data_dir: Option<PathBuf>,
    /// The most one upload may hold, in megabytes, all its files together.
    #[arg(
        long,
        env = "DUTCHMAN_WEB_MAX_UPLOAD",
        value_name = "MB",
        default_value_t = 200
    )]
    max_upload: u64,
    /// How many uploads may be received at once; more are refused.
    #[arg(long, env = "DUTCHMAN_WEB_UPLOADS", default_value_t = 4)]
    uploads: usize,
    /// Refuse new jobs above this many megabytes of saved data.
    #[arg(
        long,
        env = "DUTCHMAN_WEB_MAX_DATA",
        value_name = "MB",
        default_value_t = 10240
    )]
    max_data: u64,
    /// Refuse new jobs below this many megabytes of free space (Unix).
    #[arg(
        long,
        env = "DUTCHMAN_WEB_MIN_FREE",
        value_name = "MB",
        default_value_t = 1024
    )]
    min_free: u64,
    /// Additional browser origins allowed through a reverse proxy (comma-separated).
    #[arg(long, env = "DUTCHMAN_WEB_PUBLIC_ORIGIN", value_delimiter = ',')]
    public_origin: Vec<String>,
    /// The most all zips of one upload may unpack to, in megabytes.
    #[arg(
        long,
        env = "DUTCHMAN_WEB_MAX_UNZIPPED",
        value_name = "MB",
        default_value_t = 2048
    )]
    max_unzipped: u64,
    /// Minutes a job's designs are kept before they are deleted (at least 1).
    #[arg(
        long,
        env = "DUTCHMAN_WEB_KEEP",
        value_name = "MINUTES",
        default_value_t = 60
    )]
    keep: u64,
    /// How many uploads are converted at once; more wait their turn.
    #[arg(long, env = "DUTCHMAN_WEB_RUNS", value_name = "N", default_value_t = 2)]
    runs: usize,
    /// How many files of one upload are converted at once (default: the machine's cores, at
    /// most 8).
    #[arg(long, env = "DUTCHMAN_WEB_JOBS", value_name = "N")]
    jobs: Option<NonZeroUsize>,
    /// Seconds one file may take before it is stopped and reported as failed.
    #[arg(
        long,
        env = "DUTCHMAN_WEB_TIMEOUT",
        value_name = "SECONDS",
        default_value_t = 900
    )]
    timeout: u64,
    /// Megabytes of memory one file may hold before it is stopped and reported as failed
    /// (0: no limit).
    #[arg(
        long,
        env = "DUTCHMAN_WEB_MAX_MEMORY",
        value_name = "MB",
        default_value_t = 8192
    )]
    max_memory: u64,
    /// The user name asked for when DUTCHMAN_WEB_PASSWORD is set.
    #[arg(long, env = "DUTCHMAN_WEB_USER", default_value = "dutchman")]
    user: String,
    /// The path the server is reached under behind a reverse proxy, such as /dutchman (default:
    /// the root). Have the proxy pass the path on unchanged.
    #[arg(
        long,
        env = "DUTCHMAN_WEB_BASE_PATH",
        value_name = "PATH",
        default_value = ""
    )]
    base_path: String,
    /// A folder each job's designs are also saved in, such as a mounted network share. They
    /// keep their uploaded folders and names, are numbered rather than replace a file already
    /// there, and stay after the job is deleted.
    #[arg(long, env = "DUTCHMAN_WEB_OUTPUT", value_name = "DIR")]
    output_dir: Option<PathBuf>,
    /// Where the SolidCraft web app is: a same-origin path, such as /.
    #[arg(long, env = "DUTCHMAN_WEB_SOLIDCRAFT", value_name = "ADDRESS")]
    solidcraft: Option<String>,
}

fn main() -> ExitCode {
    if let Some(code) = dutchman::worker::run_if_worker() {
        return code;
    }
    let args = Args::parse();
    let worker = match std::env::current_exe() {
        Ok(program) => program,
        Err(err) => {
            eprintln!("dutchman-web: cannot find the running program: {err}");
            return ExitCode::FAILURE;
        }
    };
    // The password comes from the environment only, so it never shows in a process listing.
    let login = std::env::var("DUTCHMAN_WEB_PASSWORD")
        .ok()
        .filter(|password| !password.is_empty())
        .map(|password| (args.user.clone(), password));
    let base_path = match dutchman_web::base_path(&args.base_path) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("dutchman-web: {err}");
            return ExitCode::FAILURE;
        }
    };
    let solidcraft = match args
        .solidcraft
        .as_deref()
        .map(dutchman_web::solidcraft)
        .transpose()
    {
        Ok(address) => address,
        Err(err) => {
            eprintln!("dutchman-web: {err}");
            return ExitCode::FAILURE;
        }
    };
    let config = Config {
        data_dir: args
            .data_dir
            .unwrap_or_else(|| std::env::temp_dir().join("dutchman-web")),
        max_upload: args.max_upload.saturating_mul(1 << 20),
        uploads: args.uploads,
        max_data: args.max_data.saturating_mul(1 << 20),
        min_free: args.min_free.saturating_mul(1 << 20),
        public_origin: args.public_origin,
        zip: ZipLimits {
            entries: 5_000,
            bytes: args.max_unzipped.saturating_mul(1 << 20),
        },
        keep: Duration::from_secs(args.keep.max(1).saturating_mul(60)),
        runs: args.runs.max(1),
        jobs: args.jobs,
        limits: Limits {
            timeout: Duration::from_secs(args.timeout.max(1)),
            max_memory: (args.max_memory > 0).then(|| args.max_memory.saturating_mul(1 << 20)),
            design: Some(dutchman::worker::DEFAULT_DESIGN_LIMIT),
        },
        worker: Some(worker),
        login,
        base_path,
        output_dir: args.output_dir,
        solidcraft,
    };
    if let Err(err) = dutchman_web::prepare_data_dir(&config.data_dir) {
        eprintln!(
            "dutchman-web: cannot use {}: {err}",
            config.data_dir.display()
        );
        return ExitCode::FAILURE;
    }
    if let Some(folder) = &config.output_dir {
        if let Err(err) = dutchman_web::prepare_output_dir(folder) {
            eprintln!("dutchman-web: cannot use {}: {err}", folder.display());
            return ExitCode::FAILURE;
        }
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("dutchman-web: {err}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(args.listen, config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("dutchman-web: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(listen: SocketAddr, config: Config) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|err| {
        std::io::Error::new(err.kind(), format!("cannot listen on {listen}: {err}"))
    })?;
    if config.login.is_none() && !listen.ip().is_loopback() {
        eprintln!(
            "dutchman-web: anyone who can reach {listen} can convert files; set \
             DUTCHMAN_WEB_PASSWORD to ask for a password"
        );
    }
    eprintln!(
        "dutchman-web: listening on http://{listen}{}/",
        config.base_path
    );
    tokio::spawn(dutchman_web::remove_expired(
        config.data_dir.clone(),
        config.keep,
    ));
    axum::serve(listener, dutchman_web::router(config))
        .with_graceful_shutdown(stopped())
        .await
}

/// Resolves when the server is asked to stop: Ctrl-C, or SIGTERM from `docker stop`.
async fn stopped() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_solidcraft_address_parses() {
        let args = Args::try_parse_from(["dutchman-web", "--solidcraft", "/cad/"]).unwrap();
        assert_eq!(args.solidcraft.as_deref(), Some("/cad/"));
    }

    #[test]
    fn upload_and_storage_options_parse() {
        let args = Args::try_parse_from([
            "dutchman-web",
            "--uploads",
            "1",
            "--max-data",
            "2",
            "--min-free",
            "3",
            "--public-origin",
            "https://one.example,https://two.example",
            "--public-origin",
            "https://three.example",
        ])
        .unwrap();
        assert_eq!(args.uploads, 1);
        assert_eq!(args.base_path, "");
        assert_eq!(args.output_dir, None);
        assert_eq!(args.solidcraft, None);
        assert_eq!(args.max_data, 2);
        assert_eq!(args.min_free, 3);
        assert_eq!(
            args.public_origin,
            [
                "https://one.example",
                "https://two.example",
                "https://three.example"
            ]
        );
    }
}
