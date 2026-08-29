use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Drone,
    Person,
    GroundVehicle,
    GroundStation,
    Sensor,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Ground,
    Air,
    Maritime,
    Space,
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
    pub domain: Domain,
    pub position: Position,
    pub kinematics: Kinematics,
    pub mission: MissionState,
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
}
