//! Quitting is put off once when it would lose something that could not be written down
//! (the keychain refused the store's key). Quit again, soon after, quits.
//!
//! GPUI has no say in quitting: its app delegate lacks `applicationShouldTerminate:`, so
//! that is added. macOS asks it for ⌘Q, the Dock and logging out alike. Elsewhere nothing
//! is put off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long after being put off quitting is meant.
const MEANT: Duration = Duration::from_secs(20);

static WOULD_LOSE: AtomicBool = AtomicBool::new(false);
static PUT_OFF: AtomicBool = AtomicBool::new(false);
static ASKED: Mutex<Option<Instant>> = Mutex::new(None);

/// Whether quitting now would lose something.
pub fn set_would_lose(would_lose: bool) {
    WOULD_LOSE.store(would_lose, Ordering::Relaxed);
}

/// Whether quitting was put off since this was last asked.
pub fn take_put_off() -> bool {
    PUT_OFF.swap(false, Ordering::Relaxed)
}

fn may_quit() -> bool {
    let mut asked = ASKED.lock().unwrap_or_else(PoisonError::into_inner);
    let quit = allowed(WOULD_LOSE.load(Ordering::Relaxed), *asked, Instant::now());
    if !quit {
        *asked = Some(Instant::now());
        PUT_OFF.store(true, Ordering::Relaxed);
        crate::checking::OUTSIDE.ring();
    }
    quit
}

fn allowed(would_lose: bool, put_off_at: Option<Instant>, now: Instant) -> bool {
    !would_lose || put_off_at.is_some_and(|at| now.duration_since(at) < MEANT)
}

/// Has to run before the app does: AppKit looks up what its delegate answers to when it
/// is given one.
pub fn install() {
    platform::install();
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod platform {
    use objc::runtime::{Class, Imp, Object, Sel, class_addMethod};
    use objc::{sel, sel_impl};

    const TERMINATE_CANCEL: usize = 0;
    const TERMINATE_NOW: usize = 1;

    extern "C" fn should_terminate(_: &Object, _: Sel, _: *mut Object) -> usize {
        if super::may_quit() {
            TERMINATE_NOW
        } else {
            TERMINATE_CANCEL
        }
    }

    pub fn install() {
        let Some(delegate) = Class::get("GPUIApplicationDelegate") else {
            return;
        };
        // SAFETY: on the main thread before the app runs. The function has the delegate
        // method's signature, NSUInteger (id, SEL, id), and only ever adds: a delegate that
        // has the method keeps its own.
        unsafe {
            let imp = std::mem::transmute::<extern "C" fn(&Object, Sel, *mut Object) -> usize, Imp>(
                should_terminate,
            );
            class_addMethod(
                delegate as *const Class as *mut Class,
                sel!(applicationShouldTerminate:),
                imp,
                c"Q@:@".as_ptr(),
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn install() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quitting_is_put_off_once_and_only_when_it_would_lose_something() {
        let now = Instant::now();
        assert!(allowed(false, None, now));
        assert!(!allowed(true, None, now));
        assert!(allowed(true, Some(now), now + Duration::from_secs(2)));
        assert!(!allowed(true, Some(now), now + MEANT));
    }
}
