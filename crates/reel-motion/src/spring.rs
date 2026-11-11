//! A damped spring with a closed-form solution for each damping regime.

use reel_core::project::SpringFactors;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringConfig {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
    pub rest_delta: f64,
    pub rest_speed: f64,
}

impl SpringConfig {
    /// How damped it is: below 1 it oscillates, above 1 it doesn't.
    #[must_use]
    pub fn damping_ratio(&self) -> f64 {
        self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }

    fn with_factors(mut self, factors: SpringFactors) -> Self {
        self.stiffness *= factors.stiffness.clamp(0.25, 3.0);
        self.damping *= factors.damping.clamp(0.25, 3.0);
        self.mass *= factors.mass.clamp(0.25, 3.0);
        self
    }
}

/// The spring for the camera, from the Zoom Smoothness setting of 0 to 1.
#[must_use]
pub fn zoom_config(smoothness: f64, factors: SpringFactors) -> SpringConfig {
    let clamped = smoothness.clamp(0.0, 1.0);
    if clamped <= 0.0 {
        return SpringConfig {
            stiffness: 1000.0,
            damping: 100.0,
            mass: 1.0,
            rest_delta: 0.0001,
            rest_speed: 0.001,
        }
        .with_factors(factors);
    }
    let scaled = clamped * 2.0;
    SpringConfig {
        stiffness: 100.0 / scaled,
        damping: 21.0,
        mass: scaled,
        rest_delta: 0.0005,
        rest_speed: 0.015,
    }
    .with_factors(factors)
}

const CURSOR_STIFFNESS_BOOST: f64 = 1.12;

/// The spring for the cursor, from the smoothing factor.
#[must_use]
pub fn cursor_config(smoothing: f64, factors: SpringFactors) -> SpringConfig {
    let clamped = smoothing.clamp(0.0, 2.0);
    let config = if clamped <= 0.0 {
        SpringConfig {
            stiffness: 1000.0,
            damping: 100.0,
            mass: 1.0,
            rest_delta: 0.0001,
            rest_speed: 0.001,
        }
    } else if clamped <= 0.5 {
        let n = clamped / 0.5;
        SpringConfig {
            stiffness: (760.0 - n * 420.0) * CURSOR_STIFFNESS_BOOST,
            damping: 34.0 + n * 24.0,
            mass: 0.85 + n * 0.55,
            rest_delta: 0.0002,
            rest_speed: 0.01,
        }
    } else {
        let n = (clamped - 0.5) / 1.5;
        SpringConfig {
            stiffness: (340.0 - n * 180.0) * CURSOR_STIFFNESS_BOOST,
            damping: 58.0 + n * 22.0,
            mass: 1.35 + n * 0.45,
            rest_delta: 0.0002,
            rest_speed: 0.01,
        }
    };
    config.with_factors(factors)
}

fn position(t: f64, target: f64, delta: f64, velocity: f64, ratio: f64, frequency: f64) -> f64 {
    if ratio < 1.0 {
        let damped = frequency * (1.0 - ratio * ratio).sqrt();
        let envelope = (-ratio * frequency * t).exp();
        target
            - envelope
                * (((velocity + ratio * frequency * delta) / damped) * (damped * t).sin()
                    + delta * (damped * t).cos())
    } else if (ratio - 1.0).abs() < f64::EPSILON {
        target - (-frequency * t).exp() * (delta + (velocity + frequency * delta) * t)
    } else {
        let damped = frequency * (ratio * ratio - 1.0).sqrt();
        let envelope = (-ratio * frequency * t).exp();
        // Capped so sinh and cosh stay finite.
        let phase = (damped * t).min(300.0);
        target
            - (envelope
                * ((velocity + ratio * frequency * delta) * phase.sinh()
                    + damped * delta * phase.cosh()))
                / damped
    }
}

/// One value moving toward a target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub value: f64,
    pub velocity: f64,
    initialized: bool,
}

impl Spring {
    /// Jumps straight to a value, as after a cut in the timeline.
    pub fn snap(&mut self, value: f64) {
        self.value = value;
        self.velocity = 0.0;
        self.initialized = true;
    }

