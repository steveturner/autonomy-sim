use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::model::{Domain, EntityKind, Position, Radio};

#[derive(Clone, Debug, Deserialize)]
pub struct ScenarioConfig {
    pub scenario: ScenarioMetadata,
    pub simulation: SimulationConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub cot: CotConfig,
    pub nodes: Vec<NodeConfig>,
}

impl ScenarioConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading scenario {}", path.display()))?;
        let config: Self = toml::from_str(&source)
            .with_context(|| format!("parsing scenario {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.scenario.name.trim().is_empty() {
            bail!("scenario.name must not be empty");
        }
        if self.simulation.tick_hz <= 0.0 || !self.simulation.tick_hz.is_finite() {
            bail!("simulation.tick_hz must be finite and greater than zero");
        }
        if self.nodes.is_empty() {
            bail!("scenario must contain at least one node");
        }
        if !matches!(
            self.simulation.network_backend.as_str(),
            "analytic" | "sigforge"
        ) {
            bail!("simulation.network_backend must be 'analytic' or 'sigforge'");
        }
        let mut ids = std::collections::BTreeSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id.clone()) {
                bail!("duplicate node id '{}'", node.id);
            }
            if node.id.is_empty()
                || !node
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                bail!(
                    "node id '{}' must contain only letters, digits, '-' or '_'",
                    node.id
                );
            }
            match node.mission.playbook.as_str() {
                "hold" => {}
                "area_search" if node.mission.waypoints.is_empty() => {
                    bail!("node '{}' area_search requires mission.waypoints", node.id)
                }
                "persistent_surveillance" if node.mission.center.is_none() => {
                    bail!(
                        "node '{}' persistent_surveillance requires mission.center",
                        node.id
                    )
                }
                "comms_relay" if node.mission.target_entities.len() < 2 => {
                    bail!(
                        "node '{}' comms_relay requires two mission.target_entities",
                        node.id
                    )
                }
                "area_search" | "persistent_surveillance" | "comms_relay" => {}
                other => bail!("node '{}' has unknown mission playbook '{other}'", node.id),
            }
            for radio in &node.radios {
                if radio.range_m <= 0.0 || radio.capacity_bps == 0 || radio.base_latency_ms < 0.0 {
                    bail!("node '{}' has invalid radio parameters", node.id);
                }
            }
        }
        for node in &self.nodes {
            for target in &node.mission.target_entities {
                if !ids.contains(target) {
                    bail!(
                        "node '{}' mission references unknown entity '{target}'",
                        node.id
                    );
                }
            }
        }
        if self.cot.interval_s <= 0.0 || !self.cot.interval_s.is_finite() {
            bail!("cot.interval_s must be finite and greater than zero");
        }
        if self.cot.stale_after_s <= 0 {
            bail!("cot.stale_after_s must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScenarioMetadata {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_true")]
    pub realtime: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SimulationConfig {
    #[serde(default = "default_tick_hz")]
    pub tick_hz: f64,
    #[serde(default = "default_backend")]
    pub network_backend: String,
    #[serde(default = "default_sigforge_url")]
    pub sigforge_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct CotConfig {
    #[serde(default = "default_cot_sink")]
    pub sink: String,
    #[serde(default = "default_cot_path")]
    pub path: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default = "default_cot_interval")]
    pub interval_s: f64,
    #[serde(default = "default_cot_stale")]
    pub stale_after_s: i64,
}

impl Default for CotConfig {
    fn default() -> Self {
        Self {
            sink: default_cot_sink(),
            path: default_cot_path(),
            endpoint: String::new(),
            interval_s: default_cot_interval(),
            stale_after_s: default_cot_stale(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct NodeConfig {
    pub id: String,
    pub name: String,
    pub kind: EntityKind,
    pub domain: Domain,
    pub position: Position,
    #[serde(default)]
    pub radios: Vec<Radio>,
    #[serde(default)]
    pub mission: MissionConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MissionConfig {
    #[serde(default = "default_playbook")]
    pub playbook: String,
    #[serde(default)]
    pub speed_mps: f64,
    #[serde(default)]
    pub waypoints: Vec<Position>,
    #[serde(default)]
    pub center: Option<Position>,
    #[serde(default = "default_loiter_radius")]
    pub radius_m: f64,
    #[serde(default)]
    pub target_entities: Vec<String>,
    #[serde(default = "default_true")]
    pub human_authorized: bool,
}

impl Default for MissionConfig {
    fn default() -> Self {
        Self {
            playbook: default_playbook(),
            speed_mps: 0.0,
            waypoints: Vec::new(),
            center: None,
            radius_m: default_loiter_radius(),
            target_entities: Vec::new(),
            human_authorized: true,
        }
    }
}

fn default_seed() -> u64 {
    42
}
fn default_true() -> bool {
    true
}
fn default_tick_hz() -> f64 {
    5.0
}
fn default_backend() -> String {
    "analytic".into()
}
fn default_sigforge_url() -> String {
    "http://127.0.0.1:9000".into()
}
fn default_bind() -> String {
    "127.0.0.1:9000".into()
}
fn default_playbook() -> String {
    "hold".into()
}
fn default_loiter_radius() -> f64 {
    250.0
}
fn default_cot_sink() -> String {
    "disabled".into()
}
fn default_cot_path() -> String {
    "output/autonomy-sim.cot".into()
}
fn default_cot_interval() -> f64 {
    1.0
}
fn default_cot_stale() -> i64 {
    10
}
