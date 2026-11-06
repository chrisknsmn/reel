//! Times in a project, at microsecond precision so frame positions stay exact.
//!
//! There are two clocks: where something is in the recording, and where it is in the edited
//! timeline. Recordly stores click times in one and zoom times in the other, as plain numbers, and
//! mixes them up after edits (§58). Here they are different types, so that's a compile error:
//!
//! ```compile_fail
//! use reel_core::time::{SourceTime, TimelineTime};
//! let is_same = SourceTime::from_ms(1.0) == TimelineTime::from_ms(1.0);
//! ```
//!
//! The only way between the two is through a clip, which knows how it maps them.

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// A length of time, which means the same on either clock.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Micros(pub i64);

impl Micros {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub fn from_ms(ms: f64) -> Self {
        Self((ms * 1000.0).round() as i64)
    }

    #[must_use]
    pub const fn from_secs(secs: i64) -> Self {
        Self(secs * 1_000_000)
    }

    #[must_use]
    pub fn ms(self) -> f64 {
        self.0 as f64 / 1000.0
    }

    #[must_use]
    pub fn secs(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }

    /// Multiplies, rounding to the nearest microsecond.
    #[must_use]
    pub fn scaled(self, factor: f64) -> Self {
        Self((self.0 as f64 * factor).round() as i64)
    }
}

impl Add for Micros {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sub for Micros {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Neg for Micros {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

macro_rules! clock {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl $name {
            pub const ZERO: Self = Self(0);

            #[must_use]
            pub fn from_ms(ms: f64) -> Self {
                Self((ms * 1000.0).round() as i64)
            }

            #[must_use]
            pub fn ms(self) -> f64 {
                self.0 as f64 / 1000.0
            }

            #[must_use]
            pub fn secs(self) -> f64 {
                self.0 as f64 / 1_000_000.0
            }
        }

        impl Add<Micros> for $name {
            type Output = Self;
            fn add(self, other: Micros) -> Self {
                Self(self.0 + other.0)
            }
        }

        impl AddAssign<Micros> for $name {
            fn add_assign(&mut self, other: Micros) {
                self.0 += other.0;
            }
        }

        impl Sub<Micros> for $name {
            type Output = Self;
            fn sub(self, other: Micros) -> Self {
                Self(self.0 - other.0)
            }
        }

        impl SubAssign<Micros> for $name {
            fn sub_assign(&mut self, other: Micros) {
                self.0 -= other.0;
            }
        }

        impl Sub for $name {
            type Output = Micros;
            fn sub(self, other: Self) -> Micros {
                Micros(self.0 - other.0)
            }
        }
    };
}

clock!(
    /// A position in the edited timeline, which is what the playhead shows.
    TimelineTime
);
clock!(
    /// A position in the recording itself, which is what cursor data and audio files use.
    SourceTime
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliseconds_round_trip_at_frame_precision() {
        assert_eq!(Micros::from_ms(16.667).0, 16_667);
        assert_eq!(TimelineTime::from_ms(1522.575).0, 1_522_575);
        assert!((SourceTime::from_ms(1522.575).ms() - 1522.575).abs() < 1e-9);
    }

    #[test]
    fn a_clock_only_moves_by_a_length_of_time() {
        let start = TimelineTime::from_ms(100.0);
        let later = start + Micros::from_ms(50.0);
        assert_eq!(later - start, Micros::from_ms(50.0));
    }
}
