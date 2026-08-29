use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::model::{Domain, EntityKind, Position};

#[derive(Clone, Debug, Deserialize)]
pub struct ScenarioConfig {
    pub scenario: ScenarioMetadata,
    pub simulation: SimulationConfig,
    #[serde(default)]
    pub api: ApiConfig,
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
        let mut ids = std::collections::BTreeSet::new();
        for node in &self.nodes {
            if !ids.insert(&node.id) {
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
pub struct NodeConfig {
    pub id: String,
    pub name: String,
    pub kind: EntityKind,
    pub domain: Domain,
    pub position: Position,
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
}

impl Default for MissionConfig {
    fn default() -> Self {
        Self {
            playbook: default_playbook(),
            speed_mps: 0.0,
            waypoints: Vec::new(),
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
fn default_bind() -> String {
    "127.0.0.1:9000".into()
}
fn default_playbook() -> String {
    "hold".into()
}
