use std::collections::{BTreeMap, VecDeque};

use anyhow::{Result, anyhow};
use serde::Serialize;

use crate::{
    ditto::{
        BASE_QUEUE_COLLECTION, CoordinationDocument, DROP_ASSIGNMENTS_COLLECTION,
        FIRE_CELLS_COLLECTION,
    },
    model::{Entity, EntityKind, Kinematics, Position},
    scenario::WildfireConfig,
    swarm::{BoidState, steer},
    wire::{BaseState, FireCellState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirefightingState {
    Holding,
    EnrouteToFire,
    OnStation,
    Dropping,
    Egress,
    EnrouteToBase,
    Reloading,
}

impl FirefightingState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Holding => "holding",
            Self::EnrouteToFire => "enroute_to_fire",
            Self::OnStation => "on_station",
            Self::Dropping => "dropping",
            Self::Egress => "egress",
            Self::EnrouteToBase => "enroute_to_base",
            Self::Reloading => "reloading",
        }
    }
}

#[derive(Clone, Debug)]
struct TankerMission {
    state: FirefightingState,
    state_since_s: f64,
    retardant_pct: f64,
    target_cell: Option<String>,
    initial_heading_deg: f64,
}

#[derive(Clone, Debug)]
pub struct EntityUpdate {
    pub entity_id: String,
    pub position: Position,
    pub kinematics: Kinematics,
    pub mission_state: String,
    pub retardant_pct: f64,
}

pub struct WildfireTick {
    pub entity_updates: Vec<EntityUpdate>,
    pub coordination_documents: Vec<CoordinationDocument>,
}

pub struct WildfireRuntime {
    config: WildfireConfig,
    base: BaseState,
    supervisor_id: String,
    tankers: BTreeMap<String, TankerMission>,
    fire_cells: BTreeMap<String, FireCellState>,
    occupied_slots: Vec<String>,
    waiting: VecDeque<String>,
    next_launch_s: f64,
}

