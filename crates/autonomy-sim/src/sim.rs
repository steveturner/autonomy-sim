use std::collections::BTreeMap;

use anyhow::Result;
use chrono::Utc;

use crate::{
    behavior::BehaviorRuntime,
    cot::{CotSink, render_pli, sink_from_config},
    model::{Entity, Kinematics, MissionState, MissionStatus, Position},
    network::{
        AnalyticNetworkBackend, LinkState, NetworkBackend, SigForgeBackend, derive_link_events,
        synthetic_traffic,
    },
    scenario::{MissionConfig, ScenarioConfig},
    wire::{SCHEMA, StateEnvelope, StatePayload, entity_czml, link_czml},
};

struct Agent {
    entity: Entity,
    mission: MissionConfig,
    behavior: BehaviorRuntime,
}

pub struct Simulation {
    scenario_name: String,
    tick_hz: f64,
    sequence: u64,
    sim_time_s: f64,
    agents: Vec<Agent>,
    network: Box<dyn NetworkBackend>,
    previous_links: Vec<LinkState>,
    cot_sink: Box<dyn CotSink>,
    cot_interval_ticks: u64,
    cot_stale_after_s: i64,
}

impl Simulation {
    pub fn try_new(config: &ScenarioConfig) -> Result<Self> {
        let agents: Vec<_> = config
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
                        active_node: "initialize".into(),
                        status: MissionStatus::Running,
                    },
                    radios: node.radios.clone(),
                },
                mission: node.mission.clone(),
                behavior: BehaviorRuntime::for_playbook(&node.mission.playbook),
            })
            .collect();
        let mut network: Box<dyn NetworkBackend> = match config.simulation.network_backend.as_str()
        {
            "sigforge" => Box::new(SigForgeBackend::new(&config.simulation.sigforge_url)),
            _ => Box::new(AnalyticNetworkBackend::default()),
        };
        let entities: Vec<_> = agents.iter().map(|agent| agent.entity.clone()).collect();
        network.register_nodes(&entities)?;
        tracing::info!(backend = network.name(), "network backend initialized");

        Ok(Self {
            scenario_name: config.scenario.name.clone(),
            tick_hz: config.simulation.tick_hz,
            sequence: 0,
            sim_time_s: 0.0,
            agents,
            network,
            previous_links: Vec::new(),
            cot_sink: sink_from_config(&config.cot)?,
            cot_interval_ticks: (config.cot.interval_s * config.simulation.tick_hz)
                .round()
                .max(1.0) as u64,
            cot_stale_after_s: config.cot.stale_after_s,
        })
    }

    pub fn scenario_name(&self) -> &str {
        &self.scenario_name
    }

    pub fn tick_hz(&self) -> f64 {
        self.tick_hz
    }

    pub fn snapshot(&mut self) -> Result<StateEnvelope> {
        self.evaluate_frame(false)
    }

    pub fn tick(&mut self) -> Result<StateEnvelope> {
        let dt_s = 1.0 / self.tick_hz;
        let positions: BTreeMap<String, Position> = self
            .agents
            .iter()
            .map(|agent| (agent.entity.id.clone(), agent.entity.position))
            .collect();
        for agent in &mut self.agents {
            agent.behavior.tick(
                &mut agent.entity,
                &agent.mission,
                dt_s,
                self.sim_time_s,
                &positions,
                &self.previous_links,
            );
        }
        self.sequence += 1;
        self.sim_time_s += dt_s;
        self.evaluate_frame(true)
    }

    fn evaluate_frame(&mut self, emit_cot: bool) -> Result<StateEnvelope> {
        let entities: Vec<_> = self
            .agents
            .iter()
            .map(|agent| agent.entity.clone())
            .collect();
        let links = self.network.link_states(self.sim_time_s, &entities)?;
        let previous: BTreeMap<_, _> = self
            .previous_links
            .iter()
            .map(|link| (link.id.clone(), link.state))
            .collect();
        let link_events = derive_link_events(&previous, &links, self.sim_time_s);
        let traffic = synthetic_traffic(&links, self.sequence);
        self.previous_links = links.clone();

        if emit_cot && self.sequence.is_multiple_of(self.cot_interval_ticks) {
            let now = Utc::now();
            for entity in &entities {
                let xml = render_pli(entity, now, self.cot_stale_after_s);
                if let Err(error) = self.cot_sink.emit(&xml) {
                    tracing::warn!(%error, entity = %entity.id, "CoT sink write failed");
                }
            }
        }

        Ok(self.frame(entities, links, link_events, traffic))
    }

    fn frame(
        &self,
        entities: Vec<Entity>,
        links: Vec<LinkState>,
        link_events: Vec<crate::network::LinkEvent>,
        traffic: Vec<crate::network::TrafficState>,
    ) -> StateEnvelope {
        let positions: BTreeMap<_, _> = entities
            .iter()
            .map(|entity| (entity.id.clone(), entity.position))
            .collect();
        let mut czml: Vec<_> = entities.iter().map(entity_czml).collect();
        czml.extend(links.iter().filter_map(|link| link_czml(link, &positions)));
        StateEnvelope {
            schema: SCHEMA,
            message_type: "state",
            sequence: self.sequence,
            sim_time_s: self.sim_time_s,
            payload: StatePayload {
                entities,
                links,
                link_events,
                traffic,
                czml,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::LinkStatus;
    use crate::{
        model::{Domain, EntityKind, LinkType, Radio},
        scenario::{ApiConfig, CotConfig, NodeConfig, ScenarioMetadata, SimulationConfig},
    };

    fn config() -> ScenarioConfig {
        let radio = Radio {
            link_type: LinkType::Mesh,
            range_m: 500.0,
            capacity_bps: 1_000_000,
            base_latency_ms: 5.0,
        };
        ScenarioConfig {
            scenario: ScenarioMetadata {
                name: "test".into(),
                description: String::new(),
                seed: 42,
                realtime: true,
            },
            simulation: SimulationConfig {
                tick_hz: 5.0,
                network_backend: "analytic".into(),
                sigforge_url: String::new(),
            },
            api: ApiConfig::default(),
            cot: CotConfig::default(),
            nodes: vec![
                NodeConfig {
                    id: "a".into(),
                    name: "A".into(),
                    kind: EntityKind::Drone,
                    domain: Domain::Air,
                    position: Position::default(),
                    radios: vec![radio.clone()],
                    mission: MissionConfig::default(),
                },
                NodeConfig {
                    id: "b".into(),
                    name: "B".into(),
                    kind: EntityKind::GroundStation,
                    domain: Domain::Ground,
                    position: Position {
                        lon_deg: 0.001,
                        ..Position::default()
                    },
                    radios: vec![radio],
                    mission: MissionConfig::default(),
                },
            ],
        }
    }

    #[test]
    fn frame_contains_typed_link_traffic_and_czml() {
        let mut simulation = Simulation::try_new(&config()).unwrap();
        let frame = simulation.snapshot().unwrap();
        assert_eq!(frame.payload.links.len(), 1);
        assert_eq!(frame.payload.links[0].state, LinkStatus::Up);
        assert_eq!(frame.payload.traffic.len(), 1);
        assert_eq!(frame.payload.czml.len(), 3);
        assert_eq!(frame.payload.link_events.len(), 1);
    }
}
