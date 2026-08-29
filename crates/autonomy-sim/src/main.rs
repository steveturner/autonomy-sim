use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use autonomy_sim::{ScenarioConfig, Simulation, server};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(about = "Defensive ISR autonomy and connectivity simulator")]
struct Args {
    #[arg(short, long, default_value = "scenarios/thin-slice.toml")]
    scenario: PathBuf,

    #[arg(long, help = "Override the scenario API bind address")]
    bind: Option<SocketAddr>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();
    let args = Args::parse();
    let config = ScenarioConfig::from_path(&args.scenario)?;
    let bind = match args.bind {
        Some(bind) => bind,
        None => config.api.bind.parse().context("parsing api.bind")?,
    };
    tracing::info!(scenario = %config.scenario.name, path = %args.scenario.display(), "loaded scenario");
    server::run(Simulation::try_new(&config)?, bind).await
}
