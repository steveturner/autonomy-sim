use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use autonomy_sim::{
    NetworkBackendSelection, Simulation, SimulationOptions,
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

    #[arg(
        long,
        value_enum,
        help = "Override the scenario network backend (default: scenario configuration)"
    )]
    network_backend: Option<NetworkBackendArg>,

    #[arg(long, help = "SigForge REST base URL for a CLI backend override")]
    sigforge_url: Option<String>,

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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum NetworkBackendArg {
    Analytic,
    Sigforge,
}

fn network_selection(
    backend: Option<NetworkBackendArg>,
    sigforge_url: Option<String>,
    scenario_sigforge_url: &str,
) -> Result<Option<NetworkBackendSelection>> {
    Ok(match (backend, sigforge_url) {
        (None, None) => None,
        (None, Some(_)) => bail!("--sigforge-url requires --network-backend sigforge"),
        (Some(NetworkBackendArg::Analytic), None) => Some(NetworkBackendSelection::Analytic),
        (Some(NetworkBackendArg::Analytic), Some(_)) => {
            bail!("--sigforge-url cannot be used with --network-backend analytic")
        }
        (Some(NetworkBackendArg::Sigforge), base_url) => Some(NetworkBackendSelection::SigForge {
            base_url: base_url.unwrap_or_else(|| scenario_sigforge_url.to_owned()),
        }),
    })
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
    let network = network_selection(
        args.network_backend,
        args.sigforge_url,
        &config.simulation.sigforge_url,
    )?;
    let options = SimulationOptions {
        network,
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
                    collections: Vec::new(),
                })
            }
        },
    };
    tracing::info!(scenario = %config.scenario.name, selector = %args.scenario, "loaded scenario");
    server::run(
        Simulation::try_new_with_options(&config, &options)?,
        bind,
        registry,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_independent_ditto_and_network_selectors() {
        let args = Args::try_parse_from([
            "autonomy-sim",
            "--ditto",
            "real",
            "--network-backend",
            "sigforge",
            "--sigforge-url",
            "http://127.0.0.1:8080",
        ])
        .unwrap();

        assert!(matches!(args.ditto, DittoTransportArg::Real));
        assert!(matches!(
            args.network_backend,
            Some(NetworkBackendArg::Sigforge)
        ));
        assert_eq!(args.sigforge_url.as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[test]
    fn network_selector_preserves_scenario_default_and_validates_url() {
        assert!(
            network_selection(None, None, "http://scenario")
                .unwrap()
                .is_none()
        );

        let selection =
            network_selection(Some(NetworkBackendArg::Sigforge), None, "http://scenario").unwrap();
        assert!(matches!(
            selection,
            Some(NetworkBackendSelection::SigForge { base_url })
                if base_url == "http://scenario"
        ));

        assert!(network_selection(None, Some("http://unused".into()), "http://scenario").is_err());
    }
}
