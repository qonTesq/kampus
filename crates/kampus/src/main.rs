//! `kampus serve --school <school.yaml> --policy <policy.yaml>`
//! `kampus check --school <school.yaml> --policy <policy.yaml>`

use std::path::PathBuf;
use std::process::ExitCode;

use kampus::App;
use kampus_core::config::{Env, PolicyConfig, SchoolConfig};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: kampus <serve|check> --school <school.yaml> --policy <policy.yaml>";

#[derive(Debug)]
struct Args {
    command: String,
    school: PathBuf,
    policy: PathBuf,
}

fn parse_args() -> Option<Args> {
    let mut args = std::env::args().skip(1);
    let command = args.next()?;
    let (mut school, mut policy) = (None, None);
    while let Some(flag) = args.next() {
        let value = args.next().map(PathBuf::from);
        match flag.as_str() {
            "--school" => school = value,
            "--policy" => policy = value,
            _ => return None,
        }
    }
    Some(Args {
        command,
        school: school?,
        policy: policy?,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let Some(args) = parse_args().filter(|a| matches!(a.command.as_str(), "serve" | "check"))
    else {
        tracing::error!("{USAGE}");
        return ExitCode::from(2);
    };

    let built = SchoolConfig::load(&args.school)
        .and_then(|school| PolicyConfig::load(&args.policy).map(|policy| (school, policy)));
    let app = match built
        .map_err(kampus::AppError::from)
        .and_then(|(school, policy)| App::build(school, policy, &Env::process()))
    {
        Ok(app) => app,
        Err(error) => {
            tracing::error!("startup refused: {error}");
            return ExitCode::FAILURE;
        }
    };

    if args.command == "check" {
        tracing::info!(
            school = %app.school().school.id,
            tools = app.tool_names().len(),
            "configuration valid"
        );
        return ExitCode::SUCCESS;
    }

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
