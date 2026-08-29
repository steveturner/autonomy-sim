use std::collections::BTreeMap;

use crate::{
    model::{Entity, LinkType, MissionStatus, Position},
    network::{LinkState, LinkStatus},
    scenario::MissionConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BehaviorStatus {
    Running,
    Success,
    Failure,
}

impl From<BehaviorStatus> for MissionStatus {
    fn from(value: BehaviorStatus) -> Self {
        match value {
            BehaviorStatus::Running => Self::Running,
            BehaviorStatus::Success => Self::Success,
            BehaviorStatus::Failure => Self::Failure,
        }
    }
}

#[derive(Clone, Debug)]
pub enum BehaviorNode {
    Sequence(Vec<BehaviorNode>),
    Fallback(Vec<BehaviorNode>),
    Parallel {
        children: Vec<BehaviorNode>,
        success_threshold: usize,
    },
    Condition(Condition),
    Action(Action),
}

#[derive(Clone, Copy, Debug)]
pub enum Condition {
    MissionAuthorized,
    HasWaypoints,
    RelayTargetsConnected,
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    HoldPosition,
    FollowWaypoints,
    Loiter,
    ExtendMesh,
    CollectIsr,
}

pub struct BehaviorRuntime {
    root: BehaviorNode,
    waypoint_index: usize,
}

pub struct TickContext<'a> {
    pub entity: &'a mut Entity,
    pub mission: &'a MissionConfig,
    pub dt_s: f64,
    pub sim_time_s: f64,
    pub peer_positions: &'a BTreeMap<String, Position>,
    pub previous_links: &'a [LinkState],
    waypoint_index: &'a mut usize,
}

impl BehaviorRuntime {
    pub fn for_playbook(playbook: &str) -> Self {
        let authorized = || BehaviorNode::Condition(Condition::MissionAuthorized);
        let root = match playbook {
            "area_search" => BehaviorNode::Sequence(vec![
                authorized(),
                BehaviorNode::Condition(Condition::HasWaypoints),
                BehaviorNode::Action(Action::FollowWaypoints),
            ]),
            "persistent_surveillance" => BehaviorNode::Parallel {
                children: vec![
                    BehaviorNode::Sequence(vec![
                        authorized(),
                        BehaviorNode::Action(Action::Loiter),
                    ]),
                    BehaviorNode::Action(Action::CollectIsr),
                ],
                success_threshold: 2,
            },
            "comms_relay" => BehaviorNode::Fallback(vec![
                BehaviorNode::Sequence(vec![
                    BehaviorNode::Condition(Condition::RelayTargetsConnected),
                    BehaviorNode::Action(Action::HoldPosition),
                ]),
                BehaviorNode::Sequence(vec![
                    authorized(),
                    BehaviorNode::Action(Action::ExtendMesh),
                ]),
            ]),
            _ => BehaviorNode::Sequence(vec![
                authorized(),
                BehaviorNode::Action(Action::HoldPosition),
            ]),
        };
        Self {
            root,
            waypoint_index: 0,
        }
    }

    pub fn tick(
        &mut self,
        entity: &mut Entity,
        mission: &MissionConfig,
        dt_s: f64,
        sim_time_s: f64,
        peer_positions: &BTreeMap<String, Position>,
        previous_links: &[LinkState],
    ) -> BehaviorStatus {
        let (root, waypoint_index) = (&mut self.root, &mut self.waypoint_index);
        let mut context = TickContext {
            entity,
            mission,
            dt_s,
            sim_time_s,
            peer_positions,
            previous_links,
            waypoint_index,
        };
        let status = root.tick(&mut context);
        context.entity.mission.status = status.into();
        context.entity.mission_state = context.entity.mission.active_node.clone();
        status
    }
}

