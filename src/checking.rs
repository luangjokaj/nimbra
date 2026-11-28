//! Waiting for Gmail: a check belongs to the mailbox it was started for, and nothing that
//! shows as loading waits for ever.

use crate::mail::View;
use anyhow::Result;
use gpui::{BackgroundExecutor, FutureExt as _};
use std::future::Future;
use std::time::Duration;

/// Each request has a timeout of its own, but not while the address is being looked up,
/// and one check makes dozens of them, some tried again.
const LIMIT: Duration = Duration::from_secs(60);

// When a mailbox was last asked about, and whether that still stands: the engine's.
pub use nimbra_core::listing::Asked;

/// Gmail did not answer within `LIMIT`.
#[derive(Debug)]
pub struct TooSlow;

impl std::fmt::Display for TooSlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The mail server took too long to answer. Reload to try again.")
    }
}

impl std::error::Error for TooSlow {}

/// Runs `work` in the background and stops waiting after `LIMIT`. A request that is already
/// out is not interrupted, so this is for reads: a send given up on could still go out.
pub async fn within<T: Send + 'static>(
    executor: &BackgroundExecutor,
    work: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    executor
        .spawn(work)
        .with_timeout(LIMIT, executor)
        .await
        .unwrap_or_else(|_| Err(TooSlow.into()))
}

struct Check {
    serial: u64,
    view: View,
    query: String,
    /// Nobody asked for it: it does not show.
    quiet: bool,
}

impl Check {
    fn is_for(&self, view: View, query: &str) -> bool {
        self.view == view && (view.base() != View::Search || self.query == query)
    }
}

/// The newest check. Whether it shows as running follows from what is on screen, so there
/// is no flag for an early return to leave set.
#[derive(Default)]
pub struct Checks {
    last: u64,
    running: Option<Check>,
}

impl Checks {
    /// Starts a check, in place of the one before.
    pub fn start(&mut self, view: View, query: &str) -> u64 {
        self.last += 1;
        self.running = Some(Check {
            serial: self.last,
            view,
            query: query.to_string(),
            quiet: false,
        });
        self.last
    }

    /// The check just started is one nobody asked for.
    pub fn quiet(&mut self) {
        if let Some(check) = &mut self.running {
            check.quiet = true;
        }
    }

    /// Somebody asked while it was running.
    pub fn show(&mut self) {
        if let Some(check) = &mut self.running {
            check.quiet = false;
        }
    }

    /// Whether the screen says that it is checking.
    pub fn shows_for(&self, view: View, query: &str) -> bool {
        self.running
            .as_ref()
            .is_some_and(|check| !check.quiet && check.is_for(view, query))
    }

    pub fn running_for(&self, view: View, query: &str) -> bool {
        self.running
            .as_ref()
            .is_some_and(|check| check.is_for(view, query))
    }

    /// The check has its answer. Whether that is still wanted: it is the newest, and what it
    /// was for is on screen.
    pub fn finish(&mut self, serial: u64, view: View, query: &str) -> bool {
        self.running
            .take_if(|check| check.serial == serial)
            .is_some_and(|check| check.is_for(view, query))
    }

    pub fn forget(&mut self) {
        self.running = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_left_behind_neither_shows_nor_blocks() {
        let mut checks = Checks::default();
        let starred = checks.start(View::Starred, "");
        assert!(checks.running_for(View::Starred, ""));
        assert!(!checks.running_for(View::Inbox, ""));
        // Its answer comes while the Inbox is shown: not used, and no longer running.
        assert!(!checks.finish(starred, View::Inbox, ""));
        assert!(!checks.running_for(View::Starred, ""));
    }

    #[test]
    fn only_the_newest_answer_is_used() {
        let mut checks = Checks::default();
        let first = checks.start(View::Inbox, "");
        let second = checks.start(View::Inbox, "");
        assert!(!checks.finish(first, View::Inbox, ""));
        assert!(checks.running_for(View::Inbox, ""));
        assert!(checks.finish(second, View::Inbox, ""));
    }

    #[test]
    fn a_quiet_check_runs_without_showing_until_asked_for() {
        let mut checks = Checks::default();
        checks.start(View::Inbox, "");
        checks.quiet();
        assert!(checks.running_for(View::Inbox, "") && !checks.shows_for(View::Inbox, ""));
        checks.show();
        assert!(checks.shows_for(View::Inbox, ""));
    }

    #[test]
    fn a_search_is_for_its_words() {
        let mut checks = Checks::default();
        let search = checks.start(View::Search, "alpha");
        assert!(!checks.running_for(View::Search, "beta"));
        assert!(!checks.finish(search, View::Search, "beta"));
        // Every account's search too: another search's answer is not its own.
        let every = View::every(View::Search);
        let search = checks.start(every, "alpha");
        assert!(checks.running_for(every, "alpha"));
        assert!(!checks.running_for(every, "beta"));
        assert!(!checks.finish(search, every, "beta"));
    }

    #[test]
    fn a_forgotten_check_has_no_say() {
        let mut checks = Checks::default();
        let check = checks.start(View::Inbox, "");
        checks.forget();
        assert!(!checks.running_for(View::Inbox, ""));
        assert!(!checks.finish(check, View::Inbox, ""));
    }
}
