use serde::Serialize;

use crate::model::Entity;

pub const SCHEMA: &str = "autonomy-sim/v1";

#[derive(Clone, Debug, Serialize)]
pub struct HelloEnvelope {
    pub schema: &'static str,
    pub message_type: &'static str,
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
    pub sequence: u64,
    pub sim_time_s: f64,
    pub payload: StatePayload,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatePayload {
    pub entities: Vec<Entity>,
    pub links: Vec<serde_json::Value>,
    pub link_events: Vec<serde_json::Value>,
    pub traffic: Vec<serde_json::Value>,
    pub czml: Vec<serde_json::Value>,
}

impl StateEnvelope {
    pub fn empty() -> Self {
        Self {
            schema: SCHEMA,
            message_type: "state",
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
            "kind": entity.kind,
            "domain": entity.domain,
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::model::{Domain, Entity, EntityKind, Kinematics, MissionState, Position};

    #[test]
    fn czml_uses_longitude_latitude_order() {
        let entity = Entity {
            id: "one".into(),
            name: "One".into(),
            kind: EntityKind::Drone,
            domain: Domain::Air,
            position: Position {
                lat_deg: 34.0,
                lon_deg: -117.0,
                alt_m: 100.0,
            },
            kinematics: Kinematics::default(),
            mission: MissionState::default(),
        };
        let packet = super::entity_czml(&entity);
        assert_eq!(
            packet["position"]["cartographicDegrees"],
            serde_json::json!([-117.0, 34.0, 100.0])
        );
    }
}
