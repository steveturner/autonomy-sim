use std::path::PathBuf;

#[cfg(feature = "ditto-real")]
use std::collections::BTreeMap;

use anyhow::Result;
#[cfg(not(feature = "ditto-real"))]
use anyhow::bail;

use crate::{
    ditto::{DittoFrame, DittoModel},
    model::Entity,
    network::LinkState,
};

#[cfg(feature = "ditto-real")]
use crate::{
    ditto::{DittoReplicationEvent, is_ditto_peer, peer_id},
    network::LinkStatus,
};

#[derive(Clone, Debug)]
pub struct RealDittoOptions {
    pub database_id: String,
    pub license: String,
    pub storage_root: PathBuf,
    pub port_base: u16,
    pub listen_ip: String,
    /// Collections exposed through the real transport. An empty list retains
    /// the transport crate's default autonomy/wildfire collection set.
    pub collections: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub enum DittoTransportConfig {
    #[default]
    Behavioral,
    Real(RealDittoOptions),
}

pub struct DittoRuntime {
    model: DittoModel,
    #[cfg(feature = "ditto-real")]
    real: Option<RealState>,
}

#[cfg(feature = "ditto-real")]
struct RealState {
    transport: autonomy_sim_ditto_real::RealDittoTransport,
    entity_ids: std::collections::BTreeSet<String>,
    collections: std::collections::BTreeSet<String>,
    fallback_entity_id: String,
    written_revisions: BTreeMap<String, u64>,
    previous_replicas: BTreeMap<String, std::collections::BTreeSet<String>>,
}

impl DittoRuntime {
    pub fn new(
        entities: &[Entity],
        gateway_entity_id: &str,
        scenario_name: &str,
        tick_hz: f64,
        config: &DittoTransportConfig,
    ) -> Result<Self> {
        let model = DittoModel::new(entities, gateway_entity_id, scenario_name, tick_hz);
        match config {
            DittoTransportConfig::Behavioral => Ok(Self {
                model,
                #[cfg(feature = "ditto-real")]
                real: None,
            }),
            DittoTransportConfig::Real(options) => {
                #[cfg(not(feature = "ditto-real"))]
                {
                    let _ = options;
                    bail!(
                        "real Ditto transport is not compiled; rebuild with --features ditto-real"
                    )
                }
                #[cfg(feature = "ditto-real")]
                {
                    let peers: Vec<_> = entities
                        .iter()
                        .filter(|entity| is_ditto_peer(entity))
                        .map(|entity| autonomy_sim_ditto_real::RealDittoEntity {
                            entity_id: entity.id.clone(),
                            peer_id: peer_id(&entity.id),
                        })
                        .collect();
                    let entity_ids = peers.iter().map(|peer| peer.entity_id.clone()).collect();
                    let collections: std::collections::BTreeSet<_> =
                        if options.collections.is_empty() {
                            autonomy_sim_ditto_real::default_collections()
                                .into_iter()
                                .collect()
                        } else {
                            options.collections.iter().cloned().collect()
                        };
                    let storage_root = options.storage_root.join(safe_segment(scenario_name));
                    let transport = autonomy_sim_ditto_real::RealDittoTransport::new(
                        &peers,
                        autonomy_sim_ditto_real::RealDittoConfig {
                            database_id: options.database_id.clone(),
                            license: options.license.clone(),
                            storage_root,
                            port_base: options.port_base,
                            listen_ip: options.listen_ip.clone(),
                            collections: options.collections.clone(),
                        },
                    )?;
                    Ok(Self {
                        model,
                        real: Some(RealState {
                            transport,
                            entity_ids,
                            collections,
                            fallback_entity_id: gateway_entity_id.into(),
                            written_revisions: BTreeMap::new(),
                            previous_replicas: BTreeMap::new(),
                        }),
                    })
                }
            }
        }
    }

