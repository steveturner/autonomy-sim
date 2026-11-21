use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    net::{TcpStream, UdpSocket},
    path::Path,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::{
    model::{Affiliation, Entity, EntityKind},
    scenario::CotConfig,
};

pub trait CotSink: Send {
    fn emit(&mut self, event_xml: &str) -> std::io::Result<()>;
}

pub struct DisabledSink;

impl CotSink for DisabledSink {
    fn emit(&mut self, _event_xml: &str) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct FileSink {
    writer: BufWriter<File>,
}

impl FileSink {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating CoT output directory {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening CoT output {}", path.display()))?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }
}

impl CotSink for FileSink {
    fn emit(&mut self, event_xml: &str) -> std::io::Result<()> {
        self.writer.write_all(event_xml.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }
}

pub struct UdpSink {
    socket: UdpSocket,
}

impl UdpSink {
    pub fn connect(endpoint: &str) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").context("binding CoT UDP socket")?;
        socket
            .connect(endpoint)
            .with_context(|| format!("connecting CoT UDP sink to {endpoint}"))?;
        Ok(Self { socket })
    }
}

impl CotSink for UdpSink {
    fn emit(&mut self, event_xml: &str) -> std::io::Result<()> {
        self.socket.send(event_xml.as_bytes()).map(|_| ())
    }
}

pub struct TcpSink {
    stream: TcpStream,
}

impl TcpSink {
    pub fn connect(endpoint: &str) -> Result<Self> {
        Ok(Self {
            stream: TcpStream::connect(endpoint)
                .with_context(|| format!("connecting CoT TCP sink to {endpoint}"))?,
        })
    }
}

impl CotSink for TcpSink {
    fn emit(&mut self, event_xml: &str) -> std::io::Result<()> {
        self.stream.write_all(event_xml.as_bytes())?;
        self.stream.write_all(b"\n")
    }
}

pub fn sink_from_config(config: &CotConfig) -> Result<Box<dyn CotSink>> {
    match config.sink.as_str() {
        "disabled" => Ok(Box::new(DisabledSink)),
        "file" => Ok(Box::new(FileSink::open(&config.path)?)),
        "udp" if !config.endpoint.is_empty() => Ok(Box::new(UdpSink::connect(&config.endpoint)?)),
        "tcp" if !config.endpoint.is_empty() => Ok(Box::new(TcpSink::connect(&config.endpoint)?)),
        "udp" | "tcp" => bail!("cot.endpoint is required for {} sink", config.sink),
        other => bail!("unknown cot.sink '{other}'; expected disabled, file, udp, or tcp"),
    }
}

pub fn render_pli(entity: &Entity, now: DateTime<Utc>, stale_after_s: i64) -> String {
    let mut event = uas_cot::Event::new(
        format!("autonomy-sim-{}", entity.id),
        cot_type(entity.kind, entity.affiliation),
        now.timestamp_micros().max(0) as u64,
        stale_after_s.max(1) as u64,
    );
    event.point = uas_cot::Point {
        ce: 10.0,
        le: 10.0,
        ..uas_cot::Point::new(
            entity.position.lat_deg,
            entity.position.lon_deg,
            entity.position.alt_m,
        )
    };
    event.callsign = Some(entity.name.clone());
    event.track = Some(uas_cot::Track {
        course: entity.kinematics.heading_deg,
        speed: entity.kinematics.speed_mps,
    });
    event.remarks = Some(format!(
        "autonomy-sim ISR mission: {}",
        entity.mission.playbook
    ));
    event.to_xml()
}

fn cot_type(kind: EntityKind, affiliation: Affiliation) -> String {
    let tail = match kind {
        EntityKind::Uas | EntityKind::AirTanker | EntityKind::ThreatUas => "A-M-F-Q",
        EntityKind::Rotary => "A-M-H",
        EntityKind::Person => "G-U-C",
        EntityKind::GroundVehicle => "G-E-V",
        EntityKind::Base | EntityKind::ProtectedSite => "G-I",
        EntityKind::Fire => return "b-a-o-tbl".into(),
        EntityKind::Waypoint => return "b-m-p-w".into(),
        EntityKind::RadarSensor => "G-E-S-R",
        EntityKind::EwJammer => "G-U-S",
        EntityKind::Interceptor => "A-M-F",
        EntityKind::GunSystem => "G-E-W-D",
    };
    let identity = match affiliation {
        Affiliation::Friendly => 'f',
        Affiliation::Hostile => 'h',
        Affiliation::Neutral => 'n',
        Affiliation::Unknown => 'u',
    };
    format!("a-{identity}-{tail}")
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::{
        model::{Domain, Kinematics, MissionState, Position},
        symbology::{SymbolStatus, icon_hint, sidc},
    };

    #[test]
    fn pli_is_well_formed_and_escapes_callsign() {
        let entity = Entity {
            id: "uav-alpha".into(),
            name: "Alpha & One".into(),
            kind: EntityKind::Uas,
            affiliation: Affiliation::Friendly,
            sidc: sidc(
                EntityKind::Uas,
                Affiliation::Friendly,
                SymbolStatus::Present,
            ),
            icon_hint: icon_hint(EntityKind::Uas).into(),
            domain: Domain::Air,
            position: Position {
                lat_deg: 34.0,
                lon_deg: -117.0,
                alt_m: 180.0,
            },
            kinematics: Kinematics {
                speed_mps: 12.5,
                heading_deg: 90.0,
                vertical_speed_mps: 0.0,
            },
            mission: MissionState::default(),
            mission_role: "scout".into(),
            mission_state: "holding".into(),
            heading_deg: 0.0,
            retardant_pct: None,
            intensity: None,
            radios: Vec::new(),
        };
        let now = Utc.with_ymd_and_hms(2026, 8, 28, 12, 0, 0).unwrap();
        let xml = render_pli(&entity, now, 10);
        assert!(xml.contains("callsign=\"Alpha &amp; One\""));
        let mut reader = quick_xml::Reader::from_str(&xml);
        loop {
            if reader.read_event().unwrap() == quick_xml::events::Event::Eof {
                break;
            }
        }
    }

    #[test]
    fn cot_type_carries_entity_affiliation() {
        assert_eq!(
            cot_type(EntityKind::ThreatUas, Affiliation::Hostile),
            "a-h-A-M-F-Q"
        );
        assert_eq!(
            cot_type(EntityKind::Uas, Affiliation::Friendly),
            "a-f-A-M-F-Q"
        );
        assert_eq!(
            cot_type(EntityKind::GroundVehicle, Affiliation::Unknown),
            "a-u-G-E-V"
        );
        assert_eq!(
            cot_type(EntityKind::Fire, Affiliation::Hostile),
            "b-a-o-tbl"
        );
    }
}
