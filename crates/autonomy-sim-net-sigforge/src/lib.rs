#![forbid(unsafe_code)]

//! SigForge REST adapter for [`autonomy_sim::network::NetworkBackend`].
//!
//! SigForge currently exposes existing scenario NEMs and a directed SINR
//! matrix through REST. This adapter deterministically maps autonomy-sim
//! entities to those NEMs, publishes entity positions, and treats the weaker
//! of the two directed PHY measurements as the quality of a bidirectional
//! Ditto replication path.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use autonomy_sim::{
    ditto::peer_id,
    model::{Entity, LinkType, Radio},
    network::{LinkState, LinkStatus, NetworkBackend, NetworkError},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct SigForgeLink {
    pub src: u16,
    pub dst: u16,
    pub sinr_db: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct SigForgeNode {
    pub nem_id: u16,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
}

#[derive(Debug, Error)]
pub enum SigForgeError {
    #[error("unsupported SigForge URL '{0}'; only http:// REST endpoints are supported")]
    UnsupportedUrl(String),
    #[error("SigForge I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SigForge returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("invalid SigForge response: {0}")]
    InvalidResponse(String),
    #[error("SigForge JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Narrow client boundary so mapping behavior can be tested without a live
/// SigForge process. Implementations are synchronous because NetworkBackend is
/// called from autonomy-sim's fixed-step scheduler.
pub trait SigForgeApi: Send {
    fn nodes(&mut self) -> Result<Vec<SigForgeNode>, SigForgeError>;
    fn update_position(&mut self, nem_id: u16, entity: &Entity) -> Result<(), SigForgeError>;
    fn links(&mut self) -> Result<Vec<SigForgeLink>, SigForgeError>;
}

#[derive(Clone, Debug)]
struct HttpEndpoint {
    authority: String,
    host: String,
    port: u16,
    path_prefix: String,
}

impl HttpEndpoint {
    fn parse(base_url: &str) -> Result<Self, SigForgeError> {
        let Some(rest) = base_url.strip_prefix("http://") else {
            return Err(SigForgeError::UnsupportedUrl(base_url.to_owned()));
        };
        let (authority, path_prefix) = rest
            .split_once('/')
            .map_or((rest, String::new()), |(authority, path)| {
                (authority, format!("/{}", path.trim_end_matches('/')))
            });
        if authority.is_empty() {
            return Err(SigForgeError::UnsupportedUrl(base_url.to_owned()));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => (
                host.to_owned(),
                port.parse()
                    .map_err(|_| SigForgeError::UnsupportedUrl(base_url.to_owned()))?,
            ),
            _ => (authority.to_owned(), 80),
        };
        Ok(Self {
            authority: authority.to_owned(),
            host,
            port,
            path_prefix,
        })
    }
}

/// Dependency-light REST client for SigForge's `/api/v1/session` API.
///
/// It intentionally supports plain HTTP only. Deployments terminating TLS can
/// put a local proxy in front of SigForge, while the zero-dependency analytic
/// backend remains unaffected.
#[derive(Debug)]
pub struct RestSigForgeClient {
    endpoint: HttpEndpoint,
    timeout: Duration,
}

impl RestSigForgeClient {
    pub fn new(base_url: &str) -> Result<Self, SigForgeError> {
        Ok(Self {
            endpoint: HttpEndpoint::parse(base_url)?,
            timeout: Duration::from_secs(2),
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<String, SigForgeError> {
        let mut addresses = (self.endpoint.host.as_str(), self.endpoint.port).to_socket_addrs()?;
        let address = addresses.next().ok_or_else(|| {
            SigForgeError::InvalidResponse("SigForge host resolved to no addresses".into())
        })?;
        let mut stream = TcpStream::connect_timeout(&address, self.timeout)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        let body = body.unwrap_or("");
        let mut request = String::new();
        write!(
            request,
            "{method} {}{path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.endpoint.path_prefix, self.endpoint.authority
        )
        .expect("writing to String cannot fail");
        if !body.is_empty() {
            write!(
                request,
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            )
            .expect("writing to String cannot fail");
        }
        request.push_str("\r\n");
        request.push_str(body);
        stream.write_all(request.as_bytes())?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let response = String::from_utf8(response)
            .map_err(|error| SigForgeError::InvalidResponse(error.to_string()))?;
        let (headers, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
            SigForgeError::InvalidResponse("HTTP response had no header terminator".into())
        })?;
        if headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
        {
            return Err(SigForgeError::InvalidResponse(
                "chunked HTTP responses are not supported".into(),
            ));
        }
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| SigForgeError::InvalidResponse("invalid HTTP status line".into()))?;
        if !(200..300).contains(&status) {
            return Err(SigForgeError::Http {
                status,
                body: body.to_owned(),
            });
        }
        Ok(body.to_owned())
    }
}

impl SigForgeApi for RestSigForgeClient {
    fn nodes(&mut self) -> Result<Vec<SigForgeNode>, SigForgeError> {
        Ok(serde_json::from_str(&self.request(
            "GET",
            "/api/v1/session/nodes",
            None,
        )?)?)
    }

    fn update_position(&mut self, nem_id: u16, entity: &Entity) -> Result<(), SigForgeError> {
        #[derive(Serialize)]
        struct PositionUpdate {
            lat: f64,
            lon: f64,
            alt: f64,
        }
        let body = serde_json::to_string(&PositionUpdate {
            lat: entity.position.lat_deg,
            lon: entity.position.lon_deg,
            alt: entity.position.alt_m,
        })?;
        self.request(
            "PUT",
            &format!("/api/v1/session/nodes/{nem_id}/position"),
            Some(&body),
        )?;
        Ok(())
    }

    fn links(&mut self) -> Result<Vec<SigForgeLink>, SigForgeError> {
        Ok(serde_json::from_str(&self.request(
            "GET",
            "/api/v1/session/links",
            None,
        )?)?)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SigForgeMapping {
    /// Minimum SINR required in both directions for a usable Ditto path.
    pub minimum_sinr_db: f64,
    /// SINR at which normalized link quality reaches 1.0.
    pub full_quality_sinr_db: f64,
    /// Adapter/control-plane overhead added to the radio's base latency.
    pub adapter_latency_ms: f64,
}

impl Default for SigForgeMapping {
    fn default() -> Self {
        Self {
            minimum_sinr_db: 0.0,
            full_quality_sinr_db: 20.0,
            adapter_latency_ms: 2.0,
        }
    }
}

pub struct SigForgeNetworkBackend<C = RestSigForgeClient> {
    client: C,
    mapping: SigForgeMapping,
    entity_to_nem: BTreeMap<String, u16>,
}

impl SigForgeNetworkBackend<RestSigForgeClient> {
    pub fn connect(base_url: &str) -> Result<Self, SigForgeError> {
        Ok(Self::new(RestSigForgeClient::new(base_url)?))
    }
}

impl<C> SigForgeNetworkBackend<C> {
    pub fn new(client: C) -> Self {
        Self {
            client,
            mapping: SigForgeMapping::default(),
            entity_to_nem: BTreeMap::new(),
        }
    }

    pub fn with_mapping(mut self, mapping: SigForgeMapping) -> Self {
        self.mapping = mapping;
        self
    }

    pub fn entity_to_nem(&self) -> &BTreeMap<String, u16> {
        &self.entity_to_nem
    }
}

impl<C: SigForgeApi> SigForgeNetworkBackend<C> {
    fn unavailable(error: impl std::fmt::Display) -> NetworkError {
        NetworkError::Unavailable(format!("SigForge adapter: {error}"))
    }

    fn compatible_radios<'a>(
        source: &'a Entity,
        target: &'a Entity,
    ) -> Vec<(&'a Radio, &'a Radio)> {
        let target_radios: BTreeMap<LinkType, &Radio> = target
            .radios
            .iter()
            .map(|radio| (radio.link_type, radio))
            .collect();
        source
            .radios
            .iter()
            .filter_map(|source_radio| {
                target_radios
                    .get(&source_radio.link_type)
                    .map(|target_radio| (source_radio, *target_radio))
            })
            .collect()
    }

    fn map_link(
        &self,
        source: &Entity,
        target: &Entity,
        source_radio: &Radio,
        target_radio: &Radio,
        sinr_db: Option<f64>,
    ) -> LinkState {
        let distance_m = source.position.distance_to(target.position);
        let up = sinr_db.is_some_and(|sinr| sinr >= self.mapping.minimum_sinr_db);
        let quality = if up {
            let span = (self.mapping.full_quality_sinr_db - self.mapping.minimum_sinr_db)
                .max(f64::EPSILON);
            let normalized = ((sinr_db.unwrap_or_default() - self.mapping.minimum_sinr_db) / span)
                .clamp(0.0, 1.0);
            0.05 + 0.95 * normalized
        } else {
            0.0
        };
        let base_capacity = source_radio.capacity_bps.min(target_radio.capacity_bps);
        let capacity_bps = if up {
            (base_capacity as f64 * quality.powf(1.35)).round() as u64
        } else {
            0
        };
        let packet_loss = if up { (1.0 - quality).powi(2) } else { 1.0 };
        let base_latency = source_radio
            .base_latency_ms
            .max(target_radio.base_latency_ms);
        let latency_ms = base_latency
            + self.mapping.adapter_latency_ms
            + distance_m / 299_792.458
            + (1.0 - quality) * 20.0;
        let (source, target) = sorted_entities(source, target);
        LinkState {
            id: format!(
                "link/{}/{}/{}",
                source_radio.link_type, source.id, target.id
            ),
            source: source.id.clone(),
            target: target.id.clone(),
            source_peer_id: peer_id(&source.id),
            target_peer_id: peer_id(&target.id),
            link_type: source_radio.link_type,
            state: if up { LinkStatus::Up } else { LinkStatus::Down },
            quality,
            distance_m,
            latency_ms,
            packet_loss,
            capacity_bps,
        }
    }
}

impl<C: SigForgeApi> NetworkBackend for SigForgeNetworkBackend<C> {
    fn name(&self) -> &'static str {
        "sigforge-rest"
    }

    fn register_nodes(&mut self, entities: &[Entity]) -> Result<(), NetworkError> {
        let mut nem_ids: Vec<_> = self
            .client
            .nodes()
            .map_err(Self::unavailable)?
            .into_iter()
            .map(|node| node.nem_id)
            .collect();
        nem_ids.sort_unstable();
        nem_ids.dedup();
        if nem_ids.len() < entities.len() {
            return Err(Self::unavailable(format!(
                "SigForge has {} NEMs but autonomy-sim requires {}",
                nem_ids.len(),
                entities.len()
            )));
        }
        self.entity_to_nem = entities
            .iter()
            .zip(nem_ids)
            .map(|(entity, nem_id)| (entity.id.clone(), nem_id))
            .collect();
        Ok(())
    }

    fn link_states(
        &mut self,
        _sim_time_s: f64,
        entities: &[Entity],
    ) -> Result<Vec<LinkState>, NetworkError> {
        for entity in entities {
            let nem_id = self
                .entity_to_nem
                .get(&entity.id)
                .copied()
                .ok_or_else(|| NetworkError::UnknownEntity(entity.id.clone()))?;
            self.client
                .update_position(nem_id, entity)
                .map_err(Self::unavailable)?;
        }

        let measurements = self.client.links().map_err(Self::unavailable)?;
        let mut directed = BTreeMap::new();
        for measurement in measurements {
            directed
                .entry((measurement.src, measurement.dst))
                .and_modify(|sinr: &mut f64| *sinr = sinr.max(measurement.sinr_db))
                .or_insert(measurement.sinr_db);
        }

        let mut links = Vec::new();
        for (index, source) in entities.iter().enumerate() {
            for target in &entities[(index + 1)..] {
                let source_nem = self.entity_to_nem[&source.id];
                let target_nem = self.entity_to_nem[&target.id];
                let bidirectional_sinr = directed
                    .get(&(source_nem, target_nem))
                    .zip(directed.get(&(target_nem, source_nem)))
                    .map(|(forward, reverse)| forward.min(*reverse));
                for (source_radio, target_radio) in Self::compatible_radios(source, target) {
                    links.push(self.map_link(
                        source,
                        target,
                        source_radio,
                        target_radio,
                        bidirectional_sinr,
                    ));
                }
            }
        }
        links.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(links)
    }
}

fn sorted_entities<'a>(left: &'a Entity, right: &'a Entity) -> (&'a Entity, &'a Entity) {
    if left.id <= right.id {
        (left, right)
    } else {
        (right, left)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use autonomy_sim::model::{
        Affiliation, Domain, EntityKind, Kinematics, MissionState, Position,
    };

    use super::*;

    struct FakeApi {
        nodes: Vec<SigForgeNode>,
        link_frames: VecDeque<Vec<SigForgeLink>>,
        updated: Vec<(u16, String)>,
    }

    impl SigForgeApi for FakeApi {
        fn nodes(&mut self) -> Result<Vec<SigForgeNode>, SigForgeError> {
            Ok(self.nodes.clone())
        }

        fn update_position(&mut self, nem_id: u16, entity: &Entity) -> Result<(), SigForgeError> {
            self.updated.push((nem_id, entity.id.clone()));
            Ok(())
        }

        fn links(&mut self) -> Result<Vec<SigForgeLink>, SigForgeError> {
            Ok(self.link_frames.pop_front().unwrap_or_default())
        }
    }

    fn entity(id: &str, lon: f64) -> Entity {
        Entity {
            id: id.into(),
            name: id.into(),
            kind: EntityKind::Uas,
            affiliation: Affiliation::Friendly,
            sidc: String::new(),
            icon_hint: String::new(),
            domain: Domain::Air,
            position: Position {
                lat_deg: 34.0,
                lon_deg: lon,
                alt_m: 100.0,
            },
            kinematics: Kinematics::default(),
            mission: MissionState::default(),
            mission_role: String::new(),
            mission_state: String::new(),
            heading_deg: 0.0,
            retardant_pct: None,
            intensity: None,
            radios: vec![Radio {
                link_type: LinkType::Mesh,
                range_m: 10.0,
                capacity_bps: 1_000_000,
                base_latency_ms: 4.0,
            }],
        }
    }

    #[test]
    fn maps_bidirectional_phy_results_at_the_network_backend_boundary() {
        let api = FakeApi {
            nodes: vec![
                SigForgeNode {
                    nem_id: 40,
                    lat: 0.0,
                    lon: 0.0,
                    alt: 0.0,
                },
                SigForgeNode {
                    nem_id: 90,
                    lat: 0.0,
                    lon: 0.0,
                    alt: 0.0,
                },
            ],
            link_frames: VecDeque::from([
                vec![
                    SigForgeLink {
                        src: 40,
                        dst: 90,
                        sinr_db: 16.0,
                    },
                    SigForgeLink {
                        src: 90,
                        dst: 40,
                        sinr_db: 8.0,
                    },
                ],
                vec![SigForgeLink {
                    src: 40,
                    dst: 90,
                    sinr_db: 30.0,
                }],
            ]),
            updated: Vec::new(),
        };
        let mut backend = SigForgeNetworkBackend::new(api);
        let entities = vec![entity("bravo", -117.001), entity("alpha", -117.0)];
        backend.register_nodes(&entities).unwrap();
        assert_eq!(backend.entity_to_nem()["bravo"], 40);
        assert_eq!(backend.entity_to_nem()["alpha"], 90);

        let up = backend.link_states(1.0, &entities).unwrap();
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].id, "link/mesh/alpha/bravo");
        assert_eq!(up[0].state, LinkStatus::Up);
        assert!((up[0].quality - 0.43).abs() < 1e-9);
        assert!(up[0].capacity_bps > 0);

        let missing_reverse = backend.link_states(2.0, &entities).unwrap();
        assert_eq!(missing_reverse[0].state, LinkStatus::Down);
        assert_eq!(missing_reverse[0].quality, 0.0);
        assert_eq!(missing_reverse[0].packet_loss, 1.0);
    }

    #[test]
    fn registration_fails_closed_when_sigforge_has_too_few_nems() {
        let api = FakeApi {
            nodes: Vec::new(),
            link_frames: VecDeque::new(),
            updated: Vec::new(),
        };
        let mut backend = SigForgeNetworkBackend::new(api);
        let error = backend.register_nodes(&[entity("alpha", 0.0)]).unwrap_err();
        assert!(error.to_string().contains("has 0 NEMs"));
    }

    #[test]
    fn rest_client_uses_sigforge_v1_endpoints() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let server_seen = Arc::clone(&seen);
        let server = thread::spawn(move || {
            for body in [
                r#"[{"nem_id":7,"lat":1.0,"lon":2.0,"alt":3.0}]"#,
                "",
                r#"[{"src":7,"dst":8,"sinr_db":11.5}]"#,
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0_u8; 2048];
                    let read = stream.read(&mut chunk).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                server_seen
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });

        let mut client = RestSigForgeClient::new(&format!("http://{address}")).unwrap();
        assert_eq!(client.nodes().unwrap()[0].nem_id, 7);
        client.update_position(7, &entity("alpha", 2.0)).unwrap();
        assert_eq!(client.links().unwrap()[0].sinr_db, 11.5);
        server.join().unwrap();

        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("GET /api/v1/session/nodes HTTP/1.1"));
        assert!(seen[1].starts_with("PUT /api/v1/session/nodes/7/position HTTP/1.1"));
        assert!(seen[2].starts_with("GET /api/v1/session/links HTTP/1.1"));
    }

    #[test]
    fn rejects_non_http_urls() {
        let error = RestSigForgeClient::new("https://sigforge.example").unwrap_err();
        assert!(matches!(error, SigForgeError::UnsupportedUrl(_)));
    }
}