impl WildfireRuntime {
    pub fn new(config: &WildfireConfig, seed: u64, entities: &[Entity]) -> Result<Self> {
        let base_entity = entities
            .iter()
            .find(|entity| entity.id == config.base_id)
            .ok_or_else(|| anyhow!("wildfire base entity '{}' is missing", config.base_id))?;
        let first_fire = config
            .fire_cells
            .first()
            .ok_or_else(|| anyhow!("wildfire scenario has no fire cells"))?;
        let route_heading = base_entity.position.bearing_to(first_fire.position);
        let tankers = entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::AirTanker)
            .map(|entity| {
                let jitter = seeded_jitter(seed, &entity.id, 12.0);
                (
                    entity.id.clone(),
                    TankerMission {
                        state: FirefightingState::Holding,
                        state_since_s: 0.0,
                        retardant_pct: 100.0,
                        target_cell: None,
                        initial_heading_deg: (route_heading + jitter + 360.0) % 360.0,
                    },
                )
            })
            .collect();
        let fire_cells = config
            .fire_cells
            .iter()
            .map(|cell| {
                (
                    cell.id.clone(),
                    FireCellState {
                        id: cell.id.clone(),
                        position: cell.position,
                        intensity: cell.intensity,
                        assigned_tanker: None,
                        status: "available".into(),
                    },
                )
            })
            .collect();
        Ok(Self {
            config: config.clone(),
            base: BaseState {
                id: base_entity.id.clone(),
                name: base_entity.name.clone(),
                position: base_entity.position,
                reload_slots: config.reload_slots,
                occupied_slots: Vec::new(),
                queue: Vec::new(),
            },
            supervisor_id: config.supervisor_id.clone(),
            tankers,
            fire_cells,
            occupied_slots: Vec::new(),
            waiting: VecDeque::new(),
            next_launch_s: 0.0,
        })
    }

    pub fn initialize_entity(&self, entity: &mut Entity) {
        if let Some(tanker) = self.tankers.get(&entity.id) {
            entity.mission_state = tanker.state.as_str().into();
            entity.mission.active_node = entity.mission_state.clone();
            entity.retardant_pct = Some(tanker.retardant_pct);
            entity.heading_deg = tanker.initial_heading_deg;
            entity.kinematics.heading_deg = tanker.initial_heading_deg;
            entity.kinematics.speed_mps = 0.0;
        }
        if let Some(cell) = self.fire_cells.get(&entity.id) {
            entity.mission_state = cell.status.clone();
            entity.mission.active_node = "fire_model".into();
            entity.intensity = Some(cell.intensity);
        }
    }

    pub fn tick(&mut self, entities: &[Entity], dt_s: f64, sim_time_s: f64) -> WildfireTick {
        for cell in self.fire_cells.values_mut() {
            if cell.intensity > 0.5 {
                let growth = self.config.spread_per_s * dt_s * (1.0 - cell.intensity / 100.0);
                cell.intensity = (cell.intensity + growth).clamp(0.0, 100.0);
            }
        }

        let boids: Vec<_> = entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::AirTanker)
            .map(|entity| BoidState {
                id: entity.id.clone(),
                position: entity.position,
                heading_deg: entity.heading_deg,
                speed_mps: if entity.kinematics.speed_mps <= f64::EPSILON {
                    self.config.flocking.min_speed_mps
                } else {
                    entity.kinematics.speed_mps
                },
            })
            .collect();
        let entity_by_id: BTreeMap<_, _> = entities
            .iter()
            .map(|entity| (entity.id.as_str(), entity))
            .collect();
        let tanker_ids: Vec<_> = self.tankers.keys().cloned().collect();
        let mut entity_updates = Vec::with_capacity(tanker_ids.len());

        for tanker_id in tanker_ids {
            let Some(entity) = entity_by_id.get(tanker_id.as_str()).copied() else {
                continue;
            };
            let mut mission = self.tankers.remove(&tanker_id).expect("known tanker");
            let mut position = entity.position;
            let mut kinematics = entity.kinematics;
            let elapsed = (sim_time_s - mission.state_since_s).max(0.0);

            match mission.state {
                FirefightingState::Holding => {
                    kinematics.speed_mps = 0.0;
                    if sim_time_s >= self.next_launch_s
                        && let Some(cell_id) = self.assign_fire_cell(&tanker_id)
                    {
                        mission.target_cell = Some(cell_id);
                        transition(&mut mission, FirefightingState::EnrouteToFire, sim_time_s);
                        self.next_launch_s = sim_time_s + self.config.launch_interval_s;
                    }
                }
                FirefightingState::EnrouteToFire => {
                    if let Some(goal) = mission
                        .target_cell
                        .as_ref()
                        .and_then(|id| self.fire_cells.get(id))
                        .map(|cell| altitude_matched(cell.position, position.alt_m))
                    {
                        if horizontal_distance(position, goal) <= self.config.arrival_radius_m {
                            transition(&mut mission, FirefightingState::OnStation, sim_time_s);
                        } else {
                            fly_boid(
                                &tanker_id,
                                &boids,
                                goal,
                                self.config.flocking,
                                dt_s,
                                &mut position,
                                &mut kinematics,
                            );
                        }
                    } else {
                        transition(&mut mission, FirefightingState::EnrouteToBase, sim_time_s);
                    }
                }
                FirefightingState::OnStation => {
                    if elapsed >= self.config.on_station_s {
                        transition(&mut mission, FirefightingState::Dropping, sim_time_s);
                    } else if let Some(goal) = mission
                        .target_cell
                        .as_ref()
                        .and_then(|id| self.fire_cells.get(id))
                        .map(|cell| altitude_matched(cell.position, position.alt_m))
                    {
                        fly_boid(
                            &tanker_id,
                            &boids,
                            goal,
                            self.config.flocking,
                            dt_s,
                            &mut position,
                            &mut kinematics,
                        );
                    }
                }
                FirefightingState::Dropping => {
                    let fraction = (dt_s / self.config.drop_duration_s).clamp(0.0, 1.0);
                    mission.retardant_pct = (mission.retardant_pct - 100.0 * fraction).max(0.0);
                    if let Some(cell) = mission
                        .target_cell
                        .as_ref()
                        .and_then(|id| self.fire_cells.get_mut(id))
                    {
                        cell.status = "dropping".into();
                        cell.intensity =
                            (cell.intensity - self.config.drop_effect * fraction).max(0.0);
                    }
                    position =
                        position.moved(kinematics.heading_deg, kinematics.speed_mps * dt_s, 0.0);
                    if elapsed + dt_s >= self.config.drop_duration_s {
                        if let Some(cell) = mission
                            .target_cell
                            .as_ref()
                            .and_then(|id| self.fire_cells.get_mut(id))
                        {
                            cell.assigned_tanker = None;
                            cell.status = if cell.intensity <= 5.0 {
                                "contained".into()
                            } else {
                                "available".into()
                            };
                        }
                        transition(&mut mission, FirefightingState::Egress, sim_time_s);
                    }
                }
                FirefightingState::Egress => {
                    position =
                        position.moved(kinematics.heading_deg, kinematics.speed_mps * dt_s, 0.0);
                    if elapsed >= self.config.egress_time_s {
                        transition(&mut mission, FirefightingState::EnrouteToBase, sim_time_s);
                    }
                }
                FirefightingState::EnrouteToBase => {
                    let distance_to_base = horizontal_distance(position, self.base.position);
                    if distance_to_base <= self.config.base_arrival_radius_m {
                        if self.acquire_reload_slot(&tanker_id) {
                            transition(&mut mission, FirefightingState::Reloading, sim_time_s);
                            kinematics.speed_mps = 0.0;
                        } else {
                            self.enqueue(tanker_id.clone());
                            kinematics.speed_mps = 0.0;
                        }
                    } else {
                        let approach = self.base.position.moved(
                            (self.config.base_approach_lane_deg + 180.0) % 360.0,
                            self.config.approach_distance_m,
                            0.0,
                        );
                        let goal = if horizontal_distance(position, approach)
                            <= self.config.arrival_radius_m
                            || distance_to_base <= self.config.approach_distance_m
                        {
                            self.base.position
                        } else {
                            approach
                        };
                        fly_boid(
                            &tanker_id,
                            &boids,
                            altitude_matched(goal, position.alt_m),
                            self.config.flocking,
                            dt_s,
                            &mut position,
                            &mut kinematics,
                        );
                    }
                }
                FirefightingState::Reloading => {
                    kinematics.speed_mps = 0.0;
                    if elapsed >= self.config.reload_time_s {
                        self.release_reload_slot(&tanker_id);
                        mission.retardant_pct = 100.0;
                        mission.target_cell = None;
                        transition(&mut mission, FirefightingState::Holding, sim_time_s);
                    }
                }
            }

            kinematics.heading_deg = (kinematics.heading_deg + 360.0) % 360.0;
            entity_updates.push(EntityUpdate {
                entity_id: tanker_id.clone(),
                position,
                kinematics,
                mission_state: mission.state.as_str().into(),
                retardant_pct: mission.retardant_pct.clamp(0.0, 100.0),
            });
            self.tankers.insert(tanker_id, mission);
        }

        self.base.occupied_slots = self.occupied_slots.clone();
        self.base.queue = self.waiting.iter().cloned().collect();
        WildfireTick {
            entity_updates,
            coordination_documents: self.coordination_documents(),
        }
    }

    pub fn fire_cells(&self) -> Vec<FireCellState> {
        self.fire_cells.values().cloned().collect()
    }

    pub fn base_state(&self) -> BaseState {
        self.base.clone()
    }

    fn assign_fire_cell(&mut self, tanker_id: &str) -> Option<String> {
        let selected = self
            .fire_cells
            .values()
            .filter(|cell| cell.assigned_tanker.is_none() && cell.intensity > 5.0)
            .max_by(|left, right| {
                left.intensity
                    .total_cmp(&right.intensity)
                    .then_with(|| right.id.cmp(&left.id))
            })?
            .id
            .clone();
        let cell = self.fire_cells.get_mut(&selected).expect("selected cell");
        cell.assigned_tanker = Some(tanker_id.into());
        cell.status = "assigned".into();
        Some(selected)
    }

    fn acquire_reload_slot(&mut self, tanker_id: &str) -> bool {
        if self.occupied_slots.iter().any(|id| id == tanker_id) {
            return true;
        }
        if self.occupied_slots.len() >= self.config.reload_slots {
            return false;
        }
        if self.waiting.front().is_some_and(|id| id != tanker_id) {
            return false;
        }
        self.waiting.retain(|id| id != tanker_id);
        self.occupied_slots.push(tanker_id.into());
        self.occupied_slots.sort();
        true
    }

    fn enqueue(&mut self, tanker_id: String) {
        if !self.waiting.iter().any(|id| id == &tanker_id) {
            self.waiting.push_back(tanker_id);
        }
    }

    fn release_reload_slot(&mut self, tanker_id: &str) {
        self.occupied_slots.retain(|id| id != tanker_id);
    }

    fn coordination_documents(&self) -> Vec<CoordinationDocument> {
        let mut documents: Vec<_> = self
            .fire_cells
            .values()
            .map(|cell| CoordinationDocument {
                collection: FIRE_CELLS_COLLECTION,
                document_id: format!("fire-cell/{}", cell.id),
                author_entity_id: self.supervisor_id.clone(),
                value: serde_json::to_value(cell).expect("serializable fire cell"),
            })
            .collect();
        documents.push(CoordinationDocument {
            collection: BASE_QUEUE_COLLECTION,
            document_id: format!("base-queue/{}", self.base.id),
            author_entity_id: self.supervisor_id.clone(),
            value: serde_json::to_value(&self.base).expect("serializable base"),
        });
        documents.extend(
            self.tankers
                .iter()
                .map(|(tanker_id, mission)| CoordinationDocument {
                    collection: DROP_ASSIGNMENTS_COLLECTION,
                    document_id: format!("drop-assignment/{tanker_id}"),
                    author_entity_id: tanker_id.clone(),
                    value: serde_json::json!({
                        "tanker_id": tanker_id,
                        "fire_cell_id": mission.target_cell,
                        "mission_state": mission.state,
                        "retardant_pct": mission.retardant_pct,
                    }),
                }),
        );
        documents
    }
}

