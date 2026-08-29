use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, CString, c_char, c_void},
    fs,
    ptr::{self, NonNull},
};

use serde_json::{Value, json};
use thiserror::Error;

use crate::{
    RealDittoConfig, RealDittoDocumentObservation, RealDittoEntity, RealDittoLink,
    RealDittoObservation, RealDittoPeerObservation, valid_collection_name,
};

#[derive(Debug, Error)]
pub enum RealDittoError {
    #[error("invalid real-Ditto configuration: {0}")]
    InvalidConfig(String),
    #[error("unknown scenario entity '{0}'")]
    UnknownEntity(String),
    #[error("invalid Ditto collection '{0}'")]
    InvalidCollection(String),
    #[error("document value must be a JSON object")]
    DocumentNotObject,
    #[error("Ditto FFI rejected a string containing a null byte")]
    NullByte(#[from] std::ffi::NulError),
    #[error("Ditto data directory failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("serializing Ditto CBOR failed: {0}")]
    Cbor(#[from] serde_cbor::Error),
    #[error("decoding Ditto JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dittoffi: {0}")]
    Ditto(String),
}

unsafe extern "C" {
    fn autonomy_sim_ditto_peer_open(
        working_directory: *const c_char,
        database_id: *const c_char,
        license: *const c_char,
        out_error: *mut *mut c_char,
    ) -> *mut c_void;
    fn autonomy_sim_ditto_peer_subscribe(
        peer: *mut c_void,
        query: *const c_char,
        out_error: *mut *mut c_char,
    ) -> bool;
    fn autonomy_sim_ditto_peer_set_transport(
        peer: *mut c_void,
        config: *const u8,
        config_length: usize,
        out_error: *mut *mut c_char,
    ) -> bool;
    fn autonomy_sim_ditto_peer_start(peer: *mut c_void, out_error: *mut *mut c_char) -> bool;
    fn autonomy_sim_ditto_peer_exec(
        peer: *mut c_void,
        statement: *const c_char,
        args: *const u8,
        args_length: usize,
        out_error: *mut *mut c_char,
    ) -> bool;
    fn autonomy_sim_ditto_peer_query_json(
        peer: *mut c_void,
        statement: *const c_char,
        args: *const u8,
        args_length: usize,
        out_json: *mut *mut c_char,
        out_error: *mut *mut c_char,
    ) -> bool;
    fn autonomy_sim_ditto_peer_free(peer: *mut c_void);
    fn autonomy_sim_ditto_string_free(value: *mut c_char);
}

struct RawPeer(NonNull<c_void>);

// SAFETY: the Ditto SDK owns its internal synchronization and the bridge peer
// is only accessed through the owning transport. `Send` permits moving that
// owner between executor threads; it does not permit concurrent access.
unsafe impl Send for RawPeer {}

impl RawPeer {
    fn open(
        working_directory: &CString,
        database_id: &CString,
        license: &CString,
    ) -> Result<Self, RealDittoError> {
        let mut error = ptr::null_mut();
        // SAFETY: the byte and C-string pointers remain valid for the duration
        // of the call; the bridge returns either an owned peer or an error.
        let peer = unsafe {
            autonomy_sim_ditto_peer_open(
                working_directory.as_ptr(),
                database_id.as_ptr(),
                license.as_ptr(),
                &mut error,
            )
        };
        NonNull::new(peer)
            .map(Self)
            .ok_or_else(|| take_error(error))
    }

    fn subscribe(&self, query: &CString) -> Result<(), RealDittoError> {
        let mut error = ptr::null_mut();
        // SAFETY: self owns a live bridge peer and query is a valid C string.
        let ok = unsafe {
            autonomy_sim_ditto_peer_subscribe(self.0.as_ptr(), query.as_ptr(), &mut error)
        };
        bool_result(ok, error)
    }

    fn set_transport(&self, config: &[u8]) -> Result<(), RealDittoError> {
        let mut error = ptr::null_mut();
        // SAFETY: self owns a live bridge peer and config remains valid during the call.
        let ok = unsafe {
            autonomy_sim_ditto_peer_set_transport(
                self.0.as_ptr(),
                config.as_ptr(),
                config.len(),
                &mut error,
            )
        };
        bool_result(ok, error)
    }

    fn start(&self) -> Result<(), RealDittoError> {
        let mut error = ptr::null_mut();
        // SAFETY: self owns a live bridge peer.
        let ok = unsafe { autonomy_sim_ditto_peer_start(self.0.as_ptr(), &mut error) };
        bool_result(ok, error)
    }

    fn exec(&self, statement: &CString, args: &[u8]) -> Result<(), RealDittoError> {
        let mut error = ptr::null_mut();
        let args_ptr = if args.is_empty() {
            ptr::null()
        } else {
            args.as_ptr()
        };
        // SAFETY: all pointers remain valid during the call and self owns the peer.
        let ok = unsafe {
            autonomy_sim_ditto_peer_exec(
                self.0.as_ptr(),
                statement.as_ptr(),
                args_ptr,
                args.len(),
                &mut error,
            )
        };
        bool_result(ok, error)
    }

    fn query(&self, statement: &CString, args: &[u8]) -> Result<Vec<Value>, RealDittoError> {
        let mut json_ptr = ptr::null_mut();
        let mut error = ptr::null_mut();
        let args_ptr = if args.is_empty() {
            ptr::null()
        } else {
            args.as_ptr()
        };
        // SAFETY: all input pointers remain valid during the call. On success
        // the returned string is owned by the bridge and freed below.
        let ok = unsafe {
            autonomy_sim_ditto_peer_query_json(
                self.0.as_ptr(),
                statement.as_ptr(),
                args_ptr,
                args.len(),
                &mut json_ptr,
                &mut error,
            )
        };
        if !ok {
            return Err(take_error(error));
        }
        if json_ptr.is_null() {
            return Err(RealDittoError::Ditto(
                "query succeeded without a JSON result".into(),
            ));
        }
        // SAFETY: the bridge guarantees a non-null, null-terminated UTF-8 JSON string.
        let json = unsafe { CStr::from_ptr(json_ptr) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: json_ptr was allocated by the bridge for this call.
        unsafe { autonomy_sim_ditto_string_free(json_ptr) };
        Ok(serde_json::from_str(&json)?)
    }
}

impl Drop for RawPeer {
    fn drop(&mut self) {
        // SAFETY: RawPeer uniquely owns this bridge allocation.
        unsafe { autonomy_sim_ditto_peer_free(self.0.as_ptr()) };
    }
}

fn bool_result(ok: bool, error: *mut c_char) -> Result<(), RealDittoError> {
    if ok { Ok(()) } else { Err(take_error(error)) }
}

fn take_error(error: *mut c_char) -> RealDittoError {
    if error.is_null() {
        return RealDittoError::Ditto("unknown error".into());
    }
    // SAFETY: the bridge returns a null-terminated error allocated by itself.
    let message = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: error was allocated by the bridge for this failed call.
    unsafe { autonomy_sim_ditto_string_free(error) };
    RealDittoError::Ditto(message)
}

struct Peer {
    entity_id: String,
    peer_id: String,
    port: u16,
    raw: RawPeer,
}

/// Owns one actual Ditto small peer per scenario entity.
pub struct RealDittoTransport {
    config: RealDittoConfig,
    peers: BTreeMap<String, Peer>,
    active_pairs: BTreeSet<(String, String)>,
}

impl RealDittoTransport {
    pub fn new(
        entities: &[RealDittoEntity],
        config: RealDittoConfig,
    ) -> Result<Self, RealDittoError> {
        if entities.is_empty() {
            return Err(RealDittoError::InvalidConfig(
                "at least one peer entity is required".into(),
            ));
        }
        if config.database_id.is_empty() {
            return Err(RealDittoError::InvalidConfig(
                "database_id must not be empty".into(),
            ));
        }
        if config.license.is_empty() {
            return Err(RealDittoError::InvalidConfig(
                "license must not be empty".into(),
            ));
        }
        if config.collections.is_empty() {
            return Err(RealDittoError::InvalidConfig(
                "at least one collection is required".into(),
            ));
        }
        let mut unique_collections = BTreeSet::new();
        for collection in &config.collections {
            if !valid_collection_name(collection) {
                return Err(RealDittoError::InvalidConfig(format!(
                    "collection name '{collection}' is empty or unsafe for DQL"
                )));
            }
            if !unique_collections.insert(collection) {
                return Err(RealDittoError::InvalidConfig(format!(
                    "duplicate collection '{collection}'"
                )));
            }
        }
        let last_port = usize::from(config.port_base) + entities.len() - 1;
        if last_port > usize::from(u16::MAX) {
            return Err(RealDittoError::InvalidConfig(
                "port range exceeds 65535".into(),
            ));
        }
        fs::create_dir_all(&config.storage_root)?;
        let license = CString::new(config.license.as_str())?;
        let database_id = CString::new(config.database_id.as_str())?;
        let mut peers = BTreeMap::new();

        for (index, entity) in entities.iter().enumerate() {
            if peers.contains_key(&entity.entity_id) {
                return Err(RealDittoError::InvalidConfig(format!(
                    "duplicate entity ID '{}'",
                    entity.entity_id
                )));
            }
            let storage = config.storage_root.join(format!(
                "{index:04}-{}",
                safe_path_segment(&entity.entity_id)
            ));
            fs::create_dir_all(&storage)?;
            let storage = CString::new(storage.to_string_lossy().as_bytes())?;
            let raw = RawPeer::open(&storage, &database_id, &license)?;
            for collection in &config.collections {
                raw.subscribe(&CString::new(format!("SELECT * FROM `{collection}`"))?)?;
            }
            let port = config.port_base + index as u16;
            raw.set_transport(&transport_config(&config.listen_ip, port, &[])?)?;
            raw.start()?;
            peers.insert(
                entity.entity_id.clone(),
                Peer {
                    entity_id: entity.entity_id.clone(),
                    peer_id: entity.peer_id.clone(),
                    port,
                    raw,
                },
            );
        }

        Ok(Self {
            config,
            peers,
            active_pairs: BTreeSet::new(),
        })
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Applies the current emulated link matrix to Ditto's explicit TCP
    /// transport. Multiple up carriers between the same peers produce one
    /// transport connection; removing the last carrier tears it down.
    pub fn apply_links(&mut self, links: &[RealDittoLink]) -> Result<(), RealDittoError> {
        let mut active_pairs = BTreeSet::new();
        for link in links.iter().filter(|link| link.up) {
            if !self.peers.contains_key(&link.source_entity_id) {
                return Err(RealDittoError::UnknownEntity(link.source_entity_id.clone()));
            }
            if !self.peers.contains_key(&link.target_entity_id) {
                return Err(RealDittoError::UnknownEntity(link.target_entity_id.clone()));
            }
            active_pairs.insert(sorted_pair(&link.source_entity_id, &link.target_entity_id));
        }
        if active_pairs == self.active_pairs {
            return Ok(());
        }

        let mut connections: BTreeMap<String, Vec<String>> = self
            .peers
            .keys()
            .cloned()
            .map(|entity_id| (entity_id, Vec::new()))
            .collect();
        for (lower, upper) in &active_pairs {
            let lower_peer = &self.peers[lower];
            connections
                .get_mut(upper)
                .expect("both peers were validated")
                .push(format!("{}:{}", self.config.listen_ip, lower_peer.port));
        }
        for (entity_id, addresses) in connections {
            let peer = &self.peers[&entity_id];
            peer.raw.set_transport(&transport_config(
                &self.config.listen_ip,
                peer.port,
                &addresses,
            )?)?;
        }
        self.active_pairs = active_pairs;
        Ok(())
    }

    pub fn write_document(
        &self,
        entity_id: &str,
        collection: &str,
        document_id: &str,
        value: Value,
        sim_time_s: f64,
    ) -> Result<(), RealDittoError> {
        self.validate_collection(collection)?;
        let peer = self
            .peers
            .get(entity_id)
            .ok_or_else(|| RealDittoError::UnknownEntity(entity_id.into()))?;
        if !value.is_object() {
            return Err(RealDittoError::DocumentNotObject);
        }
        let statement = CString::new(format!(
            "INSERT INTO `{collection}` DOCUMENTS (:doc) ON ID CONFLICT DO UPDATE"
        ))?;
        let args = serde_cbor::to_vec(&json!({
            "doc": {
                "_id": document_id,
                "author_peer_id": peer.peer_id,
                "updated_at_s": sim_time_s,
                "payload": value,
            }
        }))?;
        peer.raw.exec(&statement, &args)
    }

    pub fn read_document(
        &self,
        entity_id: &str,
        collection: &str,
        document_id: &str,
    ) -> Result<Option<Value>, RealDittoError> {
        self.validate_collection(collection)?;
        let peer = self
            .peers
            .get(entity_id)
            .ok_or_else(|| RealDittoError::UnknownEntity(entity_id.into()))?;
        let statement = CString::new(format!("SELECT * FROM `{collection}` WHERE _id = :id"))?;
        let args = serde_cbor::to_vec(&json!({ "id": document_id }))?;
        Ok(peer.raw.query(&statement, &args)?.into_iter().next())
    }

    /// Reads real collection contents back from every peer for visualization
    /// and convergence reporting.
    pub fn observe(&self, links: &[RealDittoLink]) -> Result<RealDittoObservation, RealDittoError> {
        let connected = connected_peers(links);
        let mut peer_documents: BTreeMap<String, BTreeMap<(String, String), Value>> =
            BTreeMap::new();
        for peer in self.peers.values() {
            let mut documents = BTreeMap::new();
            for collection in &self.config.collections {
                let query = CString::new(format!("SELECT * FROM `{collection}`"))?;
                for document in peer.raw.query(&query, &[])? {
                    let Some(document_id) = document.get("_id").and_then(Value::as_str) else {
                        continue;
                    };
                    documents.insert((collection.clone(), document_id.into()), document);
                }
            }
            peer_documents.insert(peer.entity_id.clone(), documents);
        }

        let all_keys: BTreeSet<_> = peer_documents
            .values()
            .flat_map(|documents| documents.keys().cloned())
            .collect();
        let peer_count = self.peers.len();
        let mut documents = Vec::new();
        for (collection, document_id) in all_keys {
            let replicas: Vec<_> = self
                .peers
                .values()
                .filter_map(|peer| {
                    peer_documents[&peer.entity_id]
                        .get(&(collection.clone(), document_id.clone()))
                        .map(|value| (peer.peer_id.clone(), value.clone()))
                })
                .collect();
            let converged = replicas.len() == peer_count
                && replicas
                    .first()
                    .is_none_or(|(_, first)| replicas.iter().all(|(_, value)| value == first));
            let value = replicas
                .first()
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null);
            let author_peer_id = value
                .get("author_peer_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            documents.push(RealDittoDocumentObservation {
                collection,
                document_id,
                author_peer_id,
                replicated_to: replicas.into_iter().map(|(peer, _)| peer).collect(),
                converged,
                value,
            });
        }

        let peers = self
            .peers
            .values()
            .map(|peer| {
                let local = &peer_documents[&peer.entity_id];
                let pending_documents = documents
                    .iter()
                    .filter(|document| {
                        local
                            .get(&(document.collection.clone(), document.document_id.clone()))
                            .is_none_or(|value| value != &document.value)
                    })
                    .count();
                let mut collection_document_counts = BTreeMap::new();
                for (collection, _) in local.keys() {
                    *collection_document_counts
                        .entry(collection.clone())
                        .or_default() += 1;
                }
                RealDittoPeerObservation {
                    peer_id: peer.peer_id.clone(),
                    entity_id: peer.entity_id.clone(),
                    connected_peer_ids: connected.get(&peer.peer_id).cloned().unwrap_or_default(),
                    document_count: local.len(),
                    pending_documents,
                    converged: pending_documents == 0,
                    collection_document_counts,
                }
            })
            .collect();
        Ok(RealDittoObservation { peers, documents })
    }

    fn validate_collection(&self, collection: &str) -> Result<(), RealDittoError> {
        if self
            .config
            .collections
            .iter()
            .any(|configured| configured == collection)
        {
            Ok(())
        } else {
            Err(RealDittoError::InvalidCollection(collection.into()))
        }
    }
}

fn transport_config(
    listen_ip: &str,
    port: u16,
    addresses: &[String],
) -> Result<Vec<u8>, serde_cbor::Error> {
    serde_cbor::to_vec(&json!({
        "peer_to_peer": {
            "bluetooth_le": { "enabled": false },
            "lan": { "enabled": false },
            "wifi_aware": { "enabled": false },
            "awdl": { "enabled": false },
        },
        "connect": {
            "tcp_servers": addresses,
            "retry_interval": 100,
        },
        "listen": {
            "tcp": {
                "enabled": true,
                "interface_ip": listen_ip,
                "port": port,
            }
        }
    }))
}

fn sorted_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.into(), right.into())
    } else {
        (right.into(), left.into())
    }
}

fn safe_path_segment(value: &str) -> String {
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

fn connected_peers(links: &[RealDittoLink]) -> BTreeMap<String, Vec<String>> {
    let mut connected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for link in links.iter().filter(|link| link.up) {
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
