use std::net::SocketAddr;

use anyhow::{Context, Result};
use autonomy_sim::{Simulation, scenario::ScenarioRegistry, server};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(about = "Defensive ISR autonomy and connectivity simulator")]
struct Args {
    #[arg(short, long, default_value = "isr-relay-demo")]
    scenario: String,

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
    let registry = ScenarioRegistry::default();
    let config = registry.load(&args.scenario)?;
    let bind = match args.bind {
        Some(bind) => bind,
        None => config.api.bind.parse().context("parsing api.bind")?,
    };
    tracing::info!(scenario = %config.scenario.name, selector = %args.scenario, "loaded scenario");
    server::run(Simulation::try_new(&config)?, bind, registry.descriptors()?).await
}