impl BehaviorNode {
    fn tick(&mut self, context: &mut TickContext<'_>) -> BehaviorStatus {
        match self {
            Self::Sequence(children) => {
                for child in children {
                    match child.tick(context) {
                        BehaviorStatus::Success => continue,
                        status => return status,
                    }
                }
                BehaviorStatus::Success
            }
            Self::Fallback(children) => {
                for child in children {
                    match child.tick(context) {
                        BehaviorStatus::Failure => continue,
                        status => return status,
                    }
                }
                BehaviorStatus::Failure
            }
            Self::Parallel {
                children,
                success_threshold,
            } => {
                let mut successes = 0;
                let mut failures = 0;
                for child in children.iter_mut() {
                    match child.tick(context) {
                        BehaviorStatus::Success => successes += 1,
                        BehaviorStatus::Failure => failures += 1,
                        BehaviorStatus::Running => {}
                    }
                }
                if successes >= *success_threshold {
                    BehaviorStatus::Success
                } else if failures > children.len().saturating_sub(*success_threshold) {
                    BehaviorStatus::Failure
                } else {
                    BehaviorStatus::Running
                }
            }
            Self::Condition(condition) => condition.evaluate(context),
            Self::Action(action) => action.tick(context),
        }
    }
}

impl Condition {
    fn evaluate(self, context: &TickContext<'_>) -> BehaviorStatus {
        let passed = match self {
            Self::MissionAuthorized => context.mission.human_authorized,
            Self::HasWaypoints => !context.mission.waypoints.is_empty(),
            Self::RelayTargetsConnected => {
                if context.mission.target_entities.len() < 2 {
                    false
                } else {
                    let left = &context.mission.target_entities[0];
                    let right = &context.mission.target_entities[1];
                    context.previous_links.iter().any(|link| {
                        link.link_type == LinkType::Mesh
                            && link.state == LinkStatus::Up
                            && ((link.source == *left && link.target == *right)
                                || (link.source == *right && link.target == *left))
                    })
                }
            }
        };
        if passed {
            BehaviorStatus::Success
        } else {
            BehaviorStatus::Failure
        }
    }
}

impl Action {
    fn tick(self, context: &mut TickContext<'_>) -> BehaviorStatus {
        match self {
            Self::HoldPosition => {
                context.entity.kinematics.speed_mps = 0.0;
                context.entity.mission.active_node = "hold_position".into();
                BehaviorStatus::Running
            }
            Self::FollowWaypoints => {
                let target = context.mission.waypoints[*context.waypoint_index];
                move_toward(
                    context.entity,
                    target,
                    context.mission.speed_mps,
                    context.dt_s,
                );
                context.entity.mission.active_node =
                    format!("coverage_waypoint_{}", *context.waypoint_index + 1);
                if context.entity.position.distance_to(target) < 1.0 {
                    *context.waypoint_index =
                        (*context.waypoint_index + 1) % context.mission.waypoints.len();
                }
                BehaviorStatus::Running
            }
            Self::Loiter => {
                let Some(center) = context.mission.center else {
                    return BehaviorStatus::Failure;
                };
                let orbit_target = point_on_orbit(
                    center,
                    context.mission.radius_m,
                    context.sim_time_s,
                    context.mission.speed_mps,
                );
                move_toward(
                    context.entity,
                    orbit_target,
                    context.mission.speed_mps,
                    context.dt_s,
                );
                context.entity.mission.active_node = "isr_loiter".into();
                BehaviorStatus::Running
            }
            Self::ExtendMesh => {
                let positions: Vec<_> = context
                    .mission
                    .target_entities
                    .iter()
                    .filter_map(|id| context.peer_positions.get(id).copied())
                    .take(2)
                    .collect();
                if positions.len() != 2 {
                    return BehaviorStatus::Failure;
                }
                let midpoint = Position {
                    lat_deg: (positions[0].lat_deg + positions[1].lat_deg) / 2.0,
                    lon_deg: (positions[0].lon_deg + positions[1].lon_deg) / 2.0,
                    alt_m: context.entity.position.alt_m,
                };
                move_toward(
                    context.entity,
                    midpoint,
                    context.mission.speed_mps,
                    context.dt_s,
                );
                context.entity.mission.active_node = "mesh_extension".into();
                BehaviorStatus::Running
            }
            Self::CollectIsr => BehaviorStatus::Success,
        }
    }
}