    /// Advances time. The first step goes straight to the target.
    pub fn step(&mut self, target: f64, delta_ms: f64, config: &SpringConfig) -> f64 {
        if !self.initialized || !self.value.is_finite() {
            self.snap(target);
            return self.value;
        }
        if (target - self.value).abs() <= config.rest_delta
            && self.velocity.abs() <= config.rest_speed
        {
            self.snap(target);
            return self.value;
        }

        let frequency = (config.stiffness / config.mass).sqrt();
        let ratio = config.damping_ratio();
        let delta = target - self.value;
        let velocity = -self.velocity;
        let seconds = delta_ms.clamp(1.0, 80.0) / 1000.0;
        let current = position(seconds, target, delta, velocity, ratio, frequency);

        // A moving target can carry an over-damped spring past it, which would make it wobble back.
        if ratio >= 1.0 {
            let crossed = (self.value <= target && current > target)
                || (self.value >= target && current < target);
            if crossed {
                self.snap(target);
                return self.value;
            }
        }

        let epsilon = 0.0001;
        let ahead = position(seconds + epsilon, target, delta, velocity, ratio, frequency);
        let new_velocity = (ahead - current) / epsilon;
        if new_velocity.abs() <= config.rest_speed && (target - current).abs() <= config.rest_delta
        {
            self.snap(target);
        } else {
            self.value = current;
            self.velocity = new_velocity;
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_CAMERA: SpringFactors = SpringFactors {
        stiffness: 1.0,
        damping: 1.13,
        mass: 1.12,
    };

    #[test]
    fn the_default_camera_spring_matches_the_documented_values() {
        // K 100, c 23.73, m 1.12, damping ratio about 1.12.
        let config = zoom_config(0.5, DEFAULT_CAMERA);
        assert!((config.stiffness - 100.0).abs() < 1e-9);
        assert!((config.damping - 23.73).abs() < 1e-9);
        assert!((config.mass - 1.12).abs() < 1e-9);
        assert!((config.damping_ratio() - 1.12).abs() < 0.005);
    }

    #[test]
    fn the_first_step_goes_straight_to_the_target() {
        let mut spring = Spring::default();
        let config = zoom_config(0.5, DEFAULT_CAMERA);
        assert_eq!(spring.step(5.0, 16.0, &config), 5.0);
    }

    #[test]
    fn it_settles_on_the_target_and_never_overshoots_when_over_damped() {
        let mut spring = Spring::default();
        spring.snap(0.0);
        let config = zoom_config(0.5, DEFAULT_CAMERA);
        let mut settled = false;
        for _ in 0..2000 {
            let value = spring.step(1.0, 16.0, &config);
            assert!((0.0..=1.0).contains(&value), "overshot: {value}");
            if value == 1.0 {
                settled = true;
                break;
            }
        }
        assert!(settled, "never reached the target");
    }

    #[test]
    fn a_less_damped_spring_can_overshoot() {
        let mut spring = Spring::default();
        spring.snap(0.0);
        let config = SpringConfig {
            stiffness: 400.0,
            damping: 4.0,
            mass: 1.0,
            rest_delta: 0.0001,
            rest_speed: 0.001,
        };
        assert!(config.damping_ratio() < 1.0);
        let highest = (0..200)
            .map(|_| spring.step(1.0, 8.0, &config))
            .fold(0.0, f64::max);
        assert!(highest > 1.0);
    }

    #[test]
    fn critical_damping_is_handled() {
        let mut spring = Spring::default();
        spring.snap(0.0);
        let stiffness = 100.0_f64;
        let config = SpringConfig {
            stiffness,
            damping: 2.0 * stiffness.sqrt(),
            mass: 1.0,
            rest_delta: 0.0005,
            rest_speed: 0.01,
        };
        assert!((config.damping_ratio() - 1.0).abs() < 1e-12);
        let last = (0..500)
            .map(|_| spring.step(1.0, 16.0, &config))
            .last()
            .unwrap();
        assert_eq!(last, 1.0);
    }

    #[test]
    fn a_long_step_is_clamped_and_stays_finite() {
        let mut spring = Spring::default();
        spring.snap(0.0);
        let config = zoom_config(0.5, DEFAULT_CAMERA);
        assert!(spring.step(1.0, 1e9, &config).is_finite());
    }

    #[test]
    fn the_cursor_spring_gets_softer_with_more_smoothing() {
        let factors = SpringFactors {
            stiffness: 1.0,
            damping: 1.0,
            mass: 1.0,
        };
        let (low, high) = (cursor_config(0.25, factors), cursor_config(1.5, factors));
        assert!(low.stiffness > high.stiffness);
        assert!(low.mass < high.mass);
    }
}
