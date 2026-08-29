use std::{collections::BTreeSet, thread, time::Duration};

use autonomy_sim::{
    NetworkBackendSelection, Simulation, SimulationOptions,
    ditto::{CUAS_ENGAGEMENTS_COLLECTION, CUAS_EW_ASSIGNMENTS_COLLECTION, CUAS_TRACKS_COLLECTION},
    ditto_transport::{DittoTransportConfig, RealDittoOptions},
    model::EntityKind,
    scenario::ScenarioRegistry,
};

#[test]
fn real_ditto_transmits_the_defensive_funnel_between_defenders() {
    let Some(license) = std::env::var_os("DITTO_LICENSE") else {
        eprintln!("skipping real C-UAS integration test: DITTO_LICENSE is not set");
        return;
    };
    let scenarios = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios");
    let mut config = ScenarioRegistry::new(scenarios)
        .load("cuas-stadium")
        .unwrap();
    config.simulation.tick_hz = 10.0;
    let site = config
        .nodes
        .iter()
        .find(|node| node.kind == EntityKind::ProtectedSite)
        .unwrap()
        .position;
    for (index, threat) in config
        .nodes
        .iter_mut()
        .filter(|node| node.kind == EntityKind::ThreatUas)
        .enumerate()
    {
        threat.position = site.moved(index as f64 * 45.0, 1_200.0 + index as f64 * 35.0, 0.0);
        threat.position.alt_m = 150.0 + index as f64 * 5.0;
        threat.mission.speed_mps = 70.0;
    }
    let cuas = config.cuas.as_mut().unwrap();
    cuas.detection_range_m = 2_000.0;
    cuas.detection_delay_s = 0.2;
    cuas.ew_effect_delay_s = 0.4;
    cuas.interceptor_range_m = 1_500.0;
    cuas.intercept_time_s = 0.5;
    cuas.gun_range_m = 800.0;
    cuas.gun_effect_delay_s = 0.4;

    let storage = tempfile::tempdir().unwrap();
    let options = SimulationOptions {
        network: Some(NetworkBackendSelection::Analytic),
        ditto: DittoTransportConfig::Real(RealDittoOptions {
            database_id: "00000005-0000-0000-0000-000000000000".into(),
            license: license.to_string_lossy().into_owned(),
            storage_root: storage.path().into(),
            port_base: 30_000 + (std::process::id() % 1_000) as u16,
            listen_ip: "127.0.0.1".into(),
            collections: vec![
                CUAS_TRACKS_COLLECTION.into(),
                CUAS_EW_ASSIGNMENTS_COLLECTION.into(),
                CUAS_ENGAGEMENTS_COLLECTION.into(),
            ],
        }),
    };
    let mut simulation = Simulation::try_new_with_options(&config, &options).unwrap();
    let initial = simulation.snapshot().unwrap();
    assert_eq!(initial.payload.ditto_peers.len(), 8);
    assert!(initial.payload.ditto_peers.iter().all(|peer| {
        !initial
            .payload
            .entities
            .iter()
            .any(|entity| entity.id == peer.entity_id && entity.kind == EntityKind::ThreatUas)
    }));

    let mut observed_states = BTreeSet::from(["inbound".to_owned()]);
    let mut converged_collections = BTreeSet::new();
    for _ in 0..1_000 {
        let frame = simulation.tick().unwrap();
        observed_states.extend(
            frame
                .payload
                .entities
                .iter()
                .filter(|entity| entity.kind == EntityKind::ThreatUas)
                .map(|entity| entity.mission_state.clone()),
        );
        for document in frame
            .payload
            .ditto_documents
            .iter()
            .filter(|document| document.collection.starts_with("cuas."))
        {
            if document.converged && document.replicated_to.len() == 8 {
                converged_collections.insert(document.collection.clone());
            }
        }
        if [
            CUAS_TRACKS_COLLECTION,
            CUAS_EW_ASSIGNMENTS_COLLECTION,
            CUAS_ENGAGEMENTS_COLLECTION,
        ]
        .iter()
        .all(|collection| converged_collections.contains(*collection))
            && [
                "detected",
                "jammed",
                "leaking",
                "intercepted",
                "engaged_gun",
                "neutralized",
                "leaked",
            ]
            .iter()
            .all(|state| observed_states.contains(*state))
        {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(
        converged_collections,
        BTreeSet::from([
            CUAS_TRACKS_COLLECTION.to_owned(),
            CUAS_EW_ASSIGNMENTS_COLLECTION.to_owned(),
            CUAS_ENGAGEMENTS_COLLECTION.to_owned(),
        ]),
        "real Ditto must replicate every defensive collection to all defenders"
    );
    for state in [
        "inbound",
        "detected",
        "jammed",
        "leaking",
        "intercepted",
        "engaged_gun",
        "neutralized",
        "leaked",
    ] {
        assert!(
            observed_states.contains(state),
            "missing real-Ditto-gated state {state}: {observed_states:?}"
        );
    }
}
