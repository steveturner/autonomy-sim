use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    model::{Affiliation, Domain, EntityKind, Position, Radio},
    swarm::FlockingConfig,
};

const REGISTERED_SCENARIOS: &[(&str, &str, &str, ScenarioBuilder, bool)] = &[
    (
        "isr-relay-demo",
        "ISR Relay Demo",
        "isr-demo.toml",
        ScenarioBuilder::Standard,
        true,
    ),
    (
        "wildfire-paradise",
        "Wildfire - Paradise",
        "wildfire-paradise.toml",
        ScenarioBuilder::Wildfire,
        false,
    ),
    (
        "cuas-stadium",
        "C-UAS Stadium Defense",
        "cuas-stadium.toml",
        ScenarioBuilder::Cuas,
        false,
    ),
];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioBuilder {
    #[default]
    Standard,
    Wildfire,
    Cuas,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScenarioDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entity_count: usize,
    pub default: bool,
}

#[derive(Clone, Debug)]
pub struct ScenarioRegistry {
    directory: PathBuf,
}

impl Default for ScenarioRegistry {
    fn default() -> Self {
        Self::new("scenarios")
    }
}

impl ScenarioRegistry {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn load(&self, selector: &str) -> Result<ScenarioConfig> {
        let direct = Path::new(selector);
        let (path, registered_builder) = if direct.exists() || selector.ends_with(".toml") {
            (direct.to_path_buf(), None)
        } else {
            let (_, _, file, builder, _) = REGISTERED_SCENARIOS
                .iter()
                .find(|(id, _, _, _, _)| *id == selector)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown scenario '{selector}'; available: {}",
                        REGISTERED_SCENARIOS
                            .iter()
                            .map(|(id, _, _, _, _)| *id)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            (self.directory.join(file), Some(*builder))
        };
        let config = ScenarioConfig::from_path(&path)?;
        if let Some(expected) = registered_builder
            && config.scenario.builder != expected
        {
            bail!(
                "scenario '{}' is registered with builder '{expected:?}' but TOML selects '{:?}'",
                config.scenario.name,
                config.scenario.builder
            );
        }
        Ok(config)
    }

