//! The night over Inbox Zero once it has come up: now and then a star falls. The stars
//! themselves hold still, so nothing is drawn between falls; a fall follows real time,
//! frame by frame, and is let go wherever the window is not in front.

use std::time::Instant;

/// How long a falling star takes to cross, in seconds.
const FALL: f32 = 1.1;
/// How far it goes, and its longest tail, in points.
const TRAVEL: f32 = 260.;
const TAIL: f32 = 120.;
/// The dots its tail is drawn with (GPUI draws no slanted lines).
const DOTS: usize = 90;
/// The first comes soon after the sky is up, or after the window is back in front, so it
/// is seen.
const FIRST: f32 = 3.5;

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
    born: Instant,
    seed: u32,
    /// When the next star falls, in the sky's seconds.
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
            born: Instant::now(),
            seed: 0x2545_f491,
            next: FIRST,
            fall: None,
        }
    }

    /// The sky's seconds now.
    pub fn now(&self) -> f32 {
        self.born.elapsed().as_secs_f32()
    }

    fn random(&mut self) -> f32 {
        // xorshift: fixed, so every night is the same and tests can follow it.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.
    }

    /// Ends a fall that is over by `now`, and says whether the next one is due.
    pub fn due(&mut self, now: f32) -> bool {
        if self.fall.is_some_and(|f| now - f.start >= FALL) {
            self.fall = None;
        }
        self.fall.is_none() && now >= self.next
    }

    /// Lets a star fall from `now`, however late it is: a pause never skips one or starts
    /// it halfway.
    pub fn start(&mut self, now: f32) {
        let x = 0.15 + self.random() * 0.7;
        let y = 0.03 + self.random() * 0.2;
        // 18 to 38 degrees below level, away from the nearer side.
        let angle = (18. + self.random() * 20.).to_radians();
        let side = if x < 0.5 { 1. } else { -1. };
        self.fall = Some(Fall {
            start: now,
            x,
            y,
            dx: side * angle.cos(),
            dy: angle.sin(),
        });
        self.next = now + 8. + self.random() * 14.;
    }

    /// A star is falling: it needs every frame until it is gone.
    pub fn falling(&self) -> bool {
        self.fall.is_some()
    }

    /// How long until the next star, when none is falling.
    pub fn wait(&self, now: f32) -> Option<f32> {
        (!self.falling()).then(|| (self.next - now).max(0.))
    }

    /// The window went to the background: no star stops mid-flight, and the next comes
    /// a little after it is back.
    pub fn rest(&mut self, now: f32) {
        self.fall = None;
        self.next = now + FIRST;
    }

    /// The falling star at `now`: where it started, and its dots from the head back.
    pub fn fall(&self, now: f32) -> Option<(Fall, Vec<Dot>)> {
        let fall = self.fall?;
        let age = ((now - fall.start) / FALL).clamp(0., 1.);
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

    /// Runs the sky as the app does, at `hz` frames a second while a star falls, and
    /// jumping straight to the next one in between. When each fall started.
    fn falls(hz: f32, seconds: f32) -> (Sky, Vec<f32>, usize) {
        let mut sky = Sky::new();
        let mut now = 0.;
        let mut starts = Vec::new();
        let mut frames = 0;
        while now < seconds {
            if sky.due(now) {
                sky.start(now);
                starts.push(now);
            }
            if sky.falling() {
                frames += 1;
                now += 1. / hz;
            } else {
                now += sky.wait(now).unwrap().max(1e-3);
            }
        }
        (sky, starts, frames)
    }

    #[test]
    fn stars_fall_now_and_then_never_two_at_once_and_nothing_is_drawn_between() {
        let (_, starts, frames) = falls(60., 120.);
        assert!(starts[0] < 4.);
        assert!((5..=15).contains(&starts.len()), "{starts:?}");
        for pair in starts.windows(2) {
            let gap = pair[1] - pair[0];
            assert!((8. - 0.1..=22. + 0.1).contains(&gap), "{gap}");
        }
        // Frames only while falling: about 1.1 s at 60 Hz per star.
        let per_fall = frames as f32 / starts.len() as f32;
        assert!(
            (60. * FALL - 2.0..=60. * FALL + 2.).contains(&per_fall),
            "{per_fall}"
        );
    }

    #[test]
    fn a_fall_takes_the_same_time_at_60_and_120_hz() {
        let (_, a, frames_60) = falls(60., 30.);
        let (_, b, frames_120) = falls(120., 30.);
        assert_eq!(a.len(), b.len());
        let ratio = frames_120 as f32 / frames_60 as f32;
        assert!((1.9..=2.1).contains(&ratio), "{ratio}");
    }

    #[test]
    fn a_fall_goes_down_fades_in_and_out_and_ends() {
        let mut sky = Sky::new();
        let start = sky.wait(0.).unwrap();
        assert!(sky.due(start));
        sky.start(start);
        let mut heads = Vec::new();
        let mut now = start;
        while !sky.due(now) && sky.falling() {
            let (fall, dots) = sky.fall(now).unwrap();
            assert!((0.15..=0.85).contains(&fall.x) && (0.03..=0.23).contains(&fall.y));
            assert!(
                dots.iter()
                    .all(|d| d.alpha >= 0. && d.alpha <= 1. && d.size > 0.)
            );
            assert!(dots.windows(2).all(|w| w[0].alpha >= w[1].alpha));
            heads.push((dots[0].y, dots[0].alpha));
            now += 1. / 120.;
        }
        assert!(!sky.falling());
        assert!(
            heads.windows(2).all(|w| w[1].0 >= w[0].0),
            "always downwards"
        );
        assert!(heads.first().unwrap().1 < 0.2 && heads.last().unwrap().1 < 0.2);
        assert!(heads.iter().any(|h| h.1 > 0.9));
        assert!(heads.len() as f32 / 120. <= FALL + 0.02);
    }

    #[test]
    fn a_long_pause_starts_the_next_fall_from_its_beginning() {
        let mut sky = Sky::new();
        assert!(sky.due(600.));
        sky.start(600.);
        let (_, dots) = sky.fall(600.).unwrap();
        assert!(dots[0].alpha < 0.01, "not halfway through");
    }

    #[test]
    fn resting_drops_the_fall_and_waits_again() {
        let mut sky = Sky::new();
        sky.start(5.);
        sky.rest(5.5);
        assert!(!sky.falling());
        assert_eq!(sky.wait(5.5), Some(FIRST));
        assert!(!sky.due(5.5 + FIRST - 0.1));
    }
}
