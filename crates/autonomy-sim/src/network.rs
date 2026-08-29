use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{Entity, LinkType, Position, Radio};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Up,
    Down,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LinkState {
    pub id: String,
    pub source: String,
    pub target: String,
    pub link_type: LinkType,
    pub state: LinkStatus,
    pub quality: f64,
    pub distance_m: f64,
    pub latency_ms: f64,
    pub packet_loss: f64,
    pub capacity_bps: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LinkEvent {
    pub link_id: String,
    pub source: String,
    pub target: String,
    pub link_type: LinkType,
    pub state: LinkStatus,
    pub changed_at_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrafficState {
    pub link_id: String,
    pub tx_bps: u64,
    pub rx_bps: u64,
    pub messages_per_s: f64,
    pub queue_depth: u32,
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("network backend is unavailable: {0}")]
    Unavailable(String),
    #[error("network backend rejected entity '{0}'")]
    UnknownEntity(String),
}

/// Pluggable network-emulation boundary. Implementations receive authoritative
/// platform state and return one current-state record per compatible pair and
/// radio type.
pub trait NetworkBackend: Send {
    fn name(&self) -> &'static str;
    fn register_nodes(&mut self, entities: &[Entity]) -> Result<(), NetworkError>;
    fn link_states(
        &mut self,
        sim_time_s: f64,
        entities: &[Entity],
    ) -> Result<Vec<LinkState>, NetworkError>;
}

#[derive(Clone, Copy, Debug)]
pub struct PropagationInput {
    pub source: Position,
    pub target: Position,
    pub max_range_m: f64,
    pub link_type: LinkType,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropagationResult {
    pub distance_m: f64,
    pub quality: f64,
}

/// Environment-specific signal propagation seam. It intentionally has no
/// knowledge of missions, transport protocols, or visualization.
pub trait PropagationModel: Send + Sync {
    fn evaluate(&self, input: PropagationInput) -> PropagationResult;
}

#[derive(Clone, Copy, Debug)]
pub struct OutdoorAnalyticPropagation {
    pub decay_exponent: f64,
}

impl Default for OutdoorAnalyticPropagation {
    fn default() -> Self {
        Self {
            decay_exponent: 2.2,
        }
    }
}

impl PropagationModel for OutdoorAnalyticPropagation {
    fn evaluate(&self, input: PropagationInput) -> PropagationResult {
        let distance_m = input.source.distance_to(input.target);
        let normalized = distance_m / input.max_range_m.max(1.0);
        let transport_factor: f64 = match input.link_type {
            LinkType::Ble => 1.1,
            LinkType::Cellular => 0.9,
            LinkType::Mesh => 1.0,
            LinkType::Satcom => 0.75,
        };
        let quality =
            (1.0 - normalized.powf(self.decay_exponent * transport_factor)).clamp(0.0, 1.0);
        PropagationResult {
            distance_m,
            quality,
        }
    }
}

pub struct AnalyticNetworkBackend {
    propagation: Box<dyn PropagationModel>,
    registered: BTreeSet<String>,
}

impl Default for AnalyticNetworkBackend {
    fn default() -> Self {
        Self::new(Box::new(OutdoorAnalyticPropagation::default()))
    }
}

impl AnalyticNetworkBackend {
    pub fn new(propagation: Box<dyn PropagationModel>) -> Self {
        Self {
            propagation,
            registered: BTreeSet::new(),
        }
    }

    fn evaluate_radio_pair(
        &self,
        source: &Entity,
        target: &Entity,
        source_radio: &Radio,
        target_radio: &Radio,
    ) -> LinkState {
        let max_range_m = source_radio.range_m.min(target_radio.range_m);
        let result = self.propagation.evaluate(PropagationInput {
            source: source.position,
            target: target.position,
            max_range_m,
            link_type: source_radio.link_type,
        });
        let quality = result.quality;
        let up = quality >= 0.05;
        let base_capacity = source_radio.capacity_bps.min(target_radio.capacity_bps);
        let capacity_bps = if up {
            (base_capacity as f64 * quality.powf(1.35)).round() as u64
        } else {
            0
        };
        let packet_loss = if up {
            (1.0 - quality).powi(2).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let base_latency = source_radio
            .base_latency_ms
            .max(target_radio.base_latency_ms);
        let propagation_ms = if source_radio.link_type == LinkType::Satcom {
            24.0
        } else {
            result.distance_m / 299_792.458
        };
        let latency_ms = base_latency + propagation_ms + (1.0 - quality) * 20.0;
        let (a, b) = sorted_endpoints(&source.id, &target.id);
        LinkState {
            id: format!("link/{}/{a}/{b}", source_radio.link_type),
            source: a.to_owned(),
            target: b.to_owned(),
            link_type: source_radio.link_type,
            state: if up { LinkStatus::Up } else { LinkStatus::Down },
            quality,
            distance_m: result.distance_m,
            latency_ms,
            packet_loss,
            capacity_bps,
        }
    }
}

impl NetworkBackend for AnalyticNetworkBackend {
    fn name(&self) -> &'static str {
        "analytic"
    }

    fn register_nodes(&mut self, entities: &[Entity]) -> Result<(), NetworkError> {
        self.registered = entities.iter().map(|entity| entity.id.clone()).collect();
        Ok(())
    }

    fn link_states(
        &mut self,
        _sim_time_s: f64,
        entities: &[Entity],
    ) -> Result<Vec<LinkState>, NetworkError> {
        for entity in entities {
            if !self.registered.contains(&entity.id) {
                return Err(NetworkError::UnknownEntity(entity.id.clone()));
            }
        }
        let mut links = Vec::new();
        for (index, source) in entities.iter().enumerate() {
            for target in &entities[(index + 1)..] {
                let target_radios: BTreeMap<_, _> = target
                    .radios
                    .iter()
                    .map(|radio| (radio.link_type, radio))
                    .collect();
                for source_radio in &source.radios {
                    if let Some(target_radio) = target_radios.get(&source_radio.link_type) {
                        links.push(self.evaluate_radio_pair(
                            source,
                            target,
                            source_radio,
                            target_radio,
                        ));
                    }
                }
            }
        }
        links.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(links)
    }
}

/// Phase 1 proof of the SigForge seam.
///
/// A Phase 2 implementation will POST node registrations to SigForge's
/// `/api/v1/session`, publish WGS84 location updates, and consume its `/sim`
/// WebSocket link matrix. This stub fails closed rather than silently
/// substituting analytic results.
pub struct SigForgeBackend {
    pub base_url: String,
    entity_to_nem: BTreeMap<String, u16>,
}

impl SigForgeBackend {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            entity_to_nem: BTreeMap::new(),
        }
    }
}