    pub fn upsert_document(
        &mut self,
        collection: &str,
        document_id: &str,
        author_entity_id: &str,
        value: serde_json::Value,
        sim_time_s: f64,
    ) {
        self.model
            .upsert_document(collection, document_id, author_entity_id, value, sim_time_s);
    }

    pub fn frame(
        &mut self,
        advance: bool,
        sequence: u64,
        sim_time_s: f64,
        entities: &[Entity],
        links: &[LinkState],
    ) -> Result<DittoFrame> {
        let behavioral = if advance {
            self.model.tick(sequence, sim_time_s, entities, links)
        } else {
            self.model.snapshot(links)
        };
        #[cfg(feature = "ditto-real")]
        if let Some(real) = &mut self.real {
            return real.frame(behavioral, sim_time_s, links);
        }
        Ok(behavioral)
    }

    pub fn peer_has_latest(
        &self,
        entity_id: &str,
        collection: &str,
        document_id: &str,
    ) -> Result<bool> {
        #[cfg(feature = "ditto-real")]
        if let Some(real) = &self.real {
            if !real.collections.contains(collection) {
                return Ok(false);
            }
            return Ok(real
                .transport
                .read_document(entity_id, collection, document_id)?
                .is_some());
        }
        Ok(self
            .model
            .peer_has_latest(entity_id, collection, document_id))
    }

    pub fn is_real(&self) -> bool {
        #[cfg(feature = "ditto-real")]
        if self.real.is_some() {
            return true;
        }
        false
    }
}

#[cfg(feature = "ditto-real")]
impl RealState {
    fn frame(
        &mut self,
        behavioral: DittoFrame,
        sim_time_s: f64,
        links: &[LinkState],
    ) -> Result<DittoFrame> {
        let real_links = self.real_links(links);
        self.transport.apply_links(&real_links)?;

        let metadata: BTreeMap<_, _> = behavioral
            .documents
            .iter()
            .filter(|document| self.collections.contains(&document.collection))
            .map(|document| {
                (
                    document_key(&document.collection, &document.document_id),
                    document.clone(),
                )
            })
            .collect();
        for document in &behavioral.documents {
            if !self.collections.contains(&document.collection) {
                continue;
            }
            let key = document_key(&document.collection, &document.document_id);
            if self.written_revisions.get(&key) == Some(&document.revision) {
                continue;
            }
            let requested_author = document
                .author_peer_id
                .strip_prefix("ditto/")
                .unwrap_or(&document.author_peer_id);
            let author = if self.entity_ids.contains(requested_author) {
                requested_author
            } else {
                &self.fallback_entity_id
            };
            self.transport.write_document(
                author,
                &document.collection,
                &document.document_id,
                document.value.clone(),
                document.updated_at_s,
            )?;
            self.written_revisions.insert(key, document.revision);
        }

        let observation = self.transport.observe(&real_links)?;
        let mut replication_events = Vec::new();
        let mut document_ops_by_link = BTreeMap::new();
        let mut current_replicas = BTreeMap::new();
        for document in &observation.documents {
            let key = document_key(&document.collection, &document.document_id);
            let current: std::collections::BTreeSet<_> =
                document.replicated_to.iter().cloned().collect();
            let previous = self
                .previous_replicas
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let meta = metadata.get(&key);
            for to_peer_id in current.difference(&previous) {
                if document.author_peer_id.as_ref() == Some(to_peer_id) {
                    continue;
                }
                if let Some((from_peer_id, link_id)) =
                    replication_path(to_peer_id, &previous, &current, links)
                {
                    replication_events.push(DittoReplicationEvent {
                        collection: document.collection.clone(),
                        document_id: document.document_id.clone(),
                        revision: meta.map_or(1, |document| document.revision),
                        from_peer_id,
                        to_peer_id: to_peer_id.clone(),
                        link_id: link_id.clone(),
                        replicated_at_s: sim_time_s,
                    });
                    *document_ops_by_link.entry(link_id).or_default() += 1;
                }
            }
            current_replicas.insert(key, current);
        }

        let pending_documents_by_link = links
            .iter()
            .filter(|link| link.state == LinkStatus::Up)
            .map(|link| {
                let pending = observation
                    .documents
                    .iter()
                    .filter(|document| {
                        let source = document.replicated_to.contains(&link.source_peer_id);
                        let target = document.replicated_to.contains(&link.target_peer_id);
                        source != target
                    })
                    .count() as u32;
                (link.id.clone(), pending)
            })
            .collect();

        let peers = observation
            .peers
            .into_iter()
            .map(|peer| crate::ditto::DittoPeerState {
                peer_id: peer.peer_id,
                entity_id: peer.entity_id,
                connected_peer_ids: peer.connected_peer_ids,
                document_count: peer.document_count,
                pending_documents: peer.pending_documents,
                converged: peer.converged,
                collection_versions: peer
                    .collection_document_counts
                    .into_iter()
                    .map(|(collection, count)| (collection, count as u64))
                    .collect(),
            })
            .collect();
        let documents = observation
            .documents
            .into_iter()
            .map(|document| {
                let key = document_key(&document.collection, &document.document_id);
                let meta = metadata.get(&key);
                let updated_at_s = document
                    .value
                    .get("updated_at_s")
                    .and_then(serde_json::Value::as_f64)
                    .or_else(|| meta.map(|document| document.updated_at_s))
                    .unwrap_or(sim_time_s);
                let value = document
                    .value
                    .get("payload")
                    .cloned()
                    .unwrap_or(document.value);
                crate::ditto::DittoDocumentState {
                    collection: document.collection,
                    document_id: document.document_id,
                    author_peer_id: document
                        .author_peer_id
                        .or_else(|| meta.map(|document| document.author_peer_id.clone()))
                        .unwrap_or_default(),
                    revision: meta.map_or(1, |document| document.revision),
                    updated_at_s,
                    value,
                    replicated_to: document.replicated_to,
                    converged: document.converged,
                }
            })
            .collect();
        self.previous_replicas = current_replicas;
        Ok(DittoFrame {
            peers,
            documents,
            replication_events,
            document_ops_by_link,
            pending_documents_by_link,
        })
    }