fn transition(mission: &mut TankerMission, state: FirefightingState, sim_time_s: f64) {
    mission.state = state;
    mission.state_since_s = sim_time_s;
}

fn fly_boid(
    tanker_id: &str,
    boids: &[BoidState],
    goal: Position,
    config: crate::swarm::FlockingConfig,
    dt_s: f64,
    position: &mut Position,
    kinematics: &mut Kinematics,
) {
    let Some(index) = boids.iter().position(|boid| boid.id == tanker_id) else {
        return;
    };
    let output = steer(index, boids, goal, config, dt_s);
    kinematics.heading_deg = output.heading_deg;
    kinematics.speed_mps = output.speed_mps;
    kinematics.vertical_speed_mps = 0.0;
    *position = position.moved(output.heading_deg, output.speed_mps * dt_s, 0.0);
}

fn horizontal_distance(left: Position, right: Position) -> f64 {
    left.distance_to(altitude_matched(right, left.alt_m))
}

fn altitude_matched(mut position: Position, altitude_m: f64) -> Position {
    position.alt_m = altitude_m;
    position
}

fn seeded_jitter(seed: u64, id: &str, amplitude_deg: f64) -> f64 {
    let hash = id
        .as_bytes()
        .iter()
        .fold(seed ^ 0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    let unit = (hash % 10_001) as f64 / 10_000.0;
    (unit * 2.0 - 1.0) * amplitude_deg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_names_match_wire_contract() {
        assert_eq!(FirefightingState::Holding.as_str(), "holding");
        assert_eq!(FirefightingState::EnrouteToFire.as_str(), "enroute_to_fire");
        assert_eq!(FirefightingState::OnStation.as_str(), "on_station");
        assert_eq!(FirefightingState::Dropping.as_str(), "dropping");
        assert_eq!(FirefightingState::Egress.as_str(), "egress");
        assert_eq!(FirefightingState::EnrouteToBase.as_str(), "enroute_to_base");
        assert_eq!(FirefightingState::Reloading.as_str(), "reloading");
    }

    #[test]
    fn seeded_heading_jitter_is_reproducible() {
        assert_eq!(
            seeded_jitter(42, "tanker-01", 10.0),
            seeded_jitter(42, "tanker-01", 10.0)
        );
        assert_ne!(
            seeded_jitter(42, "tanker-01", 10.0),
            seeded_jitter(43, "tanker-01", 10.0)
        );
    }
}
