use std::collections::BTreeSet;

use autonomy_sim::{
    ScenarioConfig, Simulation,
    ditto::{BASE_QUEUE_COLLECTION, DROP_ASSIGNMENTS_COLLECTION, FIRE_CELLS_COLLECTION},
    model::{Affiliation, EntityKind},
    scenario::ScenarioRegistry,
};

fn load_scenario() -> ScenarioConfig {
    let scenarios = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios");
    ScenarioRegistry::new(scenarios)
        .load("wildfire-paradise")
        .unwrap()
}

#[test]
fn registry_exposes_chooseable_scenarios() {
    let scenarios = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios");
    let descriptors = ScenarioRegistry::new(scenarios).descriptors().unwrap();
    assert_eq!(
        descriptors
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect::<Vec<_>>(),
        vec!["isr-relay-demo", "thin-slice", "wildfire-paradise"]
    );
}

#[test]
fn wildfire_stream_contract_and_mission_cycle_run_end_to_end() {
    let mut config = load_scenario();
    let base_position = config
        .nodes
        .iter()
        .find(|node| node.id == config.wildfire.as_ref().unwrap().base_id)
        .unwrap()
        .position;
    let wildfire = config.wildfire.as_mut().unwrap();
    for (index, cell) in wildfire.fire_cells.iter_mut().enumerate() {
        cell.position = base_position.moved(315.0 + index as f64, 1_500.0, 0.0);
    }
    wildfire.reload_slots = 1;
    wildfire.reload_time_s = 3.0;
    wildfire.drop_duration_s = 1.0;
    wildfire.on_station_s = 0.4;
    wildfire.egress_time_s = 1.0;
    wildfire.launch_interval_s = 0.0;
    wildfire.arrival_radius_m = 150.0;
    wildfire.base_arrival_radius_m = 120.0;
    wildfire.approach_distance_m = 300.0;
    wildfire.spread_per_s = 0.0;
    let mut simulation = Simulation::try_new(&config).unwrap();
    let initial = simulation.snapshot().unwrap();
    assert_eq!(initial.payload.fire_cells.len(), 9);
    assert_eq!(
        initial.payload.base.as_ref().unwrap().id,
        "grass-valley-aab"
    );
    let tankers: Vec<_> = initial
        .payload
        .entities
        .iter()
        .filter(|entity| entity.kind == EntityKind::AirTanker)
        .collect();
    assert_eq!(tankers.len(), 12);
    assert!(tankers.iter().all(|entity| {
        entity.affiliation == Affiliation::Friendly
            && entity.sidc.len() == 15
            && entity.mission_role == "tanker"
            && entity.mission_state == "holding"
            && entity.retardant_pct == Some(100.0)
    }));

    let initial_intensity: f64 = initial
        .payload
        .fire_cells
        .iter()
        .map(|cell| cell.intensity)
        .sum();
    let mut observed_states = BTreeSet::new();
    let mut observed_collections = BTreeSet::new();
    let mut observed_queue = false;
    let mut final_intensity = initial_intensity;

    for _ in 0..1_500 {
        let frame = simulation.tick().unwrap();
        observed_states.extend(
            frame
                .payload
                .entities
                .iter()
                .filter(|entity| entity.kind == EntityKind::AirTanker)
                .map(|entity| entity.mission_state.clone()),
        );
        observed_collections.extend(
            frame
                .payload
                .ditto_documents
                .iter()
                .map(|document| document.collection.clone()),
        );
        observed_queue |= frame.payload.base.as_ref().is_some_and(|base| {
            !base.queue.is_empty() || base.occupied_slots.len() == base.reload_slots
        });
        final_intensity = frame
            .payload
            .fire_cells
            .iter()
            .map(|cell| cell.intensity)
            .sum();
        if observed_states.len() == 7 && observed_queue && final_intensity < initial_intensity {
            break;
        }
    }

    assert_eq!(
        observed_states,
        BTreeSet::from([
            "holding".to_owned(),
            "enroute_to_fire".to_owned(),
            "on_station".to_owned(),
            "dropping".to_owned(),
            "egress".to_owned(),
            "enroute_to_base".to_owned(),
            "reloading".to_owned(),
        ])
    );
    assert!(final_intensity < initial_intensity);
    assert!(
        observed_queue,
        "expected the three-slot reload constraint to be exercised"
    );
    assert!(observed_collections.contains(FIRE_CELLS_COLLECTION));
    assert!(observed_collections.contains(BASE_QUEUE_COLLECTION));
    assert!(observed_collections.contains(DROP_ASSIGNMENTS_COLLECTION));
}
