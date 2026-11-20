use crate::theme::{Accent, Tone};

pub struct Mail {
    pub sender: &'static str,
    pub initials: &'static str,
    pub tone: Tone,
    pub subject: &'static str,
    pub snippet: &'static str,
    pub time: &'static str,
    pub thread: Option<&'static str>,
    pub unread: bool,
    pub tag: Option<(&'static str, Accent)>,
    pub yesterday: bool,
    pub brief: &'static str,
    pub body: &'static [&'static str],
    /// (already typed, AI suggestion shown as ghost text)
    pub draft: (&'static str, &'static str),
    /// The hero thread shows action items, earlier messages and attachments.
    pub rich: bool,
}

const fn mail(
    sender: &'static str,
    initials: &'static str,
    tone: Tone,
    subject: &'static str,
    snippet: &'static str,
    time: &'static str,
) -> Mail {
    Mail {
        sender,
        initials,
        tone,
        subject,
        snippet,
        time,
        thread: None,
        unread: false,
        tag: None,
        yesterday: false,
        brief: snippet,
        body: &[],
        draft: ("", ""),
        rich: false,
    }
}

pub fn inbox() -> Vec<Mail> {
    vec![
        Mail {
            thread: Some("3"),
            unread: true,
            tag: Some(("Needs reply", Accent::Rose)),
            rich: true,
            brief: "Mira needs your sign-off by Friday. Public launch moves to Dec 9 so onboarding gets one more test round; budget is unchanged.",
            body: &[
                "Hi Luan,",
                "Final deck is attached with the updated timeline. We pushed the public launch to December 9 so the new onboarding flow gets one more round of testing. Budget stays exactly where it was.",
                "Two things before Friday: the pricing tiers on slide 14, and a date for the press embargo.",
                "Thanks,\nMira",
            ],
            draft: (
                "Looks great, approved for Dec 9.",
                " Let’s lock the embargo for Dec 4 and I’ll confirm the tiers by Thursday.",
            ),
            ..mail(
                "Mira Kovač",
                "MK",
                Tone::Violet,
                "Q4 launch plan, final review",
                "Attached the deck with the updated timeline. Can you sign off before Friday?",
                "9:41",
            )
        },
        Mail {
            unread: true,
            brief: "Your November payout of €24,180.00 lands on Nov 22. Nothing to do.",
            body: &[
                "Hi Luan,",
                "Your payout of €24,180.00 is on its way and should arrive on November 22 in the account ending 4421.",
                "You can see the full breakdown in your dashboard.",
            ],
            ..mail(
                "Stripe",
                "S",
                Tone::Graphite,
                "November payout",
                "€24,180.00 arrives Nov 22 in the account ending 4421.",
                "9:12",
            )
        },
        Mail {
            thread: Some("5"),
            unread: true,
            brief: "Jonah finished the component audit. Two focus-ring edge cases in dark mode need a decision; everything else passes.",
            body: &[
                "Hey,",
                "Went through Button and Input end to end. Two edge cases with focus rings in dark mode, otherwise solid.",
                "I can pair on it tomorrow morning if that works.",
            ],
            draft: ("Tomorrow works.", " 10:00 at my desk?"),
            ..mail(
                "Jonah Reyes",
                "JR",
                Tone::Mint,
                "Component audit",
                "Two edge cases with focus rings in dark mode, otherwise solid.",
                "8:47",
            )
        },
        Mail {
            tag: Some(("Travel", Accent::Sky)),
            brief: "Flights for the Lisbon offsite are booked. Everyone lands Thursday afternoon; hotel confirmation is attached.",
            body: &[
                "Hi all,",
                "Flights are booked. Everyone lands Thursday afternoon and the hotel confirmation is inside.",
                "Dinner on the first night is at 8:30, a short walk from the hotel.",
            ],
            ..mail(
                "Ana Petrović",
                "AP",
                Tone::Amber,
                "Lisbon offsite",
                "Flights booked. Everyone lands Thursday afternoon.",
                "7:30",
            )
        },
        Mail {
            brief: "Three issues were assigned to you: fuzzy matching in the command palette, snooze presets, and a keyboard hint bug.",
            body: &[
                "NIM-214 Command palette fuzzy matching",
                "NIM-219 Snooze presets",
                "NIM-223 Keyboard hints overlap on narrow windows",
            ],
            ..mail(
                "Linear",
                "L",
                Tone::Sky,
                "3 issues assigned",
                "NIM-214 Command palette fuzzy matching, NIM-219 Snooze presets",
                "7:02",
            )
        },
        Mail {
            thread: Some("2"),
            yesterday: true,
            body: &[
                "Pulled a few references for the aurora wallpaper and the serif. The italic accent on the last words of a subject line is my favourite detail.",
                "Have a look when you get a minute.",
            ],
            ..mail(
                "Elena Brooks",
                "EB",
                Tone::Rose,
                "Nimbra moodboard",
                "Pulled a few references for the aurora wallpaper and the serif.",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: &["Clause 4.2 is updated per legal. Ready to countersign when you are."],
            ..mail(
                "Daniel Osei",
                "DO",
                Tone::Graphite,
                "Contract v3",
                "Clause 4.2 updated per legal. Ready to countersign when you are.",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: &["“Can the unread counts be a touch quieter?”"],
            ..mail(
                "Figma",
                "F",
                Tone::Violet,
                "Elena mentioned you",
                "“Can the unread counts be a touch quieter?”",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: &["Design crit is now Thursday 2:00 PM in Studio B. Agenda attached."],
            ..mail(
                "Google Calendar",
                "GC",
                Tone::Sky,
                "Design crit moved",
                "Now Thursday 2:00 PM, Studio B. Agenda attached.",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: &["12 pages updated and 4 new comments in Roadmap 2027."],
            ..mail(
                "Notion",
                "N",
                Tone::Amber,
                "Weekly digest",
                "12 pages updated, 4 new comments in Roadmap 2027.",
                "Tue",
            )
        },
    ]
}

/// (icon, label, hint, key, section header shown above this row)
pub const COMMANDS: &[(&str, &str, &str, &str, Option<&str>)] = &[
    (
        "sparkles",
        "Snooze until I’m free",
        "After Design crit · Thu 3:30 PM",
        "↵",
        Some("SMART SNOOZE"),
    ),
    (
        "bell",
        "Until Mira replies",
        "Returns to Important automatically",
        "R",
        None,
    ),
    ("clock", "Later today", "Thu 6:00 PM", "H", Some("SNOOZE")),
    ("clock", "Tomorrow morning", "Fri 9:00 AM", "T", None),
    ("calendar", "Next week", "Mon 9:00 AM", "W", None),
    (
        "zap",
        "Pick a date & time…",
        "Try “next tue 3pm”",
        "/",
        None,
    ),
];
