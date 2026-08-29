use std::{
    env,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use autonomy_sim::{
    ditto::{TELEMETRY_COLLECTION, peer_id},
    model::{
        Affiliation, Domain, Entity, EntityKind, Kinematics, LinkType, MissionState, Position,
    },
    network::{LinkState, LinkStatus},
};
use autonomy_sim_ditto_real::{RealDittoConfig, RealDittoTransport};
use serde_json::json;

fn entity(id: &str) -> Entity {
    Entity {
        id: id.into(),
        name: id.into(),
        kind: EntityKind::Uas,
        affiliation: Affiliation::Friendly,
        sidc: String::new(),
        icon_hint: String::new(),
        domain: Domain::Air,
        position: Position::default(),
        kinematics: Kinematics::default(),
        mission: MissionState::default(),
        mission_role: String::new(),
        mission_state: String::new(),
        heading_deg: 0.0,
        retardant_pct: None,
        intensity: None,
        radios: Vec::new(),
    }
}

fn link(state: LinkStatus) -> LinkState {
    LinkState {
        id: "link/mesh/alpha/bravo".into(),
        source: "alpha".into(),
        target: "bravo".into(),
        source_peer_id: peer_id("alpha"),
        target_peer_id: peer_id("bravo"),
        link_type: LinkType::Mesh,
        state,
        quality: f64::from(state == LinkStatus::Up),
        distance_m: 1.0,
        latency_ms: 1.0,
        packet_loss: f64::from(state == LinkStatus::Down),
        capacity_bps: if state == LinkStatus::Up {
            1_000_000
        } else {
            0
        },
    }
}

fn wait_for_document(
    transport: &RealDittoTransport,
    entity_id: &str,
    document_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while transport
        .read_document(entity_id, TELEMETRY_COLLECTION, document_id)?
        .is_none()
    {
        if Instant::now() >= deadline {
            return Err(format!(
                "real Ditto peers did not converge {document_id} within 15 seconds"
            )
            .into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let license = env::var("DITTO_LICENSE")
        .map_err(|_| "DITTO_LICENSE must contain an offline Ditto license")?;
    let port_base = env::var("DITTO_REAL_PORT_BASE")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(46_000);
    let mode = env::var("DITTO_REAL_MODE").unwrap_or_else(|_| "gated".into());
    if !matches!(mode.as_str(), "converge" | "gated") {
        return Err("DITTO_REAL_MODE must be 'converge' or 'gated'".into());
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let storage_root = env::var_os("DITTO_REAL_STORAGE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("target/ditto-real-demo").join(format!("{}-{nonce}", std::process::id()))
        });
    let mut transport = RealDittoTransport::new(
        &[entity("alpha"), entity("bravo")],
        RealDittoConfig {
            database_id: "00000005-0000-0000-0000-000000000000".into(),
            license,
            storage_root: storage_root.clone(),
            port_base,
            listen_ip: "127.0.0.1".into(),
        },
    )?;

    println!(
        "started two real Ditto peers; stores: {}",
        storage_root.display()
    );
    println!("link is Up");
    transport.apply_links(&[link(LinkStatus::Up)])?;
    if mode == "gated" {
        transport.write_document(
            "alpha",
            TELEMETRY_COLLECTION,
            "telemetry/warmup",
            json!({"phase": "connected"}),
            0.0,
        )?;
        wait_for_document(&transport, "bravo", "telemetry/warmup")?;
        println!("confirmed: peers synchronized over the live link");
        println!("bringing link down");
        transport.apply_links(&[link(LinkStatus::Down)])?;
        thread::sleep(Duration::from_millis(200));
    }
    println!("writing telemetry on alpha");
    transport.write_document(
        "alpha",
        TELEMETRY_COLLECTION,
        "telemetry/alpha",
        json!({"battery_pct": 88}),
        1.0,
    )?;
    if mode == "gated" {
        thread::sleep(Duration::from_secs(1));
        assert!(
            transport
                .read_document("bravo", TELEMETRY_COLLECTION, "telemetry/alpha")?
                .is_none()
        );
        println!("confirmed: gated link blocked replication");
        println!("bringing link up");
        transport.apply_links(&[link(LinkStatus::Up)])?;
    }
    wait_for_document(&transport, "bravo", "telemetry/alpha")?;
    if mode == "gated" {
        println!("success: bravo received alpha's real Ditto document after link restoration");
    } else {
        println!("success: bravo received alpha's real Ditto document");
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&transport.observe(&[link(LinkStatus::Up)])?)?
    );
    Ok(())
}
