//! Defensive C-UAS coordination funnel.
//!
//! Effects are intentionally abstract state transitions driven by deterministic
//! probabilities. This module contains no fire-control solution, ballistics,
//! aiming, or vehicle guidance logic.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};

use crate::{
    ditto::{
        CUAS_ENGAGEMENTS_COLLECTION, CUAS_EW_ASSIGNMENTS_COLLECTION, CUAS_TRACKS_COLLECTION,
        CoordinationDocument,
    },
    ditto_transport::DittoRuntime,
    model::{Entity, EntityKind, Kinematics, Position},
    scenario::CuasConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThreatPhase {
    Inbound,
    Detected,
    Jammed,
    EwLeak,
    Intercepted,
    InterceptorLeak,
    EngagedGun,
    Neutralized,
    Leaked,
}

impl ThreatPhase {
    fn mission_state(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Detected => "detected",
            Self::Jammed => "jammed",
            Self::EwLeak | Self::InterceptorLeak => "leaking",
            Self::Intercepted => "intercepted",
            Self::EngagedGun => "engaged_gun",
            Self::Neutralized => "neutralized",
            Self::Leaked => "leaked",
        }
    }

    fn terminal(self) -> bool {
        matches!(self, Self::Neutralized | Self::Leaked)
    }
}

#[derive(Clone, Debug)]
struct ThreatMission {
    phase: ThreatPhase,
    phase_since_s: f64,
    detected_at_s: Option<f64>,
    ew_asset: Option<String>,
    ew_considered: bool,
    ew_success: Option<bool>,
    interceptor_asset: Option<String>,
    interceptor_considered: bool,
    interceptor_started_s: Option<f64>,
    interceptor_success: Option<bool>,
    gun_asset: Option<String>,
    gun_started_s: Option<f64>,
    gun_success: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct CuasEntityUpdate {
    pub entity_id: String,
    pub position: Position,
    pub kinematics: Kinematics,
    pub mission_state: String,
}

pub struct CuasTick {
    pub entity_updates: Vec<CuasEntityUpdate>,
    pub coordination_documents: Vec<CoordinationDocument>,
}

pub struct CuasRuntime {
    config: CuasConfig,
    seed: u64,
    site_position: Position,
    radar_ids: Vec<String>,
    jammer_ids: Vec<String>,
    interceptor_ids: Vec<String>,
    gun_ids: Vec<String>,
    threats: BTreeMap<String, ThreatMission>,
    ew_assignments: usize,
    interceptor_assignments: usize,
}

impl CuasRuntime {
    pub fn new(config: &CuasConfig, seed: u64, entities: &[Entity]) -> Result<Self> {
        let site_position = entities
            .iter()
            .find(|entity| entity.id == config.protected_site_id)
            .map(|entity| entity.position)
            .ok_or_else(|| {
                anyhow!(
                    "C-UAS protected site '{}' is missing",
                    config.protected_site_id
                )
            })?;
        let ids_for = |kind| {
            entities
                .iter()
                .filter(|entity| entity.kind == kind)
                .map(|entity| entity.id.clone())
                .collect::<Vec<_>>()
        };
        let threats = entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::ThreatUas)
            .map(|entity| {
                (
                    entity.id.clone(),
                    ThreatMission {
                        phase: ThreatPhase::Inbound,
                        phase_since_s: 0.0,
                        detected_at_s: None,
                        ew_asset: None,
                        ew_considered: false,
                        ew_success: None,
                        interceptor_asset: None,
                        interceptor_considered: false,
                        interceptor_started_s: None,
                        interceptor_success: None,
                        gun_asset: None,
                        gun_started_s: None,
                        gun_success: None,
                    },
                )
            })
            .collect();
        Ok(Self {
            config: config.clone(),
            seed,
            site_position,
            radar_ids: ids_for(EntityKind::RadarSensor),
            jammer_ids: ids_for(EntityKind::EwJammer),
            interceptor_ids: ids_for(EntityKind::Interceptor),
            gun_ids: ids_for(EntityKind::GunSystem),
            threats,
            ew_assignments: 0,
            interceptor_assignments: 0,
        })
    }