impl NetworkBackend for SigForgeBackend {
    fn name(&self) -> &'static str {
        "sigforge-stub"
    }

    fn register_nodes(&mut self, entities: &[Entity]) -> Result<(), NetworkError> {
        self.entity_to_nem = entities
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.id.clone(), (index + 1) as u16))
            .collect();
        Ok(())
    }

    fn link_states(
        &mut self,
        _sim_time_s: f64,
        _entities: &[Entity],
    ) -> Result<Vec<LinkState>, NetworkError> {
        Err(NetworkError::Unavailable(format!(
            "SigForge adapter at {} is a Phase 2 stub; use network_backend = 'analytic'",
            self.base_url
        )))
    }
}

pub fn derive_link_events(
    previous: &BTreeMap<String, LinkStatus>,
    current: &[LinkState],
    sim_time_s: f64,
) -> Vec<LinkEvent> {
    current
        .iter()
        .filter_map(|link| {
            let old = previous.get(&link.id).copied().unwrap_or(LinkStatus::Down);
            (old != link.state).then(|| LinkEvent {
                link_id: link.id.clone(),
                source: link.source.clone(),
                target: link.target.clone(),
                link_type: link.link_type,
                state: link.state,
                changed_at_s: sim_time_s,
            })
        })
        .collect()
}

pub fn synthetic_traffic(links: &[LinkState], sequence: u64) -> Vec<TrafficState> {
    links
        .iter()
        .filter(|link| link.state == LinkStatus::Up)
        .map(|link| {
            let phase = (stable_hash(&link.id) % 360) as f64 + sequence as f64 * 7.0;
            let utilization = 0.05 + 0.045 * (phase.to_radians().sin() + 1.0);
            let tx_bps = (link.capacity_bps as f64 * utilization).round() as u64;
            let rx_bps = (tx_bps as f64 * (1.0 - link.packet_loss)).round() as u64;
            TrafficState {
                link_id: link.id.clone(),
                tx_bps,
                rx_bps,
                messages_per_s: tx_bps as f64 / 8.0 / 1024.0,
                queue_depth: ((1.0 - link.quality) * 12.0).round() as u32,
            }
        })
        .collect()
}

fn sorted_endpoints<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn stable_hash(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Domain, EntityKind, Kinematics, MissionState};

    fn entity(id: &str, lon: f64) -> Entity {
        Entity {
            id: id.into(),
            name: id.into(),
            kind: EntityKind::Drone,
            domain: Domain::Air,
            position: Position {
                lat_deg: 34.0,
                lon_deg: lon,
                alt_m: 100.0,
            },
            kinematics: Kinematics::default(),
            mission: MissionState::default(),
            radios: vec![Radio {
                link_type: LinkType::Mesh,
                range_m: 1_000.0,
                capacity_bps: 1_000_000,
                base_latency_ms: 5.0,
            }],
        }
    }

    #[test]
    fn analytic_backend_transitions_at_range_boundary() {
        let mut backend = AnalyticNetworkBackend::default();
        let mut entities = vec![entity("bravo", -117.0), entity("alpha", -117.001)];
        backend.register_nodes(&entities).unwrap();
        let near = backend.link_states(0.0, &entities).unwrap();
        assert_eq!(near[0].id, "link/mesh/alpha/bravo");
        assert_eq!(near[0].state, LinkStatus::Up);

        entities[1].position.lon_deg = -117.02;
        let far = backend.link_states(1.0, &entities).unwrap();
        assert_eq!(far[0].state, LinkStatus::Down);
        assert_eq!(far[0].packet_loss, 1.0);
    }

    #[test]
    fn events_only_include_state_changes() {
        let link = LinkState {
            id: "link/mesh/a/b".into(),
            source: "a".into(),
            target: "b".into(),
            link_type: LinkType::Mesh,
            state: LinkStatus::Up,
            quality: 1.0,
            distance_m: 1.0,
            latency_ms: 1.0,
            packet_loss: 0.0,
            capacity_bps: 1,
        };
        let initial = derive_link_events(&BTreeMap::new(), std::slice::from_ref(&link), 0.2);
        assert_eq!(initial.len(), 1);
        let previous = BTreeMap::from([(link.id.clone(), LinkStatus::Up)]);
        assert!(derive_link_events(&previous, &[link], 0.4).is_empty());
    }
}
