use std::ops::{Deref, DerefMut};

use gpui::SharedString;

use crate::mail;
use crate::rich::{self, Block};
use crate::theme::{Accent, Tone};

// The stored rows are the engine's: both apps list and keep the same ones.
pub use nimbra_core::listing::{Earlier, Header, Row};

/// A list row as the Mac draws it: the engine's `Row` (what is stored), with its avatar,
/// time label and what the sample data shows beside it.
#[derive(Clone, Default)]
pub struct Mail {
    pub row: Row,
    pub initials: SharedString,
    pub tone: Tone,
    pub time: SharedString,
    /// Not from today; shown under the "Earlier" group.
    pub yesterday: bool,
    pub tag: Option<(SharedString, Accent)>,
    /// AI summary. Empty when there is none (real mail has none yet).
    pub brief: SharedString,
    /// The demo hero thread shows action items and attachments.
    pub rich: bool,
    /// The sample data's earlier messages: their avatar colour and time label, which real
    /// mail takes from the sender and the date.
    pub sample: Vec<(Tone, &'static str)>,
}

impl Deref for Mail {
    type Target = Row;
    fn deref(&self) -> &Row {
        &self.row
    }
}

impl DerefMut for Mail {
    fn deref_mut(&mut self) -> &mut Row {
        &mut self.row
    }
}

impl AsRef<Row> for Mail {
    fn as_ref(&self) -> &Row {
        &self.row
    }
}

impl AsMut<Row> for Mail {
    fn as_mut(&mut self) -> &mut Row {
        &mut self.row
    }
}

impl From<Row> for Mail {
    /// A stored or fresh row, with its avatar and the time label for today.
    fn from(row: Row) -> Self {
        let (time, earlier) = format_time(row.timestamp_ms);
        Mail {
            initials: initials(&row.sender).into(),
            tone: tone_for(&row.sender),
            time: time.into(),
            yesterday: earlier,
            row,
            ..Mail::default()
        }
    }
}

impl Mail {
    /// The subject to show: "Untitled" when it has none (a draft just begun, say), also
    /// in place of the "(no subject)" a listing may have put there.
    pub fn shown_subject(&self) -> SharedString {
        match self.subject.trim() {
            "" | "(no subject)" => "Untitled".into(),
            _ => self.subject.clone().into(),
        }
    }

    /// Whether the row names someone: not when it has no name or address (a draft with no
    /// recipients yet), whatever a listing put there instead.
    pub fn names_someone(&self) -> bool {
        !matches!(self.sender.trim(), "" | "Unknown sender")
    }

    /// The name a row shows: in Drafts, "Draft" for one with no recipients yet; elsewhere
    /// "Unknown sender" when there is no name or address.
    pub fn shown_sender(&self, drafts: bool) -> SharedString {
        if self.names_someone() {
            self.sender.trim().to_string().into()
        } else if drafts {
            "Draft".into()
        } else {
            "Unknown sender".into()
        }
    }

    /// A list row for a real thread. The messages are filled in by [`Row::fill`].
    pub fn from_summary(summary: mail::Summary) -> Self {
        Row::from_summary(summary).into()
    }

    /// Recomputes the time label and the Today / Earlier grouping for a stored thread.
    pub fn refresh_time(&mut self) {
        if self.timestamp_ms > 0 {
            let (time, earlier) = format_time(self.timestamp_ms);
            self.time = time.into();
            self.yesterday = earlier;
        }
    }

    /// Whether any message of the thread has HTML of its own, so the original view has
    /// something to show.
    pub fn has_html(&self) -> bool {
        self.html.is_some() || self.earlier.iter().any(|e| e.html.is_some())
    }