    pub fn initialize_entity(&self, entity: &mut Entity) {
        entity.mission_state = match entity.kind {
            EntityKind::ThreatUas => "inbound",
            EntityKind::ProtectedSite => "protected",
            EntityKind::RadarSensor => "monitoring",
            EntityKind::EwJammer | EntityKind::Interceptor | EntityKind::GunSystem => "ready",
            _ => return,
        }
        .into();
        entity.mission.active_node = entity.mission_state.clone();
        if entity.kind == EntityKind::ThreatUas {
            entity.heading_deg = entity.position.bearing_to(self.site_position);
            entity.kinematics.heading_deg = entity.heading_deg;
        }
    }

    pub fn tick(
        &mut self,
        entities: &[Entity],
        dt_s: f64,
        sim_time_s: f64,
        ditto: &DittoRuntime,
    ) -> CuasTick {
        let entity_by_id: BTreeMap<_, _> = entities
            .iter()
            .map(|entity| (entity.id.as_str(), entity))
            .collect();
        let mut entity_updates = Vec::new();
        let threat_ids: Vec<_> = self.threats.keys().cloned().collect();

        for threat_id in threat_ids {
            let Some(entity) = entity_by_id.get(threat_id.as_str()).copied() else {
                continue;
            };
            let mission = self.threats.get_mut(&threat_id).expect("known threat");
            let mut position = entity.position;
            let mut kinematics = entity.kinematics;
            let distance_to_site = horizontal_distance(position, self.site_position);

            if !mission.phase.terminal()
                && !matches!(
                    mission.phase,
                    ThreatPhase::Jammed | ThreatPhase::Intercepted
                )
            {
                kinematics.heading_deg = position.bearing_to(self.site_position);
                position = position.moved_toward(
                    altitude_matched(self.site_position, position.alt_m),
                    kinematics.speed_mps * dt_s,
                );
            } else {
                kinematics.speed_mps = 0.0;
            }

            match mission.phase {
                ThreatPhase::Inbound if distance_to_site <= self.config.detection_range_m => {
                    transition(mission, ThreatPhase::Detected, sim_time_s);
                    mission.detected_at_s = Some(sim_time_s);
                }
                ThreatPhase::Detected => {
                    let elapsed = sim_time_s - mission.phase_since_s;
                    if !mission.ew_considered
                        && elapsed >= self.config.detection_delay_s
                        && any_peer_has_document(
                            ditto,
                            &self.jammer_ids,
                            CUAS_TRACKS_COLLECTION,
                            &format!("track/{threat_id}"),
                        )
                    {
                        mission.ew_considered = true;
                        if self.ew_assignments < self.config.ew_capacity {
                            let slot = self.ew_assignments;
                            self.ew_assignments += 1;
                            mission.ew_asset =
                                self.jammer_ids.get(slot % self.jammer_ids.len()).cloned();
                            mission.ew_success = Some(effect_succeeds(
                                self.seed,
                                &threat_id,
                                "ew",
                                self.config.ew_success_probability,
                            ));
                        }
                    }
                    if mission.ew_considered {
                        if mission.ew_asset.is_none() {
                            transition(mission, ThreatPhase::EwLeak, sim_time_s);
                        } else if elapsed
                            >= self.config.detection_delay_s + self.config.ew_effect_delay_s
                        {
                            let phase = if mission.ew_success == Some(true) {
                                ThreatPhase::Jammed
                            } else {
                                ThreatPhase::EwLeak
                            };
                            transition(mission, phase, sim_time_s);
                        }
                    }
                }
                ThreatPhase::Jammed
                    if sim_time_s - mission.phase_since_s >= self.config.ew_effect_delay_s =>
                {
                    transition(mission, ThreatPhase::Neutralized, sim_time_s);
                }
                ThreatPhase::EwLeak => {
                    if !mission.interceptor_considered
                        && distance_to_site <= self.config.interceptor_range_m
                        && any_peer_has_document(
                            ditto,
                            &self.interceptor_ids,
                            CUAS_EW_ASSIGNMENTS_COLLECTION,
                            &format!("ew-assignment/{threat_id}"),
                        )
                    {
                        mission.interceptor_considered = true;
                        if self.interceptor_assignments < self.config.interceptor_capacity {
                            let slot = self.interceptor_assignments;
                            self.interceptor_assignments += 1;
                            mission.interceptor_asset = self
                                .interceptor_ids
                                .get(slot % self.interceptor_ids.len())
                                .cloned();
                            mission.interceptor_started_s = Some(sim_time_s);
                            mission.interceptor_success = Some(effect_succeeds(
                                self.seed,
                                &threat_id,
                                "interceptor",
                                self.config.intercept_success_probability,
                            ));
                        }
                    }
                    if let Some(started_s) = mission.interceptor_started_s
                        && sim_time_s - started_s >= self.config.intercept_time_s
                    {
                        let phase = if mission.interceptor_success == Some(true) {
                            ThreatPhase::Intercepted
                        } else {
                            ThreatPhase::InterceptorLeak
                        };
                        transition(mission, phase, sim_time_s);
                    } else if mission.interceptor_considered
                        && mission.interceptor_asset.is_none()
                        && distance_to_site <= self.config.gun_range_m
                        && any_peer_has_document(
                            ditto,
                            &self.gun_ids,
                            CUAS_ENGAGEMENTS_COLLECTION,
                            &format!("engagement/interceptor/{threat_id}"),
                        )
                    {
                        engage_gun(
                            mission,
                            &self.gun_ids,
                            self.seed,
                            &threat_id,
                            self.config.gun_success_probability,
                            sim_time_s,
                        );
                    }
                }
                ThreatPhase::Intercepted
                    if sim_time_s - mission.phase_since_s >= self.config.intercept_time_s =>
                {
                    transition(mission, ThreatPhase::Neutralized, sim_time_s);
                }
                ThreatPhase::InterceptorLeak
                    if distance_to_site <= self.config.gun_range_m
                        && any_peer_has_document(
                            ditto,
                            &self.gun_ids,
                            CUAS_ENGAGEMENTS_COLLECTION,
                            &format!("engagement/interceptor/{threat_id}"),
                        ) =>
                {
                    engage_gun(
                        mission,
                        &self.gun_ids,
                        self.seed,
                        &threat_id,
                        self.config.gun_success_probability,
                        sim_time_s,
                    );
                }
                ThreatPhase::EngagedGun
                    if sim_time_s - mission.phase_since_s >= self.config.gun_effect_delay_s =>
                {
                    let phase = if mission.gun_success == Some(true) {
                        ThreatPhase::Neutralized
                    } else {
                        ThreatPhase::Leaked
                    };
                    transition(mission, phase, sim_time_s);
                }
                _ => {}
            }

            if horizontal_distance(position, self.site_position) <= 25.0
                && !matches!(
                    mission.phase,
                    ThreatPhase::Neutralized
                        | ThreatPhase::Jammed
                        | ThreatPhase::Intercepted
                        | ThreatPhase::EngagedGun
                )
            {
                transition(mission, ThreatPhase::Leaked, sim_time_s);
            }
            entity_updates.push(CuasEntityUpdate {
                entity_id: threat_id,
                position,
                kinematics,
                mission_state: mission.phase.mission_state().into(),
            });
        }

        entity_updates.extend(self.defender_updates());
        CuasTick {
            coordination_documents: self.coordination_documents(&entity_updates, sim_time_s),
            entity_updates,
        }
    }

