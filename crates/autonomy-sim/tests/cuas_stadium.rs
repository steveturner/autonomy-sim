use std::collections::BTreeSet;

use autonomy_sim::{
    Simulation,
    ditto::{CUAS_ENGAGEMENTS_COLLECTION, CUAS_EW_ASSIGNMENTS_COLLECTION, CUAS_TRACKS_COLLECTION},
    model::{Affiliation, EntityKind},
    scenario::ScenarioRegistry,
};

#[test]
fn stadium_funnel_runs_end_to_end_and_excludes_threats_from_ditto() {
    let scenarios = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios");
    let config = ScenarioRegistry::new(scenarios)
        .load("cuas-stadium")
        .unwrap();
    let mut simulation = Simulation::try_new(&config).unwrap();
    let initial = simulation.snapshot().unwrap();
    let threats: Vec<_> = initial
        .payload
        .entities
        .iter()
        .filter(|entity| entity.kind == EntityKind::ThreatUas)
        .collect();
    assert_eq!(threats.len(), 8);
    assert!(threats.iter().all(|entity| {
        entity.affiliation == Affiliation::Hostile
            && entity.sidc == "SHAPMFQ--------"
            && entity.mission_state == "inbound"
    }));
    assert!(
        initial
            .payload
            .ditto_peers
            .iter()
            .all(|peer| { !threats.iter().any(|threat| threat.id == peer.entity_id) })
    );
    assert_eq!(initial.payload.ditto_peers.len(), 8);

    let mut states = BTreeSet::from(["inbound".to_owned()]);
    let mut collections = BTreeSet::new();
    let mut saw_ew_capacity_leak = false;
    let mut saw_interceptor_capacity_leak = false;
    let mut saw_abstract_gun = false;

    for _ in 0..1_200 {
        let frame = simulation.tick().unwrap();
        states.extend(
            frame
                .payload
                .entities
                .iter()
                .filter(|entity| entity.kind == EntityKind::ThreatUas)
                .map(|entity| entity.mission_state.clone()),
        );
        for document in &frame.payload.ditto_documents {
            collections.insert(document.collection.clone());
            saw_ew_capacity_leak |= document.collection == CUAS_EW_ASSIGNMENTS_COLLECTION
                && document.value["capacity_limited"] == true;
            saw_interceptor_capacity_leak |= document.collection == CUAS_ENGAGEMENTS_COLLECTION
                && document.value["layer"] == "interceptor"
                && document.value["capacity_limited"] == true;
            saw_abstract_gun |= document.collection == CUAS_ENGAGEMENTS_COLLECTION
                && document.value["layer"] == "gun"
                && document.value["abstract_effect"] == true;
        }
        if states.contains("engaged_gun")
            && states.contains("intercepted")
            && states.contains("neutralized")
            && states.contains("leaked")
        {
            break;
        }
    }

    for expected in [
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
            states.contains(expected),
            "missing funnel state {expected}: {states:?}"
        );
    }
    assert!(collections.contains(CUAS_TRACKS_COLLECTION));
    assert!(collections.contains(CUAS_EW_ASSIGNMENTS_COLLECTION));
    assert!(collections.contains(CUAS_ENGAGEMENTS_COLLECTION));
    assert!(saw_ew_capacity_leak);
    assert!(saw_interceptor_capacity_leak);
    assert!(saw_abstract_gun);
}
