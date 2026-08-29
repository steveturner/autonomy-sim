use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    #[serde(alias = "drone")]
    Uas,
    AirTanker,
    Rotary,
    Person,
    GroundVehicle,
    #[serde(alias = "ground_station")]
    Base,
    Fire,
    Waypoint,
    ThreatUas,
    #[serde(alias = "sensor")]
    RadarSensor,
    EwJammer,
    Interceptor,
    GunSystem,
    ProtectedSite,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Affiliation {
    #[default]
    Friendly,
    Hostile,
    Neutral,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Ground,
    Air,
    Maritime,
    Space,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, Ord, PartialEq, PartialOrd, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkType {
    Mesh,
    Cellular,
    Satcom,
    Ble,
}

impl std::fmt::Display for LinkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Mesh => "mesh",
            Self::Cellular => "cellular",
            Self::Satcom => "satcom",
            Self::Ble => "ble",
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Radio {
    pub link_type: LinkType,
    pub range_m: f64,
    pub capacity_bps: u64,
    pub base_latency_ms: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Position {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
}

impl Position {
    pub fn distance_to(self, other: Self) -> f64 {
        const EARTH_RADIUS_M: f64 = 6_371_000.0;
        let lat1 = self.lat_deg.to_radians();
        let lat2 = other.lat_deg.to_radians();
        let d_lat = lat2 - lat1;
        let d_lon = (other.lon_deg - self.lon_deg).to_radians();
        let a = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
        let horizontal = 2.0 * EARTH_RADIUS_M * a.sqrt().atan2((1.0 - a).sqrt());
        horizontal.hypot(other.alt_m - self.alt_m)
    }

    pub fn bearing_to(self, other: Self) -> f64 {
        let lat1 = self.lat_deg.to_radians();
        let lat2 = other.lat_deg.to_radians();
        let d_lon = (other.lon_deg - self.lon_deg).to_radians();
        let y = d_lon.sin() * lat2.cos();
        let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * d_lon.cos();
        (y.atan2(x).to_degrees() + 360.0) % 360.0
    }

    pub fn moved_toward(self, target: Self, max_distance_m: f64) -> Self {
        let distance = self.distance_to(target);
        if distance <= max_distance_m || distance <= f64::EPSILON {
            return target;
        }
        let fraction = (max_distance_m / distance).clamp(0.0, 1.0);
        Self {
            lat_deg: self.lat_deg + (target.lat_deg - self.lat_deg) * fraction,
            lon_deg: self.lon_deg + (target.lon_deg - self.lon_deg) * fraction,
            alt_m: self.alt_m + (target.alt_m - self.alt_m) * fraction,
        }
    }

    pub fn moved(self, heading_deg: f64, horizontal_m: f64, vertical_m: f64) -> Self {
        const EARTH_RADIUS_M: f64 = 6_371_000.0;
        if horizontal_m.abs() <= f64::EPSILON {
            return Self {
                alt_m: self.alt_m + vertical_m,
                ..self
            };
        }
        let angular = horizontal_m / EARTH_RADIUS_M;
        let bearing = heading_deg.to_radians();
        let lat1 = self.lat_deg.to_radians();
        let lon1 = self.lon_deg.to_radians();
        let lat2 = (lat1.sin() * angular.cos() + lat1.cos() * angular.sin() * bearing.cos()).asin();
        let lon2 = lon1
            + (bearing.sin() * angular.sin() * lat1.cos())
                .atan2(angular.cos() - lat1.sin() * lat2.sin());
        Self {
            lat_deg: lat2.to_degrees(),
            lon_deg: lon2.to_degrees(),
            alt_m: self.alt_m + vertical_m,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Kinematics {
    pub speed_mps: f64,
    pub heading_deg: f64,
    pub vertical_speed_mps: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Running,
    Success,
    Failure,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MissionState {
    pub playbook: String,
    pub active_node: String,
    pub status: MissionStatus,
}

impl Default for MissionState {
    fn default() -> Self {
        Self {
            playbook: "hold".into(),
            active_node: "hold_position".into(),
            status: MissionStatus::Running,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub kind: EntityKind,
    pub affiliation: Affiliation,
    pub sidc: String,
    pub icon_hint: String,
    pub domain: Domain,
    pub position: Position,
    pub kinematics: Kinematics,
    pub mission: MissionState,
    pub mission_role: String,
    pub mission_state: String,
    pub heading_deg: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retardant_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f64>,
    #[serde(skip_serializing)]
    pub radios: Vec<Radio>,
}

#[cfg(test)]
mod tests {
    use super::Position;

    #[test]
    fn moving_toward_respects_step_size() {
        let start = Position {
            lat_deg: 34.0,
            lon_deg: -117.0,
            alt_m: 0.0,
        };
        let end = Position {
            lat_deg: 34.01,
            lon_deg: -117.0,
            alt_m: 0.0,
        };
        let moved = start.moved_toward(end, 100.0);
        assert!((start.distance_to(moved) - 100.0).abs() < 0.5);
        assert!((start.bearing_to(end) - 0.0).abs() < 0.01);
    }

    #[test]
    fn moving_on_heading_preserves_distance_and_bearing() {
        let start = Position {
            lat_deg: 39.2,
            lon_deg: -121.0,
            alt_m: 100.0,
        };
        let moved = start.moved(315.0, 1_000.0, 25.0);
        assert!((start.distance_to(moved) - 1_000.3).abs() < 1.0);
        assert!((start.bearing_to(moved) - 315.0).abs() < 0.01);
        assert_eq!(moved.alt_m, 125.0);
    }
}