fn move_toward(entity: &mut Entity, target: Position, speed_mps: f64, dt_s: f64) {
    let before = entity.position;
    entity.kinematics.heading_deg = before.bearing_to(target);
    entity.heading_deg = entity.kinematics.heading_deg;
    entity.kinematics.speed_mps = speed_mps;
    entity.kinematics.vertical_speed_mps = if dt_s > 0.0 {
        (target.alt_m - before.alt_m).clamp(-speed_mps * dt_s, speed_mps * dt_s) / dt_s
    } else {
        0.0
    };
    entity.position = before.moved_toward(target, speed_mps * dt_s);
}

fn point_on_orbit(center: Position, radius_m: f64, sim_time_s: f64, speed_mps: f64) -> Position {
    const METERS_PER_DEGREE: f64 = 111_320.0;
    let angular_speed = speed_mps / radius_m.max(1.0);
    let angle = sim_time_s * angular_speed;
    let lon_scale = (METERS_PER_DEGREE * center.lat_deg.to_radians().cos()).max(1.0);
    Position {
        lat_deg: center.lat_deg + angle.sin() * radius_m / METERS_PER_DEGREE,
        lon_deg: center.lon_deg + angle.cos() * radius_m / lon_scale,
        alt_m: center.alt_m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Affiliation, Domain, EntityKind, Kinematics, MissionState},
        symbology::{SymbolStatus, icon_hint, sidc},
    };

    fn entity() -> Entity {
        Entity {
            id: "test".into(),
            name: "Test".into(),
            kind: EntityKind::Uas,
            affiliation: Affiliation::Friendly,
            sidc: sidc(
                EntityKind::Uas,
                Affiliation::Friendly,
                SymbolStatus::Present,
            ),
            icon_hint: icon_hint(EntityKind::Uas).into(),
            domain: Domain::Air,
            position: Position::default(),
            kinematics: Kinematics::default(),
            mission: MissionState::default(),
            mission_role: "scout".into(),
            mission_state: "holding".into(),
            heading_deg: 0.0,
            retardant_pct: None,
            intensity: None,
            radios: Vec::new(),
        }
    }

    #[test]
    fn hard_authorization_constraint_blocks_motion() {
        let mut runtime = BehaviorRuntime::for_playbook("area_search");
        let mut entity = entity();
        let mission = MissionConfig {
            playbook: "area_search".into(),
            speed_mps: 10.0,
            waypoints: vec![Position {
                lat_deg: 0.01,
                ..Position::default()
            }],
            human_authorized: false,
            ..MissionConfig::default()
        };
        let status = runtime.tick(&mut entity, &mission, 1.0, 1.0, &BTreeMap::new(), &[]);
        assert_eq!(status, BehaviorStatus::Failure);
        assert_eq!(entity.position, Position::default());
    }

    #[test]
    fn fallback_moves_relay_when_targets_are_disconnected() {
        let mut runtime = BehaviorRuntime::for_playbook("comms_relay");
        let mut entity = entity();
        let mission = MissionConfig {
            playbook: "comms_relay".into(),
            speed_mps: 10.0,
            target_entities: vec!["left".into(), "right".into()],
            ..MissionConfig::default()
        };
        let positions = BTreeMap::from([
            (
                "left".into(),
                Position {
                    lon_deg: -0.01,
                    ..Position::default()
                },
            ),
            (
                "right".into(),
                Position {
                    lat_deg: 0.01,
                    lon_deg: 0.01,
                    ..Position::default()
                },
            ),
        ]);
        assert_eq!(
            runtime.tick(&mut entity, &mission, 1.0, 1.0, &positions, &[]),
            BehaviorStatus::Running
        );
        assert_eq!(entity.mission.active_node, "mesh_extension");
        assert!(entity.position.lat_deg > 0.0);
    }
}
