//! Inbox Zero coming up as Nimbra opens: the glows rise, the stars come out, the horizon
//! draws out, then the words settle into place.

/// How far below their place the words start, in points.
const RISE: f32 = 8.;
/// The most one frame moves it on, so that a slow first frame does not swallow the start.
const LONGEST_STEP: f32 = 0.05;
/// When the last part is in place, in seconds.
const END: f32 = 1.6;

/// How far along it is. Each part is a share from 0 to 1, and all are 1 once it has settled.
#[derive(Clone, Copy)]
pub struct Arrival {
    seconds: f32,
}

impl Arrival {
    pub const START: Self = Self { seconds: 0. };
    pub const SETTLED: Self = Self { seconds: END };

    /// A frame later, or `None` once it has settled.
    pub fn after(self, frame: f32) -> Option<Self> {
        let seconds = self.seconds + frame.min(LONGEST_STEP);
        (seconds < END).then_some(Self { seconds })
    }

    fn part(self, start: f32, length: f32) -> f32 {
        // Not worked out at the end, so that float rounding cannot leave it short of 1.
        if self.seconds >= END {
            return 1.;
        }
        let left = 1. - ((self.seconds - start) / length).clamp(0., 1.);
        1. - left * left * left
    }

    pub fn glow(self, index: usize) -> f32 {
        self.part(index as f32 * 0.08, 1.2)
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
    use super::*;

    fn parts(at: Arrival) -> Vec<f32> {
        let mut all = vec![
            at.horizon(),
            at.date(),
            at.headline(),
            at.line(),
            at.below(),
        ];
        all.extend((0..4).map(|i| at.glow(i)));
        all.extend((0..48).map(|i| at.star(i)));
        all
    }

    #[test]
    fn starts_dark_only_moves_forward_and_ends_exactly_in_place() {
        let mut at = Arrival::START;
        let mut last = parts(at);
        assert!(last.iter().all(|part| *part == 0.));
        while let Some(next) = at.after(1. / 120.) {
            let now = parts(next);
            assert!(now.iter().zip(&last).all(|(now, last)| now >= last));
            (at, last) = (next, now);
        }
        // No jump from the last animated frame to the still screen.
        assert!(last.iter().all(|part| *part > 0.999));
        assert!(parts(Arrival::SETTLED).iter().all(|part| *part == 1.));
        assert_eq!(drop_for(1.), 0.);
    }

    #[test]
    fn a_slow_frame_does_not_swallow_it() {
        let next = Arrival::START.after(3.).expect("still arriving");
        assert!(next.headline() == 0.);
    }
}