    fn defender_updates(&self) -> Vec<CuasEntityUpdate> {
        let jammer_active = self.threats.values().any(|mission| {
            mission.ew_asset.is_some() && matches!(mission.phase, ThreatPhase::Detected)
        });
        let interceptor_active = self.threats.values().any(|mission| {
            mission.interceptor_asset.is_some() && matches!(mission.phase, ThreatPhase::EwLeak)
        });
        let gun_active = self
            .threats
            .values()
            .any(|mission| mission.phase == ThreatPhase::EngagedGun);
        self.radar_ids
            .iter()
            .map(|id| (id, "tracking"))
            .chain(
                self.jammer_ids
                    .iter()
                    .map(|id| (id, if jammer_active { "jamming" } else { "ready" })),
            )
            .chain(self.interceptor_ids.iter().map(|id| {
                (
                    id,
                    if interceptor_active {
                        "intercepting"
                    } else {
                        "ready"
                    },
                )
            }))
            .chain(
                self.gun_ids
                    .iter()
                    .map(|id| (id, if gun_active { "engaging" } else { "ready" })),
            )
            .map(|(id, state)| CuasEntityUpdate {
                entity_id: id.clone(),
                position: Position::default(),
                kinematics: Kinematics::default(),
                mission_state: state.into(),
            })
            .collect()
    }

