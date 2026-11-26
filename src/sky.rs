//! The night over Inbox Zero once it has come up. It is still: a little after it is up,
//! one star falls, and then nothing moves. Its clock is the wall clock, so a late frame
//! shows the star where it really is, never a slowed one.

use std::time::{Duration, Instant};

/// How long after the night is up the star falls.
const WAIT: f32 = 2.5;
/// How long the falling star takes to cross, in seconds.
const FALL: f32 = 1.1;
/// How far it goes, and its longest tail, in points.
const TRAVEL: f32 = 260.;
const TAIL: f32 = 120.;
/// The dots its tail is drawn with (GPUI draws no slanted lines).
const DOTS: usize = 90;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fall {
    /// Where it starts, as shares of the night's width and height.
    pub x: f32,
    pub y: f32,
    /// Which way it goes (a unit vector, always downwards).
    dx: f32,
    dy: f32,
}

/// One dot of a falling star: its place from the start (points), size, and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub alpha: f32,
}

pub struct Sky {
    up: Instant,
    /// Gone without being seen: the window went to the background first.
    missed: bool,
}

/// The one star: fixed, so every night is the same and tests can follow it. From the left
/// third, 26 degrees below level, heading right.
const STAR: Fall = Fall {
    x: 0.3,
    y: 0.09,
    dx: 0.898_794,
    dy: 0.438_371,
};

impl Sky {
    pub fn new(up: Instant) -> Self {
        Sky { up, missed: false }
    }

    /// How far into its fall the star is, 0 to 1, while it falls.
    fn age(&self, now: Instant) -> Option<f32> {
        if self.missed {
            return None;
        }
        let seconds = now.saturating_duration_since(self.up).as_secs_f32() - WAIT;
        (0. ..FALL).contains(&seconds).then(|| seconds / FALL)
    }

    pub fn falling(&self, now: Instant) -> bool {
        self.age(now).is_some()
    }

    /// How long until the star falls; `None` once it has, or never will.
    pub fn until_fall(&self, now: Instant) -> Option<Duration> {
        let left = WAIT - now.saturating_duration_since(self.up).as_secs_f32();
        (!self.missed && left > 0.).then(|| Duration::from_secs_f32(left))
    }

    /// The window went to the background: a star that has not finished falling never
    /// shows, rather than stopping in mid-flight.
    pub fn let_go(&mut self, now: Instant) {
        if self.until_fall(now).is_some() || self.falling(now) {
            self.missed = true;
        }
    }

    /// The falling star now: where it started, and its dots from the head back.
    pub fn fall(&self, now: Instant) -> Option<(Fall, Vec<Dot>)> {
        let age = self.age(now)?;
        // Quick, then slowing a little as it burns out.
        let gone = 1. - (1. - age) * (1. - age);
        let head = TRAVEL * gone;
        let tail = head.min(TAIL * (1. - 0.4 * age));
        let shown = (age / 0.12).min(1.) * ((1. - age) / 0.35).min(1.);
        let dots = (0..DOTS)
            .map(|k| {
                let back = k as f32 / (DOTS - 1) as f32;
                let along = head - tail * back;
                // Close enough to overlap into one streak; overlapping dots add up, so
                // the tail is faint and the head stands out.
                let (size, alpha) = if k == 0 {
                    (2.6, shown)
                } else {
                    (2. - 1.2 * back, shown * 0.45 * (1. - back).powf(1.8))
                };
                Dot {
                    x: STAR.dx * along,
                    y: STAR.dy * along,
                    size,
                    alpha,
                }
            })
            .collect();
        Some((STAR, dots))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(up: Instant, seconds: f32) -> Instant {
        up + Duration::from_secs_f32(seconds)
    }

    #[test]
    fn one_star_falls_and_then_the_sky_is_still() {
        let up = Instant::now();
        let sky = Sky::new(up);
        assert!(!sky.falling(at(up, 1.)));
        assert!(sky.falling(at(up, WAIT + 0.5)));
        for later in [WAIT + FALL + 0.01, 60., 3600.] {
            assert!(!sky.falling(at(up, later)));
            assert_eq!(sky.until_fall(at(up, later)), None);
        }
    }

    #[test]
    fn it_waits_for_the_star_with_one_wake() {
        let up = Instant::now();
        let sky = Sky::new(up);
        let wait = sky.until_fall(at(up, 1.)).expect("still to come");
        assert!((wait.as_secs_f32() - (WAIT - 1.)).abs() < 1e-3);
    }

    #[test]
    fn a_star_in_the_background_is_never_shown_half_way() {
        let up = Instant::now();
        let mut sky = Sky::new(up);
        sky.let_go(at(up, WAIT + 0.3));
        assert!(sky.fall(at(up, WAIT + 0.4)).is_none());
        assert_eq!(sky.until_fall(at(up, 0.)), None);
    }

    #[test]
    fn a_fall_goes_down_fades_in_and_out_and_ends() {
        let up = Instant::now();
        let sky = Sky::new(up);
        let heads: Vec<(f32, f32)> = (0..=66)
            .filter_map(|frame| sky.fall(at(up, WAIT + frame as f32 / 60.)))
            .map(|(fall, dots)| {
                assert!((0.15..=0.85).contains(&fall.x));
                assert!(
                    dots.iter()
                        .all(|d| (0. ..=1.).contains(&d.alpha) && d.size > 0.)
                );
                assert!(dots.windows(2).all(|w| w[0].alpha >= w[1].alpha));
                (dots[0].y, dots[0].alpha)
            })
            .collect();
        assert!(
            heads.windows(2).all(|w| w[1].0 >= w[0].0),
            "always downwards"
        );
        assert!(heads.first().unwrap().1 < 0.2 && heads.last().unwrap().1 < 0.2);
        assert!(heads.iter().any(|h| h.1 > 0.9));
    }

    #[test]
    fn the_star_heads_away_from_the_nearer_side_and_down() {
        assert!((STAR.dx * STAR.dx + STAR.dy * STAR.dy - 1.).abs() < 1e-4);
        assert!(STAR.x < 0.5 && STAR.dx > 0. && STAR.dy > 0.);
    }
}
