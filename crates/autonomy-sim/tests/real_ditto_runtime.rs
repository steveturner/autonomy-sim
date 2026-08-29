use std::{env, net::TcpListener, thread, time::Duration};

use autonomy_sim::{
    Simulation, SimulationOptions,
    ditto_transport::{DittoTransportConfig, RealDittoOptions},
    scenario::ScenarioRegistry,
};

fn available_port_block(count: u16) -> u16 {
    for base in 20_000_u16..60_000 {
        let listeners: Vec<_> = (base..base.saturating_add(count))
            .map(|port| TcpListener::bind(("127.0.0.1", port)))
            .collect();
        if listeners.iter().all(Result::is_ok) {
            return base;
        }
    }
    panic!("no consecutive loopback port block was available")
}

#[test]
fn selected_real_transport_drives_simulation_observations() {
    let license = match env::var("DITTO_LICENSE") {
        Ok(license) if !license.is_empty() => license,
        _ => {
            eprintln!("skipping real Ditto runtime test: DITTO_LICENSE is not set");
            return;
        }
    };
    let scenario_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios");
    let registry = ScenarioRegistry::new(scenario_dir);
    let mut config = registry.load("isr-relay-demo").unwrap();
    config.cot.sink = "disabled".into();
    let storage = tempfile::tempdir().unwrap();
    let options = SimulationOptions {
        ditto: DittoTransportConfig::Real(RealDittoOptions {
            database_id: "00000005-0000-0000-0000-000000000000".into(),
            license,
            storage_root: storage.path().into(),
            port_base: available_port_block(config.nodes.len() as u16),
            listen_ip: "127.0.0.1".into(),
            collections: Vec::new(),
        }),
        ..SimulationOptions::default()
    };
    let mut simulation = Simulation::try_new_with_options(&config, &options).unwrap();

    let mut frame = simulation.snapshot().unwrap();
    for _ in 0..30 {
        frame = simulation.tick().unwrap();
        if frame
            .payload
            .ditto_documents
            .iter()
            .any(|document| document.replicated_to.len() > 1)
        {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(frame.payload.ditto_peers.len(), config.nodes.len());
    assert!(!frame.payload.ditto_documents.is_empty());
    assert!(
        frame
            .payload
            .ditto_documents
            .iter()
            .any(|document| document.replicated_to.len() > 1),
        "real Ditto documents never replicated between scenario peers"
    );
}
