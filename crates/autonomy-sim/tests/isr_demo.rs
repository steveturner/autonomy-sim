use std::collections::BTreeSet;

use autonomy_sim::{ScenarioConfig, Simulation};

#[test]
fn demo_exercises_every_transport_and_link_flaps() {
    let scenario_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/isr-demo.toml");
    let mut config = ScenarioConfig::from_path(scenario_path).unwrap();
    config.cot.sink = "disabled".into();
    let mut simulation = Simulation::try_new(&config).unwrap();
    let mut transport_types = BTreeSet::new();
    let mut down_events = 0;
    let mut up_events = 0;

    for _ in 0..1_100 {
        let frame = simulation.tick().unwrap();
        transport_types.extend(frame.payload.links.iter().map(|link| link.link_type));
        for event in frame.payload.link_events {
            match event.state {
                autonomy_sim::network::LinkStatus::Up => up_events += 1,
                autonomy_sim::network::LinkStatus::Down => down_events += 1,
            }
        }
    }

    assert_eq!(transport_types.len(), 4);
    assert!(
        down_events >= 2,
        "expected multiple link drops, got {down_events}"
    );
    assert!(
        up_events >= 2,
        "expected multiple link restorations, got {up_events}"
    );
}
