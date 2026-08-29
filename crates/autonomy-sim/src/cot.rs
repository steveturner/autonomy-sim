use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    net::{TcpStream, UdpSocket},
    path::Path,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use quick_xml::escape::escape;

use crate::{
    model::{Entity, EntityKind},
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
    let timestamp = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let stale = (now + Duration::seconds(stale_after_s.max(1)))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let uid = escape(&entity.id);
    let callsign = escape(&entity.name);
    let playbook = escape(&entity.mission.playbook);
    format!(
        concat!(
            "<event version=\"2.0\" uid=\"autonomy-sim-{uid}\" type=\"{cot_type}\" ",
            "time=\"{timestamp}\" start=\"{timestamp}\" stale=\"{stale}\" how=\"m-g\">",
            "<point lat=\"{lat:.7}\" lon=\"{lon:.7}\" hae=\"{alt:.1}\" ce=\"10.0\" le=\"10.0\"/>",
            "<detail><contact callsign=\"{callsign}\"/>",
            "<track course=\"{heading:.1}\" speed=\"{speed:.2}\"/>",
            "<remarks>autonomy-sim ISR mission: {playbook}</remarks></detail></event>"
        ),
        uid = uid,
        cot_type = cot_type(entity.kind),
        timestamp = timestamp,
        stale = stale,
        lat = entity.position.lat_deg,
        lon = entity.position.lon_deg,
        alt = entity.position.alt_m,
        callsign = callsign,
        heading = entity.kinematics.heading_deg,
        speed = entity.kinematics.speed_mps,
        playbook = playbook,
    )
}

fn cot_type(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Drone => "a-f-A-M-F-Q",
        EntityKind::Person => "a-f-G-U-C",
        EntityKind::GroundVehicle => "a-f-G-E-V",
        EntityKind::GroundStation => "a-f-G-U-C-I",
        EntityKind::Sensor => "a-f-G-I",
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::model::{Domain, Kinematics, MissionState, Position};

    #[test]
    fn pli_is_well_formed_and_escapes_callsign() {
        let entity = Entity {
            id: "uav-alpha".into(),
            name: "Alpha & One".into(),
            kind: EntityKind::Drone,
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
}
