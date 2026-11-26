//! Waiting for Gmail: a check belongs to the mailbox it was started for, and nothing that
//! shows as loading waits for ever.

use crate::mail::View;
use anyhow::Result;
use gpui::{BackgroundExecutor, FutureExt as _};
use std::collections::HashSet;
use std::future::Future;
use std::hash::Hash;
use std::time::{Duration, Instant, SystemTime};

/// Each request has a timeout of its own, but not while the address is being looked up,
/// and one check makes dozens of them, some tried again.
const LIMIT: Duration = Duration::from_secs(60);

/// How long what a check found stands, while nothing is seen to change.
const STANDS: Duration = Duration::from_secs(60);

/// When something was asked of Gmail, and how much had been seen to change by then: our own
/// writes and what the change feed brought.
#[derive(Clone, Copy)]
pub struct Asked {
    // Not an `Instant`, which stands still while the Mac sleeps.
    at: SystemTime,
    changes: u64,
}

impl Asked {
    pub fn now(changes: u64) -> Self {
        Self {
            at: SystemTime::now(),
            changes,
        }
    }

    /// Whether asking again now would be for nothing.
    pub fn stands(self, changes: u64) -> bool {
        self.changes == changes && self.at.elapsed().is_ok_and(|age| age < STANDS)
    }
}

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

/// Runs blocking `work` on a thread of its own, for waits that can last long (IMAP IDLE):
/// on the shared executor they would hold up the short jobs queued behind them. `None` when
/// no thread could be started.
pub async fn on_own_thread<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (done, result) = futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _ = done.send(work());
        })
        .ok()?;
    result.await.ok()
}

/// What is on its way, by key: a second request for the same thing waits for the first.
pub struct OnItsWay<K>(HashSet<K>);

impl<K: Eq + Hash> OnItsWay<K> {
    /// Whether to ask: false while the same is already on its way.
    pub fn begin(&mut self, key: K) -> bool {
        self.0.insert(key)
    }

    pub fn end(&mut self, key: &K) {
        self.0.remove(key);
    }
}

impl<K> Default for OnItsWay<K> {
    fn default() -> Self {
        Self(HashSet::new())
    }
}

/// Keys held down step through rows faster than a request each could be answered: a row
/// passed over within `GAP` of the one before waits until the steps stop.
#[derive(Default)]
pub struct Pace {
    last: Option<Instant>,
}

impl Pace {
    pub const GAP: Duration = Duration::from_millis(150);

    /// Whether a request made at `now` goes at once, rather than once the steps stop.
    pub fn at_once(&mut self, now: Instant) -> bool {
        let quick = self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < Self::GAP);
        self.last = Some(now);
        !quick
    }
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
        self.view == view && (view != View::Search || self.query == query)
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
    fn what_was_found_stands_for_a_minute_or_until_a_change() {
        let asked = Asked::now(3);
        assert!(asked.stands(3) && !asked.stands(4));
        let aged = |seconds| Asked {
            at: SystemTime::now() - Duration::from_secs(seconds),
            changes: 3,
        };
        assert!(aged(50).stands(3) && !aged(61).stands(3));
        // The clock was set back.
        let ahead = Asked {
            at: SystemTime::now() + Duration::from_secs(600),
            changes: 3,
        };
        assert!(!ahead.stands(3));
    }

    #[test]
    fn a_search_is_for_its_words() {
        let mut checks = Checks::default();
        let search = checks.start(View::Search, "alpha");
        assert!(!checks.running_for(View::Search, "beta"));
        assert!(!checks.finish(search, View::Search, "beta"));
    }

    #[test]
    fn a_thread_already_on_its_way_is_not_asked_for_again() {
        let mut loads = OnItsWay::default();
        let key = ("t1".to_string(), "acct".to_string());
        assert!(loads.begin(key.clone()));
        assert!(!loads.begin(key.clone()));
        // The same thread of another account is another request.
        assert!(loads.begin(("t1".to_string(), "other".to_string())));
        loads.end(&key);
        assert!(loads.begin(key));
    }

    #[test]
    fn held_keys_wait_and_a_single_step_goes_at_once() {
        let mut pace = Pace::default();
        let start = Instant::now();
        assert!(pace.at_once(start));
        let held = (1..20).map(|n| start + Duration::from_millis(33 * n));
        assert!(held.clone().all(|at| !pace.at_once(at)));
        let after = start + Duration::from_millis(33 * 19) + Pace::GAP;
        assert!(pace.at_once(after));
    }

    #[test]
    fn work_on_its_own_thread_comes_back() {
        let answer = futures::executor::block_on(on_own_thread("test", || 6 * 7));
        assert_eq!(answer, Some(42));
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