    fn real_links(&self, links: &[LinkState]) -> Vec<autonomy_sim_ditto_real::RealDittoLink> {
        links
            .iter()
            .filter(|link| {
                self.entity_ids.contains(&link.source) && self.entity_ids.contains(&link.target)
            })
            .map(|link| autonomy_sim_ditto_real::RealDittoLink {
                source_entity_id: link.source.clone(),
                target_entity_id: link.target.clone(),
                source_peer_id: link.source_peer_id.clone(),
                target_peer_id: link.target_peer_id.clone(),
                up: link.state == LinkStatus::Up,
            })
            .collect()
    }
}

#[cfg(feature = "ditto-real")]
fn replication_path(
    to_peer_id: &str,
    previous: &std::collections::BTreeSet<String>,
    current: &std::collections::BTreeSet<String>,
    links: &[LinkState],
) -> Option<(String, String)> {
    links
        .iter()
        .filter(|link| link.state == LinkStatus::Up)
        .find_map(|link| {
            if link.source_peer_id == to_peer_id
                && (previous.contains(&link.target_peer_id)
                    || current.contains(&link.target_peer_id))
            {
                Some((link.target_peer_id.clone(), link.id.clone()))
            } else if link.target_peer_id == to_peer_id
                && (previous.contains(&link.source_peer_id)
                    || current.contains(&link.source_peer_id))
            {
                Some((link.source_peer_id.clone(), link.id.clone()))
            } else {
                None
            }
        })
}

#[cfg(feature = "ditto-real")]
fn document_key(collection: &str, document_id: &str) -> String {
    format!("{collection}/{document_id}")
}

#[cfg(feature = "ditto-real")]
fn safe_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
