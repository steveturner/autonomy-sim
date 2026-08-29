use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{
    model::{Entity, EntityKind},
    network::{LinkState, LinkStatus},
};

pub const TASKING_COLLECTION: &str = "c2.tasking";
pub const PLI_COLLECTION: &str = "c2.pli";
pub const TRACKS_COLLECTION: &str = "c2.tracks";
pub const TELEMETRY_COLLECTION: &str = "telemetry.platform";
pub const FIRE_CELLS_COLLECTION: &str = "mission.fire_cells";
pub const BASE_QUEUE_COLLECTION: &str = "mission.base_queue";
pub const DROP_ASSIGNMENTS_COLLECTION: &str = "mission.drop_assignments";

pub fn peer_id(entity_id: &str) -> String {
    format!("ditto/{entity_id}")
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DittoPeerState {
    pub peer_id: String,
    pub entity_id: String,
    pub connected_peer_ids: Vec<String>,
    pub document_count: usize,
    pub pending_documents: usize,
    pub converged: bool,
    pub collection_versions: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DittoDocumentState {
    pub collection: String,
    pub document_id: String,
    pub author_peer_id: String,
    pub revision: u64,
    pub updated_at_s: f64,
    pub value: serde_json::Value,
    pub replicated_to: Vec<String>,
    pub converged: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DittoReplicationEvent {
    pub collection: String,
    pub document_id: String,
    pub revision: u64,
    pub from_peer_id: String,
    pub to_peer_id: String,
    pub link_id: String,
    pub replicated_at_s: f64,
}

#[derive(Clone, Debug, Default)]
pub struct DittoFrame {
    pub peers: Vec<DittoPeerState>,
    pub documents: Vec<DittoDocumentState>,
    pub replication_events: Vec<DittoReplicationEvent>,
    pub document_ops_by_link: BTreeMap<String, u32>,
    pub pending_documents_by_link: BTreeMap<String, u32>,
}

#[derive(Clone, Debug)]
struct DocumentReplica {
    collection: String,
    document_id: String,
    author_peer_id: String,
    revision: u64,
    updated_at_s: f64,
    value: serde_json::Value,
    peer_revisions: BTreeMap<String, u64>,
}

pub struct DittoModel {
    entity_to_peer: BTreeMap<String, String>,
    documents: BTreeMap<String, DocumentReplica>,
    update_interval_ticks: u64,
}

impl DittoModel {
    pub fn new(
        entities: &[Entity],
        gateway_entity_id: &str,
        scenario_name: &str,
        tick_hz: f64,
    ) -> Self {
        let entity_to_peer: BTreeMap<_, _> = entities
            .iter()
            .filter(|entity| !matches!(entity.kind, EntityKind::Fire | EntityKind::Waypoint))
            .map(|entity| (entity.id.clone(), peer_id(&entity.id)))
            .collect();
        let peer_revisions = entity_to_peer.values().cloned().map(|id| (id, 0)).collect();
        let gateway_peer_id = peer_id(gateway_entity_id);
        let task_id = format!("task/{scenario_name}");
        let mut task = DocumentReplica {
            collection: TASKING_COLLECTION.into(),
            document_id: task_id.clone(),
            author_peer_id: gateway_peer_id.clone(),
            revision: 1,
            updated_at_s: 0.0,
            value: serde_json::json!({ "scenario": scenario_name, "status": "authorized" }),
            peer_revisions,
        };
        task.peer_revisions.insert(gateway_peer_id, 1);
        Self {
            entity_to_peer,
            documents: BTreeMap::from([(document_key(TASKING_COLLECTION, &task_id), task)]),
            update_interval_ticks: tick_hz.round().max(1.0) as u64,
        }
    }

    pub fn tick(
        &mut self,
        sequence: u64,
        sim_time_s: f64,
        entities: &[Entity],
        links: &[LinkState],
    ) -> DittoFrame {
        if sequence == 1 || sequence.is_multiple_of(self.update_interval_ticks) {
            for entity in entities {
                if !self.entity_to_peer.contains_key(&entity.id) {
                    continue;
                }
                self.upsert_document(
                    PLI_COLLECTION,
                    &format!("pli/{}", entity.id),
                    &entity.id,
                    serde_json::json!({
                        "entity_id": entity.id,
                        "position": entity.position,
                        "heading_deg": entity.heading_deg,
                    }),
                    sim_time_s,
                );
                self.upsert_document(
                    TRACKS_COLLECTION,
                    &format!("track/{}", entity.id),
                    &entity.id,
                    serde_json::json!({
                        "entity_id": entity.id,
                        "kind": entity.kind,
                        "affiliation": entity.affiliation,
                        "sidc": entity.sidc,
                    }),
                    sim_time_s,
                );
                self.upsert_document(
                    TELEMETRY_COLLECTION,
                    &format!("telemetry/{}", entity.id),
                    &entity.id,
                    serde_json::json!({
                        "mission_role": entity.mission_role,
                        "mission_state": entity.mission_state,
                        "retardant_pct": entity.retardant_pct,
                        "intensity": entity.intensity,
                    }),
                    sim_time_s,
                );
            }
        }

        let mut events = Vec::new();
        let mut document_ops_by_link = BTreeMap::new();
        for link in best_peer_links(links).values() {
            let budget = 1 + (link.quality * 5.0).floor() as usize;
            let mut transferred = 0;
            for document in self.documents.values_mut() {
                if transferred >= budget {
                    break;
                }
                let source_revision = document
                    .peer_revisions
                    .get(&link.source_peer_id)
                    .copied()
                    .unwrap_or(0);
                let target_revision = document
                    .peer_revisions
                    .get(&link.target_peer_id)
                    .copied()
                    .unwrap_or(0);
                let (from_peer_id, to_peer_id, revision) = if source_revision > target_revision {
                    (&link.source_peer_id, &link.target_peer_id, source_revision)
                } else if target_revision > source_revision {
                    (&link.target_peer_id, &link.source_peer_id, target_revision)
                } else {
                    continue;
                };
                document.peer_revisions.insert(to_peer_id.clone(), revision);
                events.push(DittoReplicationEvent {
                    collection: document.collection.clone(),
                    document_id: document.document_id.clone(),
                    revision,
                    from_peer_id: from_peer_id.clone(),
                    to_peer_id: to_peer_id.clone(),
                    link_id: link.id.clone(),
                    replicated_at_s: sim_time_s,
                });
                *document_ops_by_link.entry(link.id.clone()).or_default() += 1;
                transferred += 1;
            }
        }

        let mut frame = self.snapshot(links);
        frame.replication_events = events;
        frame.document_ops_by_link = document_ops_by_link;
        frame
    }

    pub fn snapshot(&self, links: &[LinkState]) -> DittoFrame {
        let connected = connected_peers(links);
        let peer_count = self.entity_to_peer.len();
        let documents: Vec<_> = self
            .documents
            .values()
            .map(|document| {
                let replicated_to: Vec<_> = document
                    .peer_revisions
                    .iter()
                    .filter(|(_, revision)| **revision == document.revision)
                    .map(|(peer, _)| peer.clone())
                    .collect();
                DittoDocumentState {
                    collection: document.collection.clone(),
                    document_id: document.document_id.clone(),
                    author_peer_id: document.author_peer_id.clone(),
                    revision: document.revision,
                    updated_at_s: document.updated_at_s,
                    value: document.value.clone(),
                    converged: replicated_to.len() == peer_count,
                    replicated_to,
                }
            })
            .collect();
        let peers = self
            .entity_to_peer
            .iter()
            .map(|(entity_id, peer)| {
                let mut document_count = 0;
                let mut pending_documents = 0;
                let mut collection_versions = BTreeMap::new();
                for document in self.documents.values() {
                    let revision = document.peer_revisions.get(peer).copied().unwrap_or(0);
                    if revision > 0 {
                        document_count += 1;
                        *collection_versions
                            .entry(document.collection.clone())
                            .or_default() += revision;
                    }
                    if revision < document.revision {
                        pending_documents += 1;
                    }
                }
                DittoPeerState {
                    peer_id: peer.clone(),
                    entity_id: entity_id.clone(),
                    connected_peer_ids: connected.get(peer).cloned().unwrap_or_default(),
                    document_count,
                    pending_documents,
                    converged: pending_documents == 0,
                    collection_versions,
                }
            })
            .collect();
        let pending_documents_by_link = links
            .iter()
            .filter(|link| link.state == LinkStatus::Up)
            .map(|link| {
                let pending = self
                    .documents
                    .values()
                    .filter(|document| {
                        document
                            .peer_revisions
                            .get(&link.source_peer_id)
                            .copied()
                            .unwrap_or(0)
                            != document
                                .peer_revisions
                                .get(&link.target_peer_id)
                                .copied()
                                .unwrap_or(0)
                    })
                    .count() as u32;
                (link.id.clone(), pending)
            })
            .collect();
        DittoFrame {
            peers,
            documents,
            pending_documents_by_link,
            ..DittoFrame::default()
        }
    }

    pub fn peer_has_latest(&self, entity_id: &str, collection: &str, document_id: &str) -> bool {
        let Some(peer) = self.entity_to_peer.get(entity_id) else {
            return false;
        };
        self.documents
            .get(&document_key(collection, document_id))
            .is_some_and(|document| {
                document.peer_revisions.get(peer).copied().unwrap_or(0) == document.revision
            })
    }

    pub fn upsert_document(
        &mut self,
        collection: &str,
        document_id: &str,
        author_entity_id: &str,
        value: serde_json::Value,
        sim_time_s: f64,
    ) {
        let author_peer_id = peer_id(author_entity_id);
        let peer_ids: Vec<_> = self.entity_to_peer.values().cloned().collect();
        let document = self
            .documents
            .entry(document_key(collection, document_id))
            .or_insert_with(|| DocumentReplica {
                collection: collection.into(),
                document_id: document_id.into(),
                author_peer_id: author_peer_id.clone(),
                revision: 0,
                updated_at_s: sim_time_s,
                value: serde_json::Value::Null,
                peer_revisions: peer_ids.into_iter().map(|peer| (peer, 0)).collect(),
            });
        if document.value == value {
            return;
        }
        document.revision += 1;
        document.updated_at_s = sim_time_s;
        document.value = value;
        document
            .peer_revisions
            .insert(author_peer_id, document.revision);
    }
}

fn document_key(collection: &str, document_id: &str) -> String {
    format!("{collection}/{document_id}")
}

fn best_peer_links(links: &[LinkState]) -> BTreeMap<(String, String), LinkState> {
    let mut best = BTreeMap::new();
    for link in links.iter().filter(|link| link.state == LinkStatus::Up) {
        let key = (link.source_peer_id.clone(), link.target_peer_id.clone());
        let replace = best
            .get(&key)
            .is_none_or(|current: &LinkState| link.quality > current.quality);
        if replace {
            best.insert(key, link.clone());
        }
    }
    best
}

fn connected_peers(links: &[LinkState]) -> BTreeMap<String, Vec<String>> {
    let mut connected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for link in links.iter().filter(|link| link.state == LinkStatus::Up) {
        connected
            .entry(link.source_peer_id.clone())
            .or_default()
            .insert(link.target_peer_id.clone());
        connected
            .entry(link.target_peer_id.clone())
            .or_default()
            .insert(link.source_peer_id.clone());
    }
    connected
        .into_iter()
        .map(|(peer, neighbors)| (peer, neighbors.into_iter().collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Affiliation, Domain, EntityKind, Kinematics, LinkType, MissionState, Position},
        symbology::{SymbolStatus, icon_hint, sidc},
    };

    fn entity(id: &str) -> Entity {
        Entity {
            id: id.into(),
            name: id.into(),
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

    fn link(state: LinkStatus) -> LinkState {
        LinkState {
            id: "link/mesh/a/b".into(),
            source: "a".into(),
            target: "b".into(),
            source_peer_id: peer_id("a"),
            target_peer_id: peer_id("b"),
            link_type: LinkType::Mesh,
            state,
            quality: 1.0,
            distance_m: 1.0,
            latency_ms: 1.0,
            packet_loss: 0.0,
            capacity_bps: 1_000_000,
        }
    }

    #[test]
    fn replicas_persist_offline_and_converge_after_reconnect() {
        let entities = vec![entity("a"), entity("b")];
        let mut model = DittoModel::new(&entities, "a", "test", 100.0);
        let partitioned = model.tick(1, 1.0, &entities, &[link(LinkStatus::Down)]);
        assert!(partitioned.peers.iter().any(|peer| !peer.converged));
        let b_count = partitioned
            .peers
            .iter()
            .find(|peer| peer.entity_id == "b")
            .unwrap()
            .document_count;
        assert!(
            b_count > 0,
            "peer keeps its own local documents while offline"
        );

        for sequence in 2..20 {
            model.tick(
                sequence,
                sequence as f64,
                &entities,
                &[link(LinkStatus::Up)],
            );
        }
        let converged = model.snapshot(&[link(LinkStatus::Up)]);
        assert!(converged.peers.iter().all(|peer| peer.converged));
        assert!(
            converged
                .documents
                .iter()
                .all(|document| document.converged)
        );
    }
}
