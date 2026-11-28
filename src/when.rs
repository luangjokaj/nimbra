//! Dates and times as the Mac is set to show them: its time zone, its 12- or 24-hour clock
//! and its language and region (System Settings › General › Language & Region, Date &
//! Time). Each pattern is a template, such as "jmm" (hour and minute, in the hour cycle the
//! Mac uses); macOS orders and spells it for the user's region. Elsewhere a fixed fallback.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};

/// What to show of a time: a template for macOS and the format used without it.
#[derive(Clone, Copy)]
pub struct Pattern {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    template: &'static str,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fallback: &'static str,
}

/// "9:41 PM" or "21:41".
pub const TIME: Pattern = Pattern {
    template: "jmm",
    fallback: "%H:%M",
};
/// "9 PM" or "21".
pub const HOUR: Pattern = Pattern {
    template: "j",
    fallback: "%H",
};
/// "Fri".
pub const WEEKDAY: Pattern = Pattern {
    template: "EEE",
    fallback: "%a",
};
/// "Nov 21".
pub const DAY_MONTH: Pattern = Pattern {
    template: "MMMd",
    fallback: "%b %-d",
};
/// "Fri 9:41 PM".
pub const WEEKDAY_TIME: Pattern = Pattern {
    template: "EEEjmm",
    fallback: "%a, %H:%M",
};
/// "Fri, Nov 21, 9:41 PM".
pub const WEEKDAY_DAY_TIME: Pattern = Pattern {
    template: "EEEMMMdjmm",
    fallback: "%a, %b %-d, %H:%M",
};
/// "Friday, November 21".
pub const FULL_DAY: Pattern = Pattern {
    template: "EEEEMMMMd",
    fallback: "%A, %B %-d",
};
/// "November 2026".
pub const MONTH_YEAR: Pattern = Pattern {
    template: "MMMMy",
    fallback: "%B %Y",
};
/// "Fri, Nov 21, 2026 at 9:41 PM".
pub const LONG: Pattern = Pattern {
    template: "EEEMMMdyjmm",
    fallback: "%a, %b %-d, %Y, %H:%M",
};

/// A moment, in the Mac's time zone.
pub fn local(when: DateTime<Local>, pattern: Pattern) -> String {
    #[cfg(target_os = "macos")]
    {
        mac::format(when.timestamp_millis(), pattern.template)
    }
    #[cfg(not(target_os = "macos"))]
    {
        when.format(pattern.fallback).to_string()
    }
}

/// A moment by its milliseconds since 1970; empty when it is not one.
pub fn millis(timestamp_ms: i64, pattern: Pattern) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|when| local(when, pattern))
        .unwrap_or_default()
}

/// A day, at its noon (so no time zone moves it to another day).
pub fn day(date: NaiveDate, pattern: Pattern) -> String {
    date.and_hms_opt(12, 0, 0)
        .and_then(|noon| Local.from_local_datetime(&noon).earliest())
        .map(|when| local(when, pattern))
        .unwrap_or_default()
}

/// A time of day, today.
pub fn clock(hour: u32, minute: u32, pattern: Pattern) -> String {
    NaiveTime::from_hms_opt(hour, minute, 0)
        .and_then(|time| {
            Local
                .from_local_datetime(&Local::now().date_naive().and_time(time))
                .earliest()
        })
        .map(|when| local(when, pattern))
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::{CStr, CString};
    use std::time::{Duration, Instant};

    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    /// How long a formatter is kept: one made again picks up a changed setting (the clock,
    /// the language) without a relaunch.
    const KEPT: Duration = Duration::from_secs(60);

    struct Kept {
        formatter: *mut Object,
        made: Instant,
    }

    impl Drop for Kept {
        fn drop(&mut self) {
            // SAFETY: a formatter this thread made with `new`, released once.
            unsafe {
                let _: () = msg_send![self.formatter, release];
            }
        }
    }

    thread_local! {
        /// One formatter per template on each thread (they are costly to make).
        static FORMATTERS: RefCell<HashMap<&'static str, Kept>> = RefCell::new(HashMap::new());
    }

    pub fn format(timestamp_ms: i64, template: &'static str) -> String {
        FORMATTERS.with(|formatters| {
            let mut formatters = formatters.borrow_mut();
            if formatters
                .get(template)
                .is_none_or(|kept| kept.made.elapsed() > KEPT)
            {
                // SAFETY: Foundation's NSDateFormatter, made and used on this thread only.
                let Some(formatter) = (unsafe { make(template) }) else {
                    return String::new();
                };
                formatters.insert(
                    template,
                    Kept {
                        formatter,
                        made: Instant::now(),
                    },
                );
            }
            let Some(kept) = formatters.get(template) else {
                return String::new();
            };
            // SAFETY: a live formatter of this thread; what Foundation hands back is
            // autoreleased into the pool drained here.
            unsafe { text(kept.formatter, timestamp_ms) }
        })
    }

    /// A formatter for `template` in the user's language, region, clock and time zone,
    /// which follow the system as it changes.
    unsafe fn make(template: &str) -> Option<*mut Object> {
        unsafe {
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let formatter: *mut Object = msg_send![class!(NSDateFormatter), new];
            if formatter.is_null() {
                let _: () = msg_send![pool, drain];
                return None;
            }
            let locale: *mut Object = msg_send![class!(NSLocale), autoupdatingCurrentLocale];
            let _: () = msg_send![formatter, setLocale: locale];
            let zone: *mut Object = msg_send![class!(NSTimeZone), localTimeZone];
            let _: () = msg_send![formatter, setTimeZone: zone];
            let template = CString::new(template).unwrap_or_default();
            let template: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: template.as_ptr()];
            let _: () = msg_send![formatter, setLocalizedDateFormatFromTemplate: template];
            let _: () = msg_send![pool, drain];
            Some(formatter)
        }
    }

    unsafe fn text(formatter: *mut Object, timestamp_ms: i64) -> String {
        unsafe {
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let seconds = timestamp_ms as f64 / 1000.;
            let date: *mut Object =
                msg_send![class!(NSDate), dateWithTimeIntervalSince1970: seconds];
            let string: *mut Object = msg_send![formatter, stringFromDate: date];
            let text = if string.is_null() {
                String::new()
            } else {
                let utf8: *const std::ffi::c_char = msg_send![string, UTF8String];
                if utf8.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(utf8).to_string_lossy().into_owned()
                }
            };
            let _: () = msg_send![pool, drain];
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_gives_some_text() {
        let when = Local::now();
        for pattern in [
            TIME,
            HOUR,
            WEEKDAY,
            DAY_MONTH,
            WEEKDAY_TIME,
            WEEKDAY_DAY_TIME,
            FULL_DAY,
            MONTH_YEAR,
            LONG,
        ] {
            assert!(!local(when, pattern).trim().is_empty());
        }
        assert_eq!(millis(i64::MAX, TIME), "");
    }

    #[test]
    fn a_time_shows_its_hour_and_minutes() {
        let text = clock(21, 41, TIME);
        assert!(text.contains("41"), "{text}");
        assert!(text.contains('9') || text.contains("21"), "{text}");
    }
}