    pub fn descriptors(&self) -> Result<Vec<ScenarioDescriptor>> {
        REGISTERED_SCENARIOS
            .iter()
            .map(|(registered_id, display_name, file, builder, is_default)| {
                let config = ScenarioConfig::from_path(self.directory.join(file))?;
                if config.scenario.name != *registered_id {
                    bail!(
                        "registered scenario '{registered_id}' has TOML name '{}'",
                        config.scenario.name
                    );
                }
                if config.scenario.builder != *builder {
                    bail!("registered scenario '{registered_id}' has mismatched builder");
                }
                Ok(ScenarioDescriptor {
                    id: config.scenario.name,
                    name: (*display_name).into(),
                    description: config.scenario.description,
                    entity_count: config.nodes.len(),
                    default: *is_default,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScenarioConfig {
    pub scenario: ScenarioMetadata,
    pub simulation: SimulationConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub cot: CotConfig,
    #[serde(default)]
    pub wildfire: Option<WildfireConfig>,
    #[serde(default)]
    pub cuas: Option<CuasConfig>,
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
                "firefighting" if self.scenario.builder != ScenarioBuilder::Wildfire => {
                    bail!(
                        "node '{}' firefighting playbook requires wildfire builder",
                        node.id
                    )
                }
                "cuas_threat" if self.scenario.builder != ScenarioBuilder::Cuas => {
                    bail!(
                        "node '{}' cuas_threat playbook requires C-UAS builder",
                        node.id
                    )
                }
                "area_search"
                | "persistent_surveillance"
                | "comms_relay"
                | "firefighting"
                | "cuas_threat" => {}
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
        match (self.scenario.builder, &self.wildfire) {
            (ScenarioBuilder::Wildfire, Some(wildfire)) => wildfire.validate(self, &ids)?,
            (ScenarioBuilder::Wildfire, None) => {
                bail!("wildfire builder requires a [wildfire] section")
            }
            (ScenarioBuilder::Standard | ScenarioBuilder::Cuas, Some(_)) => {
                bail!("[wildfire] section requires scenario.builder = 'wildfire'")
            }
            (ScenarioBuilder::Standard | ScenarioBuilder::Cuas, None) => {}
        }
        match (self.scenario.builder, &self.cuas) {
            (ScenarioBuilder::Cuas, Some(cuas)) => cuas.validate(self, &ids)?,
            (ScenarioBuilder::Cuas, None) => {
                bail!("C-UAS builder requires a [cuas] section")
            }
            (ScenarioBuilder::Standard | ScenarioBuilder::Wildfire, Some(_)) => {
                bail!("[cuas] section requires scenario.builder = 'cuas'")
            }
            (ScenarioBuilder::Standard | ScenarioBuilder::Wildfire, None) => {}
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
    #[serde(default)]
    pub builder: ScenarioBuilder,
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
    #[serde(default)]
    pub affiliation: Affiliation,
    pub domain: Domain,
    pub position: Position,
    #[serde(default)]
    pub radios: Vec<Radio>,
    #[serde(default)]
    pub mission: MissionConfig,
    #[serde(default)]
    pub mission_role: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WildfireConfig {
    pub base_id: String,
    pub supervisor_id: String,
    pub reload_slots: usize,
    pub reload_time_s: f64,
    pub drop_duration_s: f64,
    pub drop_effect: f64,
    pub on_station_s: f64,
    pub egress_time_s: f64,
    pub launch_interval_s: f64,
    pub arrival_radius_m: f64,
    pub base_arrival_radius_m: f64,
    pub approach_distance_m: f64,
    pub base_approach_lane_deg: f64,
    #[serde(default)]
    pub spread_per_s: f64,
    pub flocking: FlockingConfig,
    pub fire_cells: Vec<FireCellConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FireCellConfig {
    pub id: String,
    pub name: String,
    pub position: Position,
    pub intensity: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CuasConfig {
    pub protected_site_id: String,
    pub detection_range_m: f64,
    pub detection_delay_s: f64,
    pub ew_capacity: usize,
    pub ew_effect_delay_s: f64,
    pub ew_success_probability: f64,
    pub interceptor_capacity: usize,
    pub interceptor_range_m: f64,
    pub intercept_time_s: f64,
    pub intercept_success_probability: f64,
    pub gun_range_m: f64,
    pub gun_effect_delay_s: f64,
    pub gun_success_probability: f64,
}

impl CuasConfig {
    fn validate(
        &self,
        scenario: &ScenarioConfig,
        ids: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        if !ids.contains(&self.protected_site_id) {
            bail!(
                "cuas.protected_site_id references unknown entity '{}'",
                self.protected_site_id
            );
        }
        let site = scenario
            .nodes
            .iter()
            .find(|node| node.id == self.protected_site_id)
            .expect("protected site ID was validated");
        if site.kind != EntityKind::ProtectedSite {
            bail!("cuas.protected_site_id must reference a kind='protected_site' entity");
        }
        for kind in [
            EntityKind::RadarSensor,
            EntityKind::EwJammer,
            EntityKind::Interceptor,
            EntityKind::GunSystem,
            EntityKind::ThreatUas,
        ] {
            if !scenario.nodes.iter().any(|node| node.kind == kind) {
                bail!("C-UAS scenario requires at least one kind='{kind:?}' node");
            }
        }
        if scenario.nodes.iter().any(|node| {
            node.kind == EntityKind::ThreatUas && node.affiliation != Affiliation::Hostile
        }) {
            bail!("all threat_uas nodes must use affiliation='hostile'");
        }
        if scenario.nodes.iter().any(|node| {
            matches!(
                node.kind,
                EntityKind::RadarSensor
                    | EntityKind::EwJammer
                    | EntityKind::Interceptor
                    | EntityKind::GunSystem
            ) && node.affiliation != Affiliation::Friendly
        }) {
            bail!("all C-UAS defender nodes must use affiliation='friendly'");
        }
        let probabilities = [
            self.ew_success_probability,
            self.intercept_success_probability,
            self.gun_success_probability,
        ];
        if probabilities
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            bail!("C-UAS effect probabilities must be finite and in [0,1]");
        }
        if self.detection_range_m <= 0.0
            || self.detection_delay_s < 0.0
            || self.ew_capacity == 0
            || self.ew_effect_delay_s <= 0.0
            || self.interceptor_capacity == 0
            || self.interceptor_range_m <= 0.0
            || self.intercept_time_s <= 0.0
            || self.gun_range_m <= 0.0
            || self.gun_effect_delay_s <= 0.0
        {
            bail!("C-UAS ranges, capacities, and positive effect delays must be valid");
        }
        Ok(())
    }
}

impl WildfireConfig {
    fn validate(
        &self,
        scenario: &ScenarioConfig,
        ids: &std::collections::BTreeSet<String>,
    ) -> Result<()> {
        if !ids.contains(&self.base_id) {
            bail!(
                "wildfire.base_id references unknown entity '{}'",
                self.base_id
            );
        }
        if !ids.contains(&self.supervisor_id) {
            bail!(
                "wildfire.supervisor_id references unknown entity '{}'",
                self.supervisor_id
            );
        }
        let base = scenario
            .nodes
            .iter()
            .find(|node| node.id == self.base_id)
            .unwrap();
        if base.kind != EntityKind::Base {
            bail!("wildfire.base_id must reference a kind='base' entity");
        }
        let tanker_count = scenario
            .nodes
            .iter()
            .filter(|node| node.kind == EntityKind::AirTanker)
            .count();
        if !(8..=16).contains(&tanker_count) {
            bail!("wildfire scenario requires 8-16 air_tanker nodes; found {tanker_count}");
        }
        if self.fire_cells.is_empty() {
            bail!("wildfire.fire_cells must not be empty");
        }
        let mut cell_ids = std::collections::BTreeSet::new();
        for cell in &self.fire_cells {
            if !cell_ids.insert(&cell.id) || ids.contains(&cell.id) {
                bail!("duplicate wildfire fire-cell id '{}'", cell.id);
            }
            if !cell.intensity.is_finite() || !(0.0..=100.0).contains(&cell.intensity) {
                bail!("fire cell '{}' intensity must be in [0,100]", cell.id);
            }
        }
        if self.reload_slots == 0
            || self.reload_time_s <= 0.0
            || self.drop_duration_s <= 0.0
            || self.drop_effect <= 0.0
            || self.arrival_radius_m <= 0.0
            || self.base_arrival_radius_m <= 0.0
            || self.flocking.min_speed_mps <= 0.0
            || self.flocking.max_speed_mps < self.flocking.min_speed_mps
            || self.flocking.max_turn_rate_deg_s <= 0.0
            || self.flocking.neighbor_radius_m <= 0.0
            || self.flocking.separation_radius_m <= 0.0
        {
            bail!("wildfire mission and flocking parameters must be positive and ordered");
        }
        Ok(())
    }
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
