//! Inbox Zero coming up as Nimbra opens: the stars come out, the horizon
//! draws out, then the words settle into place. Timed by the wall clock from the first
//! frame it is drawn in, so late frames never slow it down.

use std::time::Instant;

/// How far below their place the words start, in points.
const RISE: f32 = 8.;
/// When the last part is in place, in seconds.
const END: f32 = 1.6;

/// How far along it is. Each part is a share from 0 to 1, and all are 1 once it has settled.
#[derive(Clone, Copy)]
pub struct Arrival {
    /// When it was first drawn; `None` until then.
    start: Option<Instant>,
    seconds: f32,
}

impl Arrival {
    pub const START: Self = Self {
        start: None,
        seconds: 0.,
    };
    pub const SETTLED: Self = Self {
        start: None,
        seconds: END,
    };

    /// As it is at `now`, or `None` once it has settled.
    pub fn after(self, now: Instant) -> Option<Self> {
        let start = self.start.unwrap_or(now);
        let seconds = now.saturating_duration_since(start).as_secs_f32();
        (seconds < END).then_some(Self {
            start: Some(start),
            seconds,
        })
    }

    fn part(self, start: f32, length: f32) -> f32 {
        // Not worked out at the end, so that float rounding cannot leave it short of 1.
        if self.seconds >= END {
            return 1.;
        }
        let left = 1. - ((self.seconds - start) / length).clamp(0., 1.);
        1. - left * left * left
    }

    pub fn star(self, index: usize) -> f32 {
        // Scattered: neighbours in the field do not come out together.
        self.part(0.2 + (index * 29 % 48) as f32 / 48. * 0.8, 0.5)
    }

    pub fn horizon(self) -> f32 {
        self.part(0.2, 1.1)
    }

    pub fn date(self) -> f32 {
        self.part(0.2, 0.7)
    }

    pub fn headline(self) -> f32 {
        self.part(0.3, 0.9)
    }

    pub fn line(self) -> f32 {
        self.part(0.5, 0.8)
    }

    /// What is below the horizon.
    pub fn below(self) -> f32 {
        self.part(0.8, 0.8)
    }
}

/// How far below its place something is that is this far in.
pub fn drop_for(shown: f32) -> f32 {
    RISE * (1. - shown)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn parts(at: Arrival) -> Vec<f32> {
        let mut all = vec![
            at.horizon(),
            at.date(),
            at.headline(),
            at.line(),
            at.below(),
        ];
        all.extend((0..48).map(|i| at.star(i)));
        all
    }

    #[test]
    fn starts_dark_only_moves_forward_and_ends_exactly_in_place() {
        let first = Instant::now();
        let mut at = Arrival::START.after(first).expect("just started");
        let mut last = parts(at);
        assert!(last.iter().all(|part| *part == 0.));
        let mut frame = 1;
        while let Some(next) = at.after(first + Duration::from_secs_f32(frame as f32 / 120.)) {
            let now = parts(next);
            assert!(now.iter().zip(&last).all(|(now, last)| now >= last));
            (at, last, frame) = (next, now, frame + 1);
        }
        // No jump from the last animated frame to the still screen.
        assert!(last.iter().all(|part| *part > 0.999));
        assert!(parts(Arrival::SETTLED).iter().all(|part| *part == 1.));
        assert_eq!(drop_for(1.), 0.);
    }

    #[test]
    fn it_starts_at_its_first_frame_and_keeps_to_the_clock() {
        let first = Instant::now();
        let started = Arrival::START.after(first).expect("just started");
        assert_eq!(started.headline(), 0.);
        // A frame that comes 0.6 s late shows where it is at 0.6 s, not one step on.
        let late = started
            .after(first + Duration::from_millis(600))
            .expect("still arriving");
        assert!((late.seconds - 0.6).abs() < 1e-3);
        assert!(started.after(first + Duration::from_secs(2)).is_none());
    }
}
