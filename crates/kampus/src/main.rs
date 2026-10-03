//! `kampus serve --school <school.yaml> --policy <policy.yaml>`
//! `kampus check --school <school.yaml> --policy <policy.yaml>`
//! Optional: `--audit <file.jsonl>` (default `data/audit/<school_id>.jsonl`).
//! `kampus auth rotate-key --school <school.yaml>` (broker builds): new
//! active signing key, previous key kept; takes effect on restart.

use std::path::PathBuf;
use std::process::ExitCode;

use kampus::{App, AppOptions};
use kampus_core::config::{Env, PolicyConfig, SchoolConfig};
use tokio_util::sync::CancellationToken;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, fmt};

const USAGE: &str = "usage: kampus <serve|check> --school <school.yaml> --policy <policy.yaml> [--audit <file.jsonl>]";

#[derive(Debug)]
struct Args {
    command: String,
    school: PathBuf,
    policy: PathBuf,
    audit: Option<PathBuf>,
}

fn parse_args() -> Option<Args> {
    let mut args = std::env::args().skip(1);
    let command = args.next()?;
    let (mut school, mut policy, mut audit) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next().map(PathBuf::from);
        match flag.as_str() {
            "--school" => school = value,
            "--policy" => policy = value,
            "--audit" => audit = Some(value?),
            _ => return None,
        }
    }
    Some(Args {
        command,
        school: school?,
        policy: policy?,
        audit,
    })
}

/// `RUST_LOG` (default `info`) under a fixed cap it cannot lift: an event
/// must pass both filters. Only kampus targets may go below `info`; rmcp logs
/// whole requests and results at debug/trace and client-supplied text at
/// info, so it is capped at `warn`. Unlisted dependencies are capped at `info`.
fn init_logging() {
    let cap = Targets::new()
        .with_default(Level::INFO)
        .with_target("rmcp", Level::WARN)
        .with_target("kampus", Level::TRACE);
    let configured = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(cap)
        .with(configured)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
}

/// `kampus auth rotate-key --school <school.yaml>`.
fn rotate_key(mut args: impl Iterator<Item = String>) -> ExitCode {
    let school = match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--school"), Some(path), None) => PathBuf::from(path),
        _ => {
            tracing::error!("usage: kampus auth rotate-key --school <school.yaml>");
            return ExitCode::from(2);
        }
    };
    let config = match SchoolConfig::load(&school) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(broker) = &config.auth.broker else {
        tracing::error!("school.yaml has no auth.broker section");
        return ExitCode::FAILURE;
    };
    #[cfg(feature = "broker")]
    {
        match kampus_auth::rotate_key(&broker.key_file) {
            Ok(()) => {
                tracing::info!(
                    file = %broker.key_file.display(),
                    "signing key rotated; restart kampus to use it"
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                tracing::error!("{error}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(feature = "broker"))]
    {
        let _ = broker;
        tracing::error!("this binary was built without the broker feature");
        ExitCode::FAILURE
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();

    let mut raw = std::env::args().skip(1);
    if raw.next().as_deref() == Some("auth") {
        return match raw.next().as_deref() {
            Some("rotate-key") => rotate_key(raw),
            _ => {
                tracing::error!("usage: kampus auth rotate-key --school <school.yaml>");
                ExitCode::from(2)
            }
        };
    }

    let Some(args) = parse_args().filter(|a| matches!(a.command.as_str(), "serve" | "check"))
    else {
        tracing::error!("{USAGE}");
        return ExitCode::from(2);
    };

    let built = SchoolConfig::load(&args.school)
        .and_then(|school| PolicyConfig::load(&args.policy).map(|policy| (school, policy)));

    // `check` validates only: no audit log, listeners or sandbox services.
    if args.command == "check" {
        return match built
            .map_err(kampus::AppError::from)
            .and_then(|(school, policy)| App::check(school, &policy, &Env::process()))
        {
            Ok(report) => {
                tracing::info!(
                    school = %report.school_id,
                    tools = report.tools,
                    "configuration valid"
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                tracing::error!("configuration refused: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let app = match built
        .map_err(kampus::AppError::from)
        .and_then(|(school, policy)| {
            let options = AppOptions {
                secrets: None,
                audit_hasher: None,
                keys: None,
                audit_path: args.audit.clone(),
                policy_path: Some(args.policy.clone()),
            };
            App::build_with(school, policy, &Env::process(), options)
        }) {
        Ok(app) => app,
        Err(error) => {
            tracing::error!("startup refused: {error}");
            return ExitCode::FAILURE;
        }
    };

    let cancel = CancellationToken::new();
    let on_signal = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_signal.cancel();
        }
    });
    match app.serve(cancel).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
