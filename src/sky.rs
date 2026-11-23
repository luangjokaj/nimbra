//! The night over Inbox Zero once it has come up: the stars twinkle, and now and then one
//! falls. Driven by drawn frames, like `arrival.rs`, so it stops wherever it is not drawn.

use std::f32::consts::TAU;

/// How long a falling star takes to cross, in seconds.
const FALL: f32 = 1.1;
/// How far it goes, and its longest tail, in points.
const TRAVEL: f32 = 260.;
const TAIL: f32 = 120.;
/// The dots its tail is drawn with (GPUI draws no slanted lines).
const DOTS: usize = 90;
/// The most one frame moves it on.
const LONGEST_STEP: f32 = 0.1;
/// The twinkling grows in over this long, so the field does not jump once it has come up.
const EASE_IN: f32 = 2.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fall {
    /// When it started, in the sky's seconds.
    start: f32,
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
    seconds: f32,
    seed: u32,
    /// When the next star falls.
    next: f32,
    fall: Option<Fall>,
}

impl Default for Sky {
    fn default() -> Self {
        Self::new()
    }
}

impl Sky {
    pub fn new() -> Self {
        Sky {
            seconds: 0.,
            seed: 0x2545_f491,
            // The first comes soon, so it is seen.
            next: 3.5,
            fall: None,
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift: fixed, so every night is the same and tests can follow it.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.
    }

    /// A frame later.
    pub fn advance(&mut self, frame: f32) {
        self.seconds += frame.clamp(0., LONGEST_STEP);
        if self.fall.is_some_and(|f| self.seconds - f.start >= FALL) {
            self.fall = None;
        }
        if self.fall.is_none() && self.seconds >= self.next {
            let x = 0.15 + self.random() * 0.7;
            let y = 0.03 + self.random() * 0.2;
            // 18 to 38 degrees below level, away from the nearer side.
            let angle = (18. + self.random() * 20.).to_radians();
            let side = if x < 0.5 { 1. } else { -1. };
            self.fall = Some(Fall {
                start: self.seconds,
                x,
                y,
                dx: side * angle.cos(),
                dy: angle.sin(),
            });
            self.next = self.seconds + 8. + self.random() * 14.;
        }
    }

    /// A star is falling: it needs every frame until it is gone.
    pub fn falling(&self) -> bool {
        self.fall.is_some()
    }

    /// Lets go of one that is falling (the window went inactive): no star stops mid-flight.
    pub fn drop_fall(&mut self) {
        self.fall = None;
    }

    /// How bright star `index` is now, as a share of its own brightness. About a third
    /// hold still; the rest breathe, each at its own pace.
    pub fn twinkle(&self, index: usize) -> f32 {
        let hash = (index as u32).wrapping_mul(2_654_435_761);
        if hash.is_multiple_of(3) {
            return 1.;
        }
        let period = 2.2 + ((hash >> 8) % 100) as f32 / 100. * 3.8;
        let phase = ((hash >> 16) % 628) as f32 / 100.;
        let wave = 0.5 + 0.5 * (TAU * self.seconds / period + phase).sin();
        let depth = 0.65 * (self.seconds / EASE_IN).min(1.);
        // Dimming briefly, bright most of the time.
        1. - depth * wave * wave * wave
    }

    /// The falling star now: where it started, and its dots from the head back.
    pub fn fall(&self) -> Option<(Fall, Vec<Dot>)> {
        let fall = self.fall?;
        let age = ((self.seconds - fall.start) / FALL).clamp(0., 1.);
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
                    x: fall.dx * along,
                    y: fall.dy * along,
                    size,
                    alpha,
                }
            })
            .collect();
        Some((fall, dots))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twinkling_stays_in_range_and_starts_still() {
        let mut sky = Sky::new();
        assert!((0..48).all(|i| sky.twinkle(i) == 1.));
        for _ in 0..2000 {
            sky.advance(1. / 60.);
            for i in 0..48 {
                let b = sky.twinkle(i);
                assert!((0.34..=1.).contains(&b), "{b}");
            }
        }
        // Some do breathe.
        assert!((0..48).any(|i| sky.twinkle(i) < 0.9));
    }

    #[test]
    fn stars_fall_now_and_then_and_never_two_at_once() {
        let mut sky = Sky::new();
        let mut starts = Vec::new();
        let mut was = false;
        for _ in 0..(120 * 60) {
            sky.advance(1. / 60.);
            let now = sky.falling();
            if now && !was {
                starts.push(sky.seconds);
            }
            was = now;
        }
        // Two minutes: the first early, then every 8 to 22 seconds.
        assert!(starts[0] < 4.);
        assert!((5..=15).contains(&starts.len()), "{starts:?}");
        for pair in starts.windows(2) {
            let gap = pair[1] - pair[0];
            assert!((8. - 0.1..=22. + 0.1).contains(&gap), "{gap}");
        }
    }

    #[test]
    fn a_fall_goes_down_fades_in_and_out_and_ends() {
        let mut sky = Sky::new();
        while !sky.falling() {
            sky.advance(1. / 60.);
        }
        let mut heads = Vec::new();
        while let Some((fall, dots)) = sky.fall() {
            assert!((0.15..=0.85).contains(&fall.x) && (0.03..=0.23).contains(&fall.y));
            assert!(
                dots.iter()
                    .all(|d| d.alpha >= 0. && d.alpha <= 1. && d.size > 0.)
            );
            assert!(dots.windows(2).all(|w| w[0].alpha >= w[1].alpha));
            heads.push((dots[0].y, dots[0].alpha));
            sky.advance(1. / 60.);
        }
        assert!(
            heads.windows(2).all(|w| w[1].0 >= w[0].0),
            "always downwards"
        );
        assert!(heads.first().unwrap().1 < 0.2 && heads.last().unwrap().1 < 0.2);
        assert!(heads.iter().any(|h| h.1 > 0.9));
        assert!(heads.len() as f32 / 60. <= FALL + 0.05);
    }

    #[test]
    fn a_long_pause_does_not_skip_a_fall() {
        let mut sky = Sky::new();
        sky.advance(30.);
        assert!(sky.seconds <= LONGEST_STEP);
    }
}
