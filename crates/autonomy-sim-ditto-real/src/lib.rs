#![cfg_attr(not(feature = "dittoffi"), forbid(unsafe_code))]
#![deny(unsafe_op_in_unsafe_fn)]

//! Real Ditto small-peer transport for autonomy-sim.
//!
//! The default build contains only the configuration and observation types.
//! Enable `dittoffi` to instantiate one real Ditto peer per scenario entity.

use std::{collections::BTreeMap, path::PathBuf};

use autonomy_sim::ditto::{
    PLI_COLLECTION, TASKING_COLLECTION, TELEMETRY_COLLECTION, TRACKS_COLLECTION,
};
use serde::{Deserialize, Serialize};

pub const AUTONOMY_COLLECTIONS: [&str; 4] = [
    TASKING_COLLECTION,
    PLI_COLLECTION,
    TRACKS_COLLECTION,
    TELEMETRY_COLLECTION,
];

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
