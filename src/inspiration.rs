//! The line below Inbox Zero's horizon when there is nothing to undo: one for each day.

use chrono::{Datelike, NaiveDate};

/// Not a multiple of seven, so that a line does not always land on the same weekday.
const LINES: [&str; 15] = [
    "The best ideas rarely arrive by email.",
    "Go look at the sky. It’s been working on something.",
    "Call someone you’d normally email.",
    "Take the long way to get coffee.",
    "Follow the thought you didn’t have time for.",
    "Start it badly. You can fix it later.",
    "Read ten pages of something nobody sent you.",
    "Walk until you stop writing replies in your head.",
    "Draw it before you explain it.",
    "Water the plants. They never write, but they notice.",
    "Open a window and do nothing for a minute.",
    "Learn the name of one tree on your street.",
    "Go be hard to reach for a while.",
    "Somewhere nearby, a dog would love to meet you.",
    "Write the first line. Only the first line.",
];

pub fn line_for(day: NaiveDate) -> &'static str {
    LINES[day.num_days_from_ce().rem_euclid(LINES.len() as i32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_gets_its_day_before_any_comes_back() {
        let first = NaiveDate::from_ymd_opt(2026, 12, 25).expect("a real date");
        let shown: Vec<_> = first.iter_days().take(LINES.len()).map(line_for).collect();
        let distinct: std::collections::HashSet<_> = shown.iter().collect();
        assert_eq!(distinct.len(), LINES.len());
        let again = first + chrono::Days::new(LINES.len() as u64);
        assert_eq!(line_for(again), shown[0]);
    }
}
