use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use autonomy_sim::{
    Simulation, SimulationOptions,
    ditto_transport::{DittoTransportConfig, RealDittoOptions},
    scenario::ScenarioRegistry,
    server,
};
use clap::{Parser, ValueEnum};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(about = "Defensive ISR autonomy and connectivity simulator")]
struct Args {
    #[arg(short, long, default_value = "isr-relay-demo")]
    scenario: String,

    #[arg(long, help = "Override the scenario API bind address")]
    bind: Option<SocketAddr>,

    #[arg(long, value_enum, default_value_t = DittoTransportArg::Behavioral)]
    ditto: DittoTransportArg,

    #[arg(long, default_value = "target/ditto-real")]
    ditto_storage_root: PathBuf,

    #[arg(long, default_value = "00000005-0000-0000-0000-000000000000")]
    ditto_database_id: String,

    #[arg(long, default_value_t = 46_000)]
    ditto_port_base: u16,

    #[arg(long, default_value = "127.0.0.1")]
    ditto_listen_ip: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DittoTransportArg {
    Behavioral,
    Real,
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
    let options = SimulationOptions {
        ditto: match args.ditto {
            DittoTransportArg::Behavioral => DittoTransportConfig::Behavioral,
            DittoTransportArg::Real => {
                if !cfg!(feature = "ditto-real") {
                    bail!(
                        "--ditto real is not compiled; rebuild autonomy-sim with --features ditto-real"
                    );
                }
                DittoTransportConfig::Real(RealDittoOptions {
                    database_id: args.ditto_database_id,
                    license: env::var("DITTO_LICENSE")
                        .context("--ditto real requires DITTO_LICENSE")?,
                    storage_root: args.ditto_storage_root,
                    port_base: args.ditto_port_base,
                    listen_ip: args.ditto_listen_ip,
                })
            }
        },
    };
    tracing::info!(scenario = %config.scenario.name, selector = %args.scenario, "loaded scenario");
    server::run(
        Simulation::try_new_with_options(&config, &options)?,
        bind,
        registry,
        options,
    )
    .await
}
