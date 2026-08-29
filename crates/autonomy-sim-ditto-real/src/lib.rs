#![cfg_attr(not(feature = "dittoffi"), forbid(unsafe_code))]
#![deny(unsafe_op_in_unsafe_fn)]

//! Real Ditto small-peer transport for autonomy-sim.
//!
//! The default build contains only the configuration and observation types.
//! Enable `dittoffi` to instantiate one real Ditto peer per scenario entity.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const TASKING_COLLECTION: &str = "c2.tasking";
pub const PLI_COLLECTION: &str = "c2.pli";
pub const TRACKS_COLLECTION: &str = "c2.tracks";
pub const TELEMETRY_COLLECTION: &str = "telemetry.platform";
pub const FIRE_CELLS_COLLECTION: &str = "mission.fire_cells";
pub const BASE_QUEUE_COLLECTION: &str = "mission.base_queue";
pub const DROP_ASSIGNMENTS_COLLECTION: &str = "mission.drop_assignments";

pub const AUTONOMY_COLLECTIONS: [&str; 7] = [
    TASKING_COLLECTION,
    PLI_COLLECTION,
    TRACKS_COLLECTION,
    TELEMETRY_COLLECTION,
    FIRE_CELLS_COLLECTION,
    BASE_QUEUE_COLLECTION,
    DROP_ASSIGNMENTS_COLLECTION,
];

/// The collections used by the existing ISR and wildfire scenarios.
pub fn default_collections() -> Vec<String> {
    AUTONOMY_COLLECTIONS
        .iter()
        .map(|value| (*value).into())
        .collect()
}

/// Cycle-free peer descriptor supplied by the simulator integration layer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RealDittoEntity {
    pub entity_id: String,
    pub peer_id: String,
}

/// Reachability edge supplied from the current emulated network frame.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RealDittoLink {
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub source_peer_id: String,
    pub target_peer_id: String,
    pub up: bool,
}

#[derive(Clone, Debug)]
pub struct RealDittoConfig {
    /// All peers must use the same database ID to join the same Ditto mesh.
    pub database_id: String,
    /// Offline Ditto license token. Read this from `DITTO_LICENSE`; do not store it in scenarios.
    pub license: String,
    /// Parent directory for one persistent Ditto store per entity.
    pub storage_root: PathBuf,
    /// First TCP port assigned to a peer. Later peers use consecutive ports.
    pub port_base: u16,
    /// Interface used by explicit Ditto TCP listeners and connections.
    pub listen_ip: String,
    /// Collections subscribed, accepted by read/write, and returned by observation.
    pub collections: Vec<String>,
}

impl RealDittoConfig {
    /// Constructs a configuration with the existing autonomy collection set.
    pub fn new(
        database_id: String,
        license: String,
        storage_root: PathBuf,
        port_base: u16,
        listen_ip: String,
    ) -> Self {
        Self {
            database_id,
            license,
            storage_root,
            port_base,
            listen_ip,
            collections: default_collections(),
        }
    }

    /// Replaces the subscribed collection set for a scenario integration.
    pub fn with_collections<I, S>(mut self, collections: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.collections = collections.into_iter().map(Into::into).collect();
        self
    }

    /// Adds scenario-specific collections while retaining the defaults.
    pub fn with_additional_collections<I, S>(mut self, collections: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for collection in collections {
            let collection = collection.into();
            if !self.collections.contains(&collection) {
                self.collections.push(collection);
            }
        }
        self
    }
}

#[cfg(any(feature = "dittoffi", test))]
pub(crate) fn valid_collection_name(collection: &str) -> bool {
    !collection.is_empty()
        && collection == collection.trim()
        && !collection
            .chars()
            .any(|character| character == '`' || character.is_control())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RealDittoPeerObservation {
    pub peer_id: String,
    pub entity_id: String,
    pub connected_peer_ids: Vec<String>,
    pub document_count: usize,
    pub pending_documents: usize,
    pub converged: bool,
    pub collection_document_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RealDittoDocumentObservation {
    pub collection: String,
    pub document_id: String,
    pub author_peer_id: Option<String>,
    pub replicated_to: Vec<String>,
    pub converged: bool,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RealDittoObservation {
    pub peers: Vec<RealDittoPeerObservation>,
    pub documents: Vec<RealDittoDocumentObservation>,
}

#[cfg(feature = "dittoffi")]
mod enabled;

#[cfg(feature = "dittoffi")]
pub use enabled::{RealDittoError, RealDittoTransport};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_uses_existing_collection_contract() {
        let config = RealDittoConfig::new(
            "database".into(),
            "license".into(),
            PathBuf::from("storage"),
            46_000,
            "127.0.0.1".into(),
        );

        assert_eq!(config.collections, default_collections());
    }

    #[test]
    fn collection_names_are_safe_for_quoted_dql() {
        assert!(valid_collection_name("cuas.ew_assignments"));
        assert!(!valid_collection_name(""));
        assert!(!valid_collection_name("cuas.`injected"));
        assert!(!valid_collection_name(" leading-space"));
    }

    #[test]
    fn scenario_collections_extend_defaults_without_duplicates() {
        let config = RealDittoConfig::new(
            "database".into(),
            "license".into(),
            PathBuf::from("storage"),
            46_000,
            "127.0.0.1".into(),
        )
        .with_additional_collections(["cuas.tracks", TASKING_COLLECTION]);

        assert!(
            config
                .collections
                .iter()
                .any(|value| value == "cuas.tracks")
        );
        assert_eq!(
            config
                .collections
                .iter()
                .filter(|value| value.as_str() == TASKING_COLLECTION)
                .count(),
            1
        );
    }
}