    fn coordination_documents(
        &self,
        updates: &[CuasEntityUpdate],
        sim_time_s: f64,
    ) -> Vec<CoordinationDocument> {
        let positions: BTreeMap<_, _> = updates
            .iter()
            .map(|update| (update.entity_id.as_str(), update.position))
            .collect();
        let radar = &self.radar_ids[0];
        let ew_coordinator = &self.jammer_ids[0];
        let engagement_coordinator = &self.interceptor_ids[0];
        let mut documents = Vec::new();
        for (threat_id, mission) in &self.threats {
            if mission.detected_at_s.is_some() {
                documents.push(CoordinationDocument {
                    collection: CUAS_TRACKS_COLLECTION,
                    document_id: format!("track/{threat_id}"),
                    author_entity_id: radar.clone(),
                    value: serde_json::json!({
                        "threat_id": threat_id,
                        "position": positions.get(threat_id.as_str()),
                        "mission_state": mission.phase.mission_state(),
                        "detected_at_s": mission.detected_at_s,
                        "updated_at_s": sim_time_s,
                        "classification": "simulated_hostile_uas",
                    }),
                });
            }
            if mission.ew_considered {
                documents.push(CoordinationDocument {
                    collection: CUAS_EW_ASSIGNMENTS_COLLECTION,
                    document_id: format!("ew-assignment/{threat_id}"),
                    author_entity_id: mission
                        .ew_asset
                        .clone()
                        .unwrap_or_else(|| ew_coordinator.clone()),
                    value: serde_json::json!({
                        "threat_id": threat_id,
                        "assigned_asset": mission.ew_asset,
                        "capacity_limited": mission.ew_asset.is_none(),
                        "abstract_effect": true,
                        "status": layer_status(mission, "ew"),
                    }),
                });
            }
            if mission.interceptor_considered {
                documents.push(CoordinationDocument {
                    collection: CUAS_ENGAGEMENTS_COLLECTION,
                    document_id: format!("engagement/interceptor/{threat_id}"),
                    author_entity_id: mission
                        .interceptor_asset
                        .clone()
                        .unwrap_or_else(|| engagement_coordinator.clone()),
                    value: serde_json::json!({
                        "threat_id": threat_id,
                        "layer": "interceptor",
                        "assigned_asset": mission.interceptor_asset,
                        "capacity_limited": mission.interceptor_asset.is_none(),
                        "abstract_effect": true,
                        "status": layer_status(mission, "interceptor"),
                    }),
                });
            }
            if let Some(gun_id) = &mission.gun_asset {
                documents.push(CoordinationDocument {
                    collection: CUAS_ENGAGEMENTS_COLLECTION,
                    document_id: format!("engagement/gun/{threat_id}"),
                    author_entity_id: gun_id.clone(),
                    value: serde_json::json!({
                        "threat_id": threat_id,
                        "layer": "gun",
                        "assigned_asset": gun_id,
                        "abstract_effect": true,
                        "status": layer_status(mission, "gun"),
                        "safety_note": "simulation-only abstract effect; no ballistics or targeting",
                    }),
                });
            }
        }
        documents
    }
}

