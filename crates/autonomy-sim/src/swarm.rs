use serde::Deserialize;

use crate::model::Position;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub east: f64,
    pub north: f64,
}

impl Vector2 {
    fn magnitude(self) -> f64 {
        self.east.hypot(self.north)
    }

    fn normalized(self) -> Self {
        let magnitude = self.magnitude();
        if magnitude <= f64::EPSILON {
            Self::default()
        } else {
            self * (1.0 / magnitude)
        }
    }

    fn heading_deg(self) -> f64 {
        (self.east.atan2(self.north).to_degrees() + 360.0) % 360.0
    }
}

impl std::ops::Add for Vector2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            east: self.east + rhs.east,
            north: self.north + rhs.north,
        }
    }
}

impl std::ops::Mul<f64> for Vector2 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            east: self.east * rhs,
            north: self.north * rhs,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoidState {
    pub id: String,
    pub position: Position,
    pub heading_deg: f64,
    pub speed_mps: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct FlockingConfig {
    pub neighbor_radius_m: f64,
    pub separation_radius_m: f64,
    pub separation_weight: f64,
    pub alignment_weight: f64,
    pub cohesion_weight: f64,
    pub goal_weight: f64,
    pub min_speed_mps: f64,
    pub max_speed_mps: f64,
    pub max_turn_rate_deg_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteeringOutput {
    pub heading_deg: f64,
    pub speed_mps: f64,
    pub velocity: Vector2,
    pub neighbor_count: usize,
}

pub fn steer(
    boid_index: usize,
    boids: &[BoidState],
    goal: Position,
    config: FlockingConfig,
    dt_s: f64,
) -> SteeringOutput {
    let boid = &boids[boid_index];
    let mut separation = Vector2::default();
    let mut alignment = Vector2::default();
    let mut center = Vector2::default();
    let mut neighbor_count = 0;

    for (index, neighbor) in boids.iter().enumerate() {
        if index == boid_index {
            continue;
        }
        let relative = offset_m(boid.position, neighbor.position);
        let distance = relative.magnitude();
        if distance > config.neighbor_radius_m || distance <= f64::EPSILON {
            continue;
        }
        neighbor_count += 1;
        center = center + relative;
        alignment = alignment + heading_vector(neighbor.heading_deg);
        if distance < config.separation_radius_m {
            separation = separation + relative * (-1.0 / distance.max(1.0));
        }
    }

    let goal_vector = offset_m(boid.position, goal).normalized();
    let mut desired = goal_vector * config.goal_weight;
    if neighbor_count > 0 {
        let scale = 1.0 / neighbor_count as f64;
        desired = desired
            + separation.normalized() * config.separation_weight
            + (alignment * scale).normalized() * config.alignment_weight
            + (center * scale).normalized() * config.cohesion_weight;
    }
    if desired.magnitude() <= f64::EPSILON {
        desired = heading_vector(boid.heading_deg);
    }

    let desired_heading = desired.heading_deg();
    let max_turn = config.max_turn_rate_deg_s.max(0.0) * dt_s.max(0.0);
    let delta = signed_heading_delta(boid.heading_deg, desired_heading).clamp(-max_turn, max_turn);
    let heading_deg = (boid.heading_deg + delta + 360.0) % 360.0;
    let speed_mps = boid
        .speed_mps
        .clamp(config.min_speed_mps, config.max_speed_mps);
    SteeringOutput {
        heading_deg,
        speed_mps,
        velocity: heading_vector(heading_deg) * speed_mps,
        neighbor_count,
    }
}

fn heading_vector(heading_deg: f64) -> Vector2 {
    let heading = heading_deg.to_radians();
    Vector2 {
        east: heading.sin(),
        north: heading.cos(),
    }
}

fn offset_m(origin: Position, target: Position) -> Vector2 {
    const METERS_PER_DEGREE: f64 = 111_320.0;
    let lon_scale = (METERS_PER_DEGREE * origin.lat_deg.to_radians().cos()).max(1.0);
    Vector2 {
        east: (target.lon_deg - origin.lon_deg) * lon_scale,
        north: (target.lat_deg - origin.lat_deg) * METERS_PER_DEGREE,
    }
}

fn signed_heading_delta(from: f64, to: f64) -> f64 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FlockingConfig {
        FlockingConfig {
            neighbor_radius_m: 500.0,
            separation_radius_m: 80.0,
            separation_weight: 1.8,
            alignment_weight: 0.8,
            cohesion_weight: 0.5,
            goal_weight: 2.5,
            min_speed_mps: 25.0,
            max_speed_mps: 60.0,
            max_turn_rate_deg_s: 10.0,
        }
    }

    #[test]
    fn steering_obeys_turn_and_speed_limits() {
        let start = Position {
            lat_deg: 39.2,
            lon_deg: -121.0,
            alt_m: 1_200.0,
        };
        let boids = vec![BoidState {
            id: "one".into(),
            position: start,
            heading_deg: 90.0,
            speed_mps: 100.0,
        }];
        let goal = Position {
            lat_deg: 39.3,
            ..start
        };
        let output = steer(0, &boids, goal, config(), 1.0);
        assert_eq!(output.heading_deg, 80.0);
        assert_eq!(output.speed_mps, 60.0);
        assert_eq!(output.neighbor_count, 0);
    }

    #[test]
    fn steering_is_deterministic_and_uses_neighbors() {
        let origin = Position {
            lat_deg: 39.2,
            lon_deg: -121.0,
            alt_m: 1_200.0,
        };
        let boids = vec![
            BoidState {
                id: "one".into(),
                position: origin,
                heading_deg: 315.0,
                speed_mps: 45.0,
            },
            BoidState {
                id: "two".into(),
                position: origin.moved(90.0, 50.0, 0.0),
                heading_deg: 300.0,
                speed_mps: 45.0,
            },
        ];
        let goal = origin.moved(315.0, 10_000.0, 0.0);
        let first = steer(0, &boids, goal, config(), 0.2);
        let second = steer(0, &boids, goal, config(), 0.2);
        assert_eq!(first, second);
        assert_eq!(first.neighbor_count, 1);
    }
}
