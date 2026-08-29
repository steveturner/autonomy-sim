use crate::{
    model::{Entity, Kinematics, MissionState, MissionStatus},
    scenario::ScenarioConfig,
    wire::{SCHEMA, StateEnvelope, StatePayload, entity_czml},
};

#[derive(Clone, Debug)]
struct Agent {
    entity: Entity,
    speed_mps: f64,
    waypoints: Vec<crate::model::Position>,
    next_waypoint: usize,
}

pub struct Simulation {
    scenario_name: String,
    tick_hz: f64,
    sequence: u64,
    sim_time_s: f64,
    agents: Vec<Agent>,
}

impl Simulation {
    pub fn new(config: &ScenarioConfig) -> Self {
        let agents = config
            .nodes
            .iter()
            .map(|node| Agent {
                entity: Entity {
                    id: node.id.clone(),
                    name: node.name.clone(),
                    kind: node.kind,
                    domain: node.domain,
                    position: node.position,
                    kinematics: Kinematics {
                        speed_mps: node.mission.speed_mps,
                        ..Kinematics::default()
                    },
                    mission: MissionState {
                        playbook: node.mission.playbook.clone(),
                        active_node: if node.mission.waypoints.is_empty() {
                            "hold_position"
                        } else {
                            "follow_waypoints"
                        }
                        .into(),
                        status: MissionStatus::Running,
                    },
                },
                speed_mps: node.mission.speed_mps,
                waypoints: node.mission.waypoints.clone(),
                next_waypoint: 0,
            })
            .collect();
        Self {
            scenario_name: config.scenario.name.clone(),
            tick_hz: config.simulation.tick_hz,
            sequence: 0,
            sim_time_s: 0.0,
            agents,
        }
    }

    pub fn scenario_name(&self) -> &str {
        &self.scenario_name
    }
    pub fn tick_hz(&self) -> f64 {
        self.tick_hz
    }

    pub fn snapshot(&self) -> StateEnvelope {
        self.frame(Vec::new())
    }

    pub fn tick(&mut self) -> StateEnvelope {
        let dt_s = 1.0 / self.tick_hz;
        for agent in &mut self.agents {
            if agent.waypoints.is_empty() || agent.speed_mps <= 0.0 {
                continue;
            }
            let target = agent.waypoints[agent.next_waypoint];
            let before = agent.entity.position;
            agent.entity.kinematics.heading_deg = before.bearing_to(target);
            agent.entity.position = before.moved_toward(target, agent.speed_mps * dt_s);
            if agent.entity.position.distance_to(target) < 1.0 {
                agent.next_waypoint = (agent.next_waypoint + 1) % agent.waypoints.len();
            }
        }
        self.sequence += 1;
        self.sim_time_s += dt_s;
        self.frame(Vec::new())
    }

    fn frame(&self, link_events: Vec<serde_json::Value>) -> StateEnvelope {
        let entities: Vec<_> = self
            .agents
            .iter()
            .map(|agent| agent.entity.clone())
            .collect();
        let czml = entities.iter().map(entity_czml).collect();
        StateEnvelope {
            schema: SCHEMA,
            message_type: "state",
            sequence: self.sequence,
            sim_time_s: self.sim_time_s,
            payload: StatePayload {
                entities,
                links: Vec::new(),
                link_events,
                traffic: Vec::new(),
                czml,
            },
        }
    }
}
