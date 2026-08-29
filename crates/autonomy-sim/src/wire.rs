use serde::Serialize;

use crate::{
    ditto::{DittoDocumentState, DittoPeerState, DittoReplicationEvent, is_ditto_peer, peer_id},
    model::{Entity, LinkType, Position},
    network::{LinkEvent, LinkState, LinkStatus, TrafficState},
};

pub const SCHEMA: &str = "autonomy-sim/v1";

#[derive(Clone, Debug, Serialize)]
pub struct HelloEnvelope {
    pub schema: &'static str,
    pub message_type: &'static str,
    pub scenario: String,
    pub sequence: u64,
    pub sim_time_s: f64,
    pub payload: HelloPayload,
}

#[derive(Clone, Debug, Serialize)]
pub struct HelloPayload {
    pub scenario: String,
    pub tick_hz: f64,
    pub server: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct StateEnvelope {
    pub schema: &'static str,
    pub message_type: &'static str,
    pub scenario: String,
    pub sequence: u64,
    pub sim_time_s: f64,
    pub payload: StatePayload,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatePayload {
    pub entities: Vec<Entity>,
    pub links: Vec<LinkState>,
    pub link_events: Vec<LinkEvent>,
    pub traffic: Vec<TrafficState>,
    pub ditto_peers: Vec<DittoPeerState>,
    pub ditto_documents: Vec<DittoDocumentState>,
    pub ditto_replication_events: Vec<DittoReplicationEvent>,
    pub fire_cells: Vec<FireCellState>,
    pub base: Option<BaseState>,
    pub czml: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FireCellState {
    pub id: String,
    pub position: Position,
    pub intensity: f64,
    pub assigned_tanker: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BaseState {
    pub id: String,
    pub name: String,
    pub position: Position,
    pub reload_slots: usize,
    pub occupied_slots: Vec<String>,
    pub queue: Vec<String>,
}

pub fn link_czml(
    link: &LinkState,
    positions: &std::collections::BTreeMap<String, Position>,
) -> Option<serde_json::Value> {
    if link.state != LinkStatus::Up {
        return None;
    }
    let source = positions.get(&link.source)?;
    let target = positions.get(&link.target)?;
    let color = match link.link_type {
        LinkType::Mesh => [34, 211, 238, 220],
        LinkType::Cellular => [232, 121, 249, 220],
        LinkType::Satcom => [251, 191, 36, 220],
        LinkType::Ble => [74, 222, 128, 220],
    };
    Some(serde_json::json!({
        "id": link.id,
        "name": format!("Ditto over {}: {} ↔ {}", link.link_type, link.source, link.target),
        "polyline": {
            "positions": { "cartographicDegrees": [
                source.lon_deg, source.lat_deg, source.alt_m,
                target.lon_deg, target.lat_deg, target.alt_m
            ]},
            "material": { "solidColor": { "color": { "rgba": color }}},
            "width": 1.0 + link.quality * 3.0
        },
        "properties": {
            "source": link.source,
            "target": link.target,
            "source_peer_id": link.source_peer_id,
            "target_peer_id": link.target_peer_id,
            "link_type": link.link_type,
            "quality": link.quality,
            "traffic_units": "bits_per_second"
        }
    }))
}

impl StateEnvelope {
    pub fn empty() -> Self {
        Self {
            schema: SCHEMA,
            message_type: "state",
            scenario: String::new(),
            sequence: 0,
            sim_time_s: 0.0,
            payload: StatePayload::default(),
        }
    }
}

pub fn entity_czml(entity: &Entity) -> serde_json::Value {
    let color = match entity.domain {
        crate::model::Domain::Air => [65, 191, 255, 255],
        crate::model::Domain::Ground => [76, 230, 154, 255],
        crate::model::Domain::Maritime => [49, 120, 198, 255],
        crate::model::Domain::Space => [255, 202, 58, 255],
    };
    let ditto_peer_id = is_ditto_peer(entity).then(|| peer_id(&entity.id));
    serde_json::json!({
        "id": format!("entity/{}", entity.id),
        "name": entity.name,
        "position": {
            "cartographicDegrees": [entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m]
        },
        "point": { "pixelSize": 12, "color": { "rgba": color } },
        "label": { "text": entity.name },
        "properties": {
            "entity_id": entity.id,
            "ditto_peer_id": ditto_peer_id,
            "kind": entity.kind,
            "affiliation": entity.affiliation,
            "sidc": entity.sidc,
            "icon_hint": entity.icon_hint,
            "domain": entity.domain,
            "mission_role": entity.mission_role,
            "mission_state": entity.mission_state,
            "heading_deg": entity.heading_deg,
            "retardant_pct": entity.retardant_pct,
            "intensity": entity.intensity,
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        model::{Affiliation, Domain, Entity, EntityKind, Kinematics, MissionState, Position},
        symbology::{SymbolStatus, icon_hint, sidc},
    };

    #[test]
    fn czml_uses_longitude_latitude_order() {
        let entity = Entity {
            id: "one".into(),
            name: "One".into(),
            kind: EntityKind::Uas,
            affiliation: Affiliation::Friendly,
            sidc: sidc(
                EntityKind::Uas,
                Affiliation::Friendly,
                SymbolStatus::Present,
            ),
            icon_hint: icon_hint(EntityKind::Uas).into(),
            domain: Domain::Air,
            position: Position {
                lat_deg: 34.0,
                lon_deg: -117.0,
                alt_m: 100.0,
            },
            kinematics: Kinematics::default(),
            mission: MissionState::default(),
            mission_role: "scout".into(),
            mission_state: "holding".into(),
            heading_deg: 0.0,
            retardant_pct: None,
            intensity: None,
            radios: Vec::new(),
        };
        let packet = super::entity_czml(&entity);
        assert_eq!(
            packet["position"]["cartographicDegrees"],
            serde_json::json!([-117.0, 34.0, 100.0])
        );
    }
}