    /// An earlier message's avatar colour, initials and time label.
    pub fn earlier_look(&self, k: usize) -> (Tone, SharedString, SharedString) {
        let Some(e) = self.earlier.get(k) else {
            return (
                Tone::Graphite,
                SharedString::default(),
                SharedString::default(),
            );
        };
        let (tone, when) = match self.sample.get(k) {
            Some((tone, when)) => (*tone, when.to_string()),
            None => (tone_for(&e.sender), format_time(e.header.date_ms).0),
        };
        (tone, initials(&e.sender).into(), when.into())
    }
}

/// Combines a freshly fetched mailbox listing with what is already in memory (the
/// engine's `listing::merge`).
pub fn merge(cached: &[Mail], fresh: Vec<Mail>) -> Vec<Mail> {
    nimbra_core::listing::merge(cached, fresh)
}

/// Up to two initials: "Mira Kovač" gives "MK", "stripe" gives "S".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// A stable avatar colour per sender.
pub fn tone_for(name: &str) -> Tone {
    const TONES: [Tone; 6] = [
        Tone::Violet,
        Tone::Rose,
        Tone::Amber,
        Tone::Mint,
        Tone::Sky,
        Tone::Graphite,
    ];
    let hash = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    TONES[hash as usize % TONES.len()]
}

/// A short label for a time, as the Mac shows times (`when`), and whether it falls before
/// today. Within the last day it is the time, as Gmail shows it, then the weekday within
/// the week, then the date.
fn format_time(timestamp_ms: i64) -> (String, bool) {
    use chrono::{Local, TimeZone};
    let Some(when) = Local.timestamp_millis_opt(timestamp_ms).single() else {
        return (String::new(), true);
    };
    let now = Local::now();
    let days = (now.date_naive() - when.date_naive()).num_days();
    let within_a_day = days == 0 || (days == 1 && now - when < chrono::TimeDelta::hours(24));
    let label = match days {
        _ if within_a_day => crate::when::local(when, crate::when::TIME),
        1..=6 => crate::when::local(when, crate::when::WEEKDAY),
        _ => crate::when::local(when, crate::when::DAY_MONTH),
    };
    (label, days != 0)
}

fn mail(
    sender: &'static str,
    initials: &'static str,
    tone: Tone,
    subject: &'static str,
    snippet: &'static str,
    time: &'static str,
) -> Mail {
    Mail {
        row: Row {
            sender: sender.into(),
            subject: subject.into(),
            snippet: snippet.into(),
            participants: format!("{sender} and you"),
            loaded: true,
            ..Row::default()
        },
        initials: initials.into(),
        tone,
        time: time.into(),
        brief: snippet.into(),
        ..Mail::default()
    }
}

fn person(name: &str, address: &str) -> mail::Person {
    mail::Person {
        name: name.into(),
        address: address.into(),
    }
}

/// A sample attachment. Opening it needs an account, which sample mail has not.
fn attachment(name: &str, mime: &str, size: u64) -> mail::Attachment {
    mail::Attachment {
        message_id: String::new(),
        attachment_id: format!("sample-{name}"),
        name: name.into(),
        mime: mime.into(),
        size,
    }
}

fn paragraphs(items: &[&'static str]) -> Vec<Block> {
    rich::from_text(items.iter().map(|p| p.to_string()).collect())
}

/// A formatted message for the sample data.
const SAMPLE_HTML: &str = r#"<html><head><meta name="color-scheme" content="light dark">
                <style>body{font-family:-apple-system,sans-serif;margin:24px}
                @media (prefers-color-scheme: dark){body{background:#22252a;color:#e6e6e6}a{color:#8ab4ff}}</style></head>
                <body><div><p>Hi Luan,</p>
                <p>Pulled a few references for the <b>aurora wallpaper</b> and the serif. The
                <i>italic accent</i> on the last words of a subject line is my favourite detail.</p>
                <h2>What I would keep</h2>
                <ul><li>The violet to rose gradient on the primary button</li>
                <li>Mono for timestamps and <code>keyboard hints</code></li>
                <li>One accent colour per label, nothing more</li></ul>
                <h3>Next steps</h3>
                <ol><li>Review the board: <a href="https://example.com/nimbra-moodboard">nimbra-moodboard</a></li>
                <li>Pick two directions for light mode</li></ol>
                <blockquote><p>Can the unread counts be a touch quieter?</p></blockquote>
                <p>I think that note from last week still stands.</p>
                <img src="https://example.com/aurora-reference.png" alt="Aurora reference" width="560">
                <table width="100%" cellpadding="8"><tr><th align="left">Status</th><th align="left">Direction</th><th align="right">Votes</th></tr>
                <tr><td>Kept</td><td><b>Aurora</b> gradient</td><td align="right">4</td></tr>
                <tr><td>Open</td><td><b>Paper</b> light mode</td><td align="right">2</td></tr></table>
                <hr><pre>--color-accent: #8b7cff;
--color-rose:   #ff7ab6;</pre>
                <p>Thanks,<br>Elena</p></div></body></html>"#;

/// The sample hero thread in HTML, one email per message, each styled its own way, so the
/// original view's messages can be told apart.
const MIRA_FIRST_HTML: &str = r#"<html><head><style>.card{border:1px solid #e3e0f5;border-radius:12px;padding:16px 20px;font-family:Georgia,serif}
                h1{font-size:20px;color:#5b47e0;margin:0 0 8px}</style></head>
                <body><div class="card"><h1>Q4 launch plan: first pass</h1>
                <p>First pass of the deck. Timeline still says <b>Nov 28</b>, fixing now.</p>
                <p>Draft is here: <a href="https://example.com/q4-launch-plan">q4-launch-plan</a></p>
                <p>Mira</p></div></body></html>"#;

const JONAH_HTML: &str = r#"<div style="font-family:-apple-system,sans-serif">
                <p>Numbers look right to me. Looping Luan in for final sign-off.</p>
                <table cellpadding="6" style="border-collapse:collapse"><tr style="background:#eef7f2"><th align="left">Line</th><th align="right">Q4</th></tr>
                <tr><td>Paid ads</td><td align="right">€18,000</td></tr><tr><td>Events</td><td align="right">€9,500</td></tr></table>
                <p>The only open question on my side is the <i>embargo date</i>.</p>
                <p style="color:#6b7280">Jonah Reyes · Northwind Studio</p></div>"#;

const MIRA_LATEST_HTML: &str = r#"<html><head><meta name="color-scheme" content="light dark">
                <style>body{font-family:-apple-system,sans-serif}.note{background:#fff6e5;border-left:3px solid #ffb86b;padding:10px 14px}
                @media (prefers-color-scheme: dark){.note{background:#3a2f1f}}</style></head>
                <body><p>Hi Luan,</p>
                <p>Final deck is attached with the updated timeline. We pushed the public launch to <b>December 9</b> so the new onboarding flow gets one more round of testing. Budget stays exactly where it was.</p>
                <div class="note">Two things before Friday: the pricing tiers on slide 14, and a date for the press embargo.</div>
                <p>Thanks,<br>Mira</p></body></html>"#;

/// The message that started the moodboard thread.
const MOODBOARD_ASK_HTML: &str = r#"<p>Hi Elena,</p>
                <p>Could you pull a few references for the <b>wallpaper</b> and the serif? Anything with an
                aurora feel is welcome.</p><p>Luan</p>"#;

pub fn inbox() -> Vec<Mail> {
    vec![
        {
            let mut m = mail(
                "Mira Kovač",
                "MK",
                Tone::Violet,
                "Q4 launch plan, final review",
                "Attached the deck with the updated timeline. Can you sign off before Friday?",
                "9:41",
            );
            m.thread = Some("3".into());
            m.unread = true;
            m.tag = Some(("Needs reply".into(), Accent::Rose));
            m.rich = true;
            m.header = Header {
                from: person("Mira Kovač", "mira@northwind.studio"),
                to: vec![
                    person("Luan Gjokaj", "luan@example.com"),
                    person("Jonah Reyes", "jonah@northwind.studio"),
                ],
                cc: vec![person("", "launch@northwind.studio")],
                bcc: Vec::new(),
                reply_to: None,
                date_ms: 0,
                attachments: Vec::new(),
                unsubscribe: None,
                message_id: String::new(),
            };
            m.participants = "Mira, Jonah and you · 3 messages".into();
            m.sample = vec![(Tone::Mint, "Yesterday"), (Tone::Violet, "Tue")];
            m.earlier = vec![
                Earlier {
                    sender: "Jonah Reyes".into(),
                    snippet: "Numbers look right to me. Looping Luan in for final sign-off.".into(),
                    blocks: paragraphs(&[
                        "Numbers look right to me. Looping Luan in for final sign-off.",
                        "The only open question on my side is the embargo date.",
                    ]),
                    expanded: false,
                    header: Header {
                        attachments: vec![attachment(
                            "Q4-budget.xlsx",
                            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                            48_200,
                        )],
                        from: person("Jonah Reyes", "jonah@northwind.studio"),
                        to: vec![
                            person("Luan Gjokaj", "luan@example.com"),
                            person("Mira Kovač", "mira@northwind.studio"),
                        ],
                        ..Header::default()
                    },
                    html: Some(JONAH_HTML.into()),
                },
                Earlier {
                    sender: "Mira Kovač".into(),
                    snippet: "First pass of the deck. Timeline still says Nov 28, fixing now."
                        .into(),
                    blocks: paragraphs(&[
                        "First pass of the deck. Timeline still says Nov 28, fixing now.",
                        "Draft is here: https://example.com/q4-launch-plan",
                    ]),
                    expanded: false,
                    header: Header {
                        from: person("Mira Kovač", "mira@northwind.studio"),
                        to: vec![person("Jonah Reyes", "jonah@northwind.studio")],
                        ..Header::default()
                    },
                    html: Some(MIRA_FIRST_HTML.into()),
                },
            ]
            .into();
            m.brief = "Mira needs your sign-off by Friday. Public launch moves to Dec 9 so onboarding gets one more test round; budget is unchanged.".into();
            m.body = paragraphs(&[
                "Hi Luan,",
                "Final deck is attached with the updated timeline. We pushed the public launch to December 9 so the new onboarding flow gets one more round of testing. Budget stays exactly where it was.",
                "Two things before Friday: the pricing tiers on slide 14, and a date for the press embargo.",
                "Thanks,\nMira",
            ]).into();
            m.html = Some(MIRA_LATEST_HTML.into());
            m
        },
        {
            let mut m = mail(
                "Stripe",
                "S",
                Tone::Graphite,
                "November payout",
                "€24,180.00 arrives Nov 22 in the account ending 4421.",
                "9:12",
            );
            m.unread = true;
            m.brief = "Your November payout of €24,180.00 lands on Nov 22. Nothing to do.".into();
            m.body = paragraphs(&[
                "Hi Luan,",
                "Your payout of €24,180.00 is on its way and should arrive on November 22 in the account ending 4421.",
                "You can see the full breakdown in your dashboard.",
            ]).into();
            m
        },
        {
            let mut m = mail(
                "Jonah Reyes",
                "JR",
                Tone::Mint,
                "Component audit",
                "Two edge cases with focus rings in dark mode, otherwise solid.",
                "8:47",
            );
            m.thread = Some("5".into());
            m.unread = true;
            m.brief = "Jonah finished the component audit. Two focus-ring edge cases in dark mode need a decision; everything else passes.".into();
            m.body = paragraphs(&[
                "Hey,",
                "Went through Button and Input end to end. Two edge cases with focus rings in dark mode, otherwise solid.",
                "I can pair on it tomorrow morning if that works.",
            ]).into();
            m
        },
        {
            let mut m = mail(
                "Ana Petrović",
                "AP",
                Tone::Amber,
                "Lisbon offsite",
                "Flights booked. Everyone lands Thursday afternoon.",
                "7:30",
            );
            m.tag = Some(("Travel".into(), Accent::Sky));
            m.brief = "Flights for the Lisbon offsite are booked. Everyone lands Thursday afternoon; hotel confirmation is attached.".into();
            m.body = paragraphs(&[
                "Hi all,",
                "Flights are booked. Everyone lands Thursday afternoon and the hotel confirmation is inside.",
                "Dinner on the first night is at 8:30, a short walk from the hotel.",
            ]).into();
            m
        },
        {
            let mut m = mail(
                "Linear",
                "L",
                Tone::Sky,
                "3 issues assigned",
                "NIM-214 Command palette fuzzy matching, NIM-219 Snooze presets",
                "7:02",
            );
            m.brief = "Three issues were assigned to you: fuzzy matching in the command palette, snooze presets, and a keyboard hint bug.".into();
            m.body = paragraphs(&[
                "NIM-214 Command palette fuzzy matching",
                "NIM-219 Snooze presets",
                "NIM-223 Keyboard hints overlap on narrow windows",
            ])
            .into();
            // An HTML message with no styling of its own, to check the web view's defaults.
            m.html = Some(
                "<p><b>3 issues</b> were assigned to you in <a href=\"https://example.com\">Nimbra</a>.</p>\
                 <ol><li><b>NIM-214.</b> Command palette fuzzy matching, see <code>palette.rs</code></li>\
                 <li><b>NIM-219.</b> Snooze presets</li><li><b>NIM-223.</b> Keyboard hints overlap on narrow windows</li></ol>\
                 <p>Reply to this email directly or view it in Linear.</p>"
                    .into(),
            );
            m
        },
        {
            let mut m = mail(
                "Elena Brooks",
                "EB",
                Tone::Rose,
                "Nimbra moodboard",
                "Pulled a few references for the aurora wallpaper and the serif.",
                "Wed",
            );
            m.thread = Some("2".into());
            m.yesterday = true;
            m.participants = "Elena and you · 2 messages".into();
            m.sample = vec![(Tone::Graphite, "Mon")];
            m.earlier = vec![Earlier {
                sender: "Luan Gjokaj".into(),
                snippet: "Could you pull a few references for the wallpaper?".into(),
                blocks: rich::from_html(MOODBOARD_ASK_HTML),
                expanded: false,
                header: Header {
                    from: person("Luan Gjokaj", "luan@example.com"),
                    to: vec![person("Elena Brooks", "elena@example.com")],
                    ..Header::default()
                },
                html: Some(MOODBOARD_ASK_HTML.into()),
            }]
            .into();
            // An HTML message with attachments, to exercise the rich-text viewer.
            m.header = Header {
                from: person("Elena Brooks", "elena@example.com"),
                to: vec![person("Luan Gjokaj", "luan@example.com")],
                attachments: vec![
                    attachment("aurora-wallpaper.png", "image/png", 2_480_000),
                    attachment("moodboard.pdf", "application/pdf", 6_120_000),
                ],
                ..Header::default()
            };
            m.body = rich::from_html(SAMPLE_HTML).into();
            m.html = Some(SAMPLE_HTML.into());
            m
        },
        {
            let mut m = mail(
                "Daniel Osei",
                "DO",
                Tone::Graphite,
                "Contract v3",
                "Clause 4.2 updated per legal. Ready to countersign when you are.",
                "Wed",
            );
            m.yesterday = true;
            m.body = paragraphs(&[
                "Clause 4.2 is updated per legal. Ready to countersign when you are.",
            ])
            .into();
            m
        },
        {
            let mut m = mail(
                "Figma",
                "F",
                Tone::Violet,
                "Elena mentioned you",
                "“Can the unread counts be a touch quieter?”",
                "Wed",
            );
            m.yesterday = true;
            m.body = paragraphs(&["“Can the unread counts be a touch quieter?”"]).into();
            m
        },
        {
            let mut m = mail(
                "Google Calendar",
                "GC",
                Tone::Sky,
                "Design crit moved",
                "Now Thursday 2:00 PM, Studio B. Agenda attached.",
                "Wed",
            );
            m.yesterday = true;
            m.body =
                paragraphs(&["Design crit is now Thursday 2:00 PM in Studio B. Agenda attached."])
                    .into();
            m
        },
        {
            let mut m = mail(
                "Notion",
                "N",
                Tone::Amber,
                "Weekly digest: everything that changed across the Design workspace this week",
                "12 pages updated, 4 new comments in Roadmap 2027.",
                "Tue",
            );
            m.header = Header {
                from: person("Notion", "notify@mail.notion.so"),
                to: vec![person("", "luan@example.com")],
                unsubscribe: Some("https://example.com/unsubscribe".into()),
                ..Header::default()
            };
            m.yesterday = true;
            m.body = paragraphs(&["12 pages updated and 4 new comments in Roadmap 2027."]).into();
            m
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_draft_shows_placeholders() {
        let mut m = inbox().remove(0);
        m.row.subject = "  ".into();
        m.row.sender = String::new();
        assert_eq!(m.shown_subject(), "Untitled");
        assert_eq!(m.shown_sender(true), "Draft");
        assert_eq!(m.shown_sender(false), "Unknown sender");
        m.row.sender = "Unknown sender".into();
        m.row.subject = "(no subject)".into();
        assert_eq!(m.shown_subject(), "Untitled");
        assert_eq!(m.shown_sender(true), "Draft");
        m.row.subject = "Plan".into();
        m.row.sender = "Ana".into();
        assert_eq!(m.shown_subject(), "Plan");
        assert_eq!(m.shown_sender(true), "Ana");
    }

    #[test]
    fn a_thread_has_an_original_when_any_message_has_html() {
        let mut m = inbox().remove(0);
        assert!(m.has_html());
        m.html = None;
        assert!(m.has_html(), "its earlier messages have");
        for e in m.earlier.iter_mut() {
            e.html = None;
        }
        assert!(!m.has_html());
    }
}