fn engage_gun(
    mission: &mut ThreatMission,
    gun_ids: &[String],
    seed: u64,
    threat_id: &str,
    probability: f64,
    sim_time_s: f64,
) {
    if mission.gun_asset.is_some() {
        return;
    }
    mission.gun_asset = gun_ids.first().cloned();
    mission.gun_started_s = Some(sim_time_s);
    mission.gun_success = Some(effect_succeeds(seed, threat_id, "gun", probability));
    transition(mission, ThreatPhase::EngagedGun, sim_time_s);
}

fn transition(mission: &mut ThreatMission, phase: ThreatPhase, sim_time_s: f64) {
    mission.phase = phase;
    mission.phase_since_s = sim_time_s;
}

fn layer_status(mission: &ThreatMission, layer: &str) -> &'static str {
    match layer {
        "ew" if mission.ew_asset.is_none() => "capacity_leak",
        "ew" if matches!(mission.phase, ThreatPhase::Detected) => "assigned",
        "ew" if mission.ew_success == Some(true) => "effective",
        "ew" => "leaked",
        "interceptor" if mission.interceptor_asset.is_none() => "capacity_leak",
        "interceptor" if matches!(mission.phase, ThreatPhase::EwLeak) => "assigned",
        "interceptor" if mission.interceptor_success == Some(true) => "effective",
        "interceptor" => "leaked",
        "gun" if mission.phase == ThreatPhase::EngagedGun => "engaged",
        "gun" if mission.gun_success == Some(true) => "effective",
        "gun" => "leaked",
        _ => "pending",
    }
}

fn effect_succeeds(seed: u64, threat_id: &str, layer: &str, probability: f64) -> bool {
    let hash = threat_id
        .bytes()
        .chain(layer.bytes())
        .fold(seed ^ 0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    let roll = (hash % 1_000_000) as f64 / 1_000_000.0;
    roll < probability
}

fn any_peer_has_document(
    ditto: &DittoRuntime,
    entity_ids: &[String],
    collection: &str,
    document_id: &str,
) -> bool {
    entity_ids.iter().any(|entity_id| {
        ditto
            .peer_has_latest(entity_id, collection, document_id)
            .unwrap_or(false)
    })
}

fn horizontal_distance(left: Position, right: Position) -> f64 {
    left.distance_to(altitude_matched(right, left.alt_m))
}

fn altitude_matched(mut position: Position, altitude_m: f64) -> Position {
    position.alt_m = altitude_m;
    position
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_roll_is_reproducible() {
        assert_eq!(
            effect_succeeds(42, "threat-01", "ew", 0.5),
            effect_succeeds(42, "threat-01", "ew", 0.5)
        );
    }

    #[test]
    fn public_states_match_the_defensive_funnel() {
        assert_eq!(ThreatPhase::Inbound.mission_state(), "inbound");
        assert_eq!(ThreatPhase::Detected.mission_state(), "detected");
        assert_eq!(ThreatPhase::Jammed.mission_state(), "jammed");
        assert_eq!(ThreatPhase::EwLeak.mission_state(), "leaking");
        assert_eq!(ThreatPhase::Intercepted.mission_state(), "intercepted");
        assert_eq!(ThreatPhase::EngagedGun.mission_state(), "engaged_gun");
        assert_eq!(ThreatPhase::Neutralized.mission_state(), "neutralized");
        assert_eq!(ThreatPhase::Leaked.mission_state(), "leaked");
    }
}
