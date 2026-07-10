//! Core simulation primitives: time, calendar, seeded RNG.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

/// Fixed simulation ticks per in-game day.
pub const TICKS_PER_DAY: u64 = 1200;
pub const DAYS_PER_SEASON: u64 = 28;
pub const SEASONS_PER_YEAR: u64 = 4;
pub const DAYS_PER_YEAR: u64 = DAYS_PER_SEASON * SEASONS_PER_YEAR;

/// Simulation ticks per real-time second at normal speed.
pub const SIM_HZ: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub fn name(self) -> &'static str {
        match self {
            Season::Spring => "Spring",
            Season::Summer => "Summer",
            Season::Autumn => "Autumn",
            Season::Winter => "Winter",
        }
    }
}

/// The world clock. One instance lives for the entire game.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Calendar {
    pub tick: u64,
}

impl Calendar {
    pub fn advance(&mut self) {
        self.tick += 1;
    }

    pub fn day_of_season(&self) -> u64 {
        (self.tick / TICKS_PER_DAY) % DAYS_PER_SEASON + 1
    }

    pub fn season(&self) -> Season {
        match (self.tick / (TICKS_PER_DAY * DAYS_PER_SEASON)) % SEASONS_PER_YEAR {
            0 => Season::Spring,
            1 => Season::Summer,
            2 => Season::Autumn,
            _ => Season::Winter,
        }
    }

    pub fn year(&self) -> u64 {
        self.tick / (TICKS_PER_DAY * DAYS_PER_YEAR) + 1
    }
}

/// Deterministic RNG stream. Every subsystem should derive its own stream
/// from the world seed so replays and bug reports are reproducible.
pub fn rng_from_seed(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_rolls_over() {
        let mut c = Calendar::default();
        assert_eq!(c.season(), Season::Spring);
        assert_eq!(c.day_of_season(), 1);
        c.tick = TICKS_PER_DAY * DAYS_PER_SEASON; // first day of summer
        assert_eq!(c.season(), Season::Summer);
        assert_eq!(c.year(), 1);
        c.tick = TICKS_PER_DAY * DAYS_PER_YEAR; // first day of year 2
        assert_eq!(c.season(), Season::Spring);
        assert_eq!(c.year(), 2);
    }

    #[test]
    fn rng_is_deterministic() {
        use rand::Rng;
        let mut a = rng_from_seed(42);
        let mut b = rng_from_seed(42);
        assert_eq!(a.gen::<u64>(), b.gen::<u64>());
    }
}
