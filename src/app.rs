use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, Div, Entity, FocusHandle,
    Focusable as _, FontStyle, FontWeight, HighlightStyle, Hsla, MouseButton, SharedString,
    Stateful, StyledText, Transformation, Window, actions, div, ease_out_quint, percentage,
    prelude::*, px, relative,
};

use crate::data::{COMMANDS, Mail, inbox};
use crate::gmail::{self, View};
use crate::input::{InputEvent, TextInput};
use crate::rich::{self, Block, Kind};
use crate::theme::{Accent, SERIF, Theme, Tone, c, fade, gradient};
use crate::ui::{
    ButtonKind, avatar, button, icon, icon_button, kbd, meta, mono, overline, pill, shadow, tip,
};
use crate::webview::HtmlView;

actions!(
    nimbra,
    [
        Next,
        Prev,
        MarkDone,
        Undo,
        TogglePalette,
        Dismiss,
        Confirm,
        ToggleTheme,
        ToggleSidebar,
        ToggleList,
        ToggleStar,
        MarkUnread,
        Trash,
        Compose,
        Reply,
        SendMail,
        SaveDraft,
        ToggleFullCompose,
        ToggleOriginal,
        FocusSearch,
        HideWindow,
        Back,
        Forward,
        Reload,
        OpenSettings,
        SelectAllThreads,
        Print,
        // Menu-bar twins of actions whose shortcut is a bare key (E, R, S…). A menu item
        // takes its action's shortcut as a system key equivalent, which would fire while
        // typing; these have no shortcut of their own.
        MenuReply,
        MenuMarkDone,
        MenuMarkUnread,
        MenuStar,
        MenuTrash,
        MenuSnooze,
        MenuUndo,
        MenuToggleOriginal,
        MenuToggleSidebar,
        MenuSearch,
        Quit
    ]
);

/// Where the app is in connecting to a mailbox.
#[derive(Clone, PartialEq)]
enum Status {
    /// Built-in sample data, no network.
    Demo,
    SignedOut,
    SigningIn,
    Loading,
    Ready,
    Failed(SharedString),
}

/// The open composer: two single-line fields and a body made of one field per line.
struct Composer {
    /// Tells composers apart while one is still finishing in the background.
    serial: u64,
    to: Entity<TextInput>,
    cc: Entity<TextInput>,
    bcc: Entity<TextInput>,
    /// The Cc and Bcc fields are shown (after "Cc Bcc" is clicked, or when they have text).
    show_cc: bool,
    /// Finished addresses in To, Cc and Bcc, shown as pills; the fields hold only what is
    /// still being typed.
    chips: [Vec<String>; 3],
    /// Address suggestions under a recipient field.
    suggest: Option<Suggest>,
    /// Files picked or dropped in.
    files: Vec<gmail::OutgoingFile>,
    /// Files already attached to the draft being edited.
    kept: Vec<gmail::Attachment>,
    /// Counts edits; the draft is saved a moment after the last one.
    edits: u64,
    /// The edit count the saved draft has.
    saved_edits: u64,
    /// A save was asked for while one was running.
    resave: bool,
    /// What to do once the draft is saved.
    after_save: Option<After>,
    /// Written in the formatted editor (a web view) rather than the plain line fields.
    rich: bool,
    /// The editor's content, as HTML and as plain text.
    html: String,
    text: String,
    /// Content still to be put into the editor.
    load: Option<String>,
    /// Bold, italic, underline, bullets, numbers at the cursor.
    format: [bool; 5],
    /// Put the keyboard in the message body once the editor shows it.
    focus_body: bool,
    subject: Entity<TextInput>,
    lines: Vec<Entity<TextInput>>,
    reply: Option<gmail::ReplyTo>,
    /// The Gmail draft being edited, if this message was saved or opened from Drafts.
    draft_id: Option<String>,
    /// The earlier message a draft quotes or forwards. Kept out of the editor behind a
    /// "…" toggle and added back when the message is saved or sent.
    quoted: Option<String>,
    show_quoted: bool,
    saving: bool,
    /// The last save succeeded (shown in the footer).
    saved: bool,
    /// Fills the window instead of floating over the mail.
    full: bool,
    /// Top-left corner of the floating card; `None` until it is first laid out, centred.
    position: Option<(f32, f32)>,
    size: (f32, f32),
    /// An in-progress move or resize: what, where the mouse started, and the card's
    /// position and size at that moment.
    grab: Option<(Grab, (f32, f32), (f32, f32), (f32, f32))>,
    sending: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
enum ComposeKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// What a row's right-click menu can do.
#[derive(Clone, Copy, PartialEq)]
enum RowAction {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Done,
    MoveToInbox,
    Trash,
    ReportSpam,
    OpenInGmail,
}

impl RowAction {
    /// Icon and label for the menu.
    fn label(self) -> (&'static str, &'static str) {
        match self {
            RowAction::MarkRead => ("mail", "Mark as read"),
            RowAction::MarkUnread => ("mail", "Mark as unread"),
            RowAction::Star => ("star", "Star"),
            RowAction::Unstar => ("star", "Remove star"),
            RowAction::Done => ("done", "Mark done"),
            RowAction::MoveToInbox => ("inbox", "Move to inbox"),
            RowAction::Trash => ("trash", "Move to trash"),
            RowAction::ReportSpam => ("bell", "Report spam"),
            RowAction::OpenInGmail => ("link", "Open in Gmail"),
        }
    }
}

/// A small menu dropped from the reading toolbar.
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Labels,
    More,
    /// Times to send the open composer's message at.
    SendLater,
    /// Inbox, a label or Spam, for the open thread.
    MoveTo,
    /// The accounts on this Mac, Add account and Settings (from the account row).
    Accounts,
    /// The right-click menu of a row in the list (by index into the loaded threads).
    Row(usize),
}

/// A thread put away until later (snooze), or until later unless someone replies
/// (a reminder). Kept on this Mac: Gmail's API has no snooze.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Snooze {
    thread_id: String,
    /// When it comes back (or, for a reminder, when to check for a reply).
    until_ms: i64,
    /// Only come back if nobody has written since.
    no_reply: bool,
    /// The time of the thread's latest message when it was put away.
    since_ms: i64,
    /// The row, to list it under Snoozed.
    mail: Mail,
}

/// One choice in the snooze palette.
#[derive(Clone)]
struct SnoozeChoice {
    icon: &'static str,
    label: String,
    hint: String,
    header: Option<&'static str>,
    until_ms: i64,
    no_reply: bool,
}

/// Where Move to sends the open thread.
#[derive(Clone, PartialEq)]
enum MoveTarget {
    Inbox,
    Label(String),
    Spam,
}

/// What the mouse is doing to the floating composer.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Move,
    Resize,
}

const COMPOSER_MIN: (f32, f32) = (460., 320.);

/// Address suggestions for one recipient field.
struct Suggest {
    field: Field,
    people: Vec<gmail::Person>,
    selected: usize,
}

/// Something to do once a composer's draft is saved.
#[derive(Clone, Copy, PartialEq)]
enum After {
    /// Send it at this time (milliseconds since the epoch).
    Schedule(i64),
}

/// A composer that is off screen but still has work to finish.
#[derive(Clone, Copy, PartialEq)]
enum Parked {
    /// Closed; kept until its last edits are saved.
    Closing,
    /// Sent, waiting out the undo delay.
    Sending,
    /// Being sent now; brought back if sending fails.
    InFlight,
    /// Discarded, waiting out the undo delay before its draft is deleted.
    Discarded,
}

/// Changes and messages made while offline, sent once Gmail can be reached again.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Outbox {
    changes: Vec<(String, gmail::Change)>,
    mail: Vec<QueuedMail>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct QueuedMail {
    draft_id: Option<String>,
    outgoing: gmail::Outgoing,
}

/// A message to send later: its saved draft and when.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Scheduled {
    draft_id: String,
    at_ms: i64,
    subject: String,
}

/// How long a sent or discarded message can still be taken back.
const UNDO_DELAY: Duration = Duration::from_secs(6);

impl Composer {
    fn input(&self, field: Field) -> Option<Entity<TextInput>> {
        match field {
            Field::To => Some(self.to.clone()),
            Field::Cc => Some(self.cc.clone()),
            Field::Bcc => Some(self.bcc.clone()),
            Field::Subject => Some(self.subject.clone()),
            Field::Line(index) => self.lines.get(index).cloned(),
        }
    }

    /// Where a recipient field keeps its pills.
    fn slot(field: Field) -> Option<usize> {
        match field {
            Field::To => Some(0),
            Field::Cc => Some(1),
            Field::Bcc => Some(2),
            _ => None,
        }
    }

    /// A recipient field's pills and typed text, as one comma-separated list.
    fn recipients(&self, field: Field, cx: &App) -> String {
        let (Some(slot), Some(input)) = (Self::slot(field), self.input(field)) else {
            return String::new();
        };
        let typed = input.read(cx).text();
        self.chips[slot]
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(typed.trim()).filter(|t| !t.is_empty()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Turns the finished addresses typed in a recipient field into pills. `all` also takes
    /// the part after the last comma (on Tab, Enter, or when filled in by the app). Anything
    /// that is not a valid address stays as text, to be corrected.
    fn chipify(&mut self, field: Field, all: bool, cx: &mut App) {
        let (Some(slot), Some(input)) = (Self::slot(field), self.input(field)) else {
            return;
        };
        let text = input.read(cx).text();
        let mut parts: Vec<&str> = text.split([',', ';']).collect();
        let last = if all { "" } else { parts.pop().unwrap_or("") };
        let mut keep: Vec<String> = Vec::new();
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match gmail::addresses(part) {
                Ok(found) if !found.is_empty() => {
                    for address in found {
                        if !self.chips[slot]
                            .iter()
                            .any(|c| c.eq_ignore_ascii_case(&address))
                        {
                            self.chips[slot].push(address);
                        }
                    }
                }
                _ => keep.push(part.to_string()),
            }
        }
        let mut rest = keep.join(", ");
        if !last.trim().is_empty() {
            if !rest.is_empty() {
                rest.push_str(", ");
            }
            rest.push_str(last.trim_start());
        }
        if rest != text {
            input.update(cx, |input, cx| input.set(rest, usize::MAX, cx));
        }
        self.sync_placeholder(field, cx);
    }

    /// The To field's hint shows only while it has no pills.
    fn sync_placeholder(&self, field: Field, cx: &mut App) {
        if let (Field::To, Some(input)) = (field, self.input(field)) {
            let hint = if self.chips[0].is_empty() {
                "name@example.com"
            } else {
                ""
            };
            input.update(cx, |input, cx| input.set_placeholder(hint, cx));
        }
    }

    /// The message as it would be saved or sent.
    fn outgoing(&self, cx: &App) -> gmail::Outgoing {
        let html = self
            .rich
            .then(|| crate::editor::message_html(&self.html, self.quoted.as_deref()));
        gmail::Outgoing {
            to: self.recipients(Field::To, cx),
            cc: self.recipients(Field::Cc, cx),
            bcc: self.recipients(Field::Bcc, cx),
            subject: self.subject.read(cx).text(),
            body: self.body(cx),
            html,
            files: self.files.clone(),
            kept: self.kept.clone(),
            reply: self.reply.clone(),
        }
    }

    /// Anything typed or attached.
    fn has_content(&self, cx: &App) -> bool {
        (self.rich && !self.text.trim().is_empty())
            || self.chips.iter().any(|c| !c.is_empty())
            || !self.files.is_empty()
            || !self.kept.is_empty()
            || [&self.to, &self.cc, &self.bcc, &self.subject]
                .iter()
                .chain(self.lines.iter().collect::<Vec<_>>().iter())
                .any(|input| !input.read(cx).text().trim().is_empty())
    }

    fn dirty(&self) -> bool {
        self.edits != self.saved_edits
    }

    /// The whole message text: the edited lines, then the quoted earlier message.
    fn body(&self, cx: &App) -> String {
        let mut body = if self.rich {
            self.text.trim_end().to_string()
        } else {
            self.lines
                .iter()
                .map(|line| line.read(cx).text())
                .collect::<Vec<_>>()
                .join("\n")
        };
        if let Some(quoted) = &self.quoted {
            body.push_str("\n\n");
            body.push_str(quoted);
        }
        body
    }
}

/// Pull-to-refresh distances, in points: where a release triggers a refresh, where the
/// indicator rests while refreshing, and the distance the resistance is scaled to.
const PULL_TRIGGER: f32 = 64.;
const PULL_REST: f32 = 46.;
const PULL_LIMIT: f32 = 420.;

/// Rubber-band resistance: the further the pull, the less the list follows.
fn rubber_band(pull: f32) -> f32 {
    (1. - 1. / (pull * 0.55 / PULL_LIMIT + 1.)) * PULL_LIMIT
}

/// Where the indicator is, `start.elapsed()` after being let go: a critically damped
/// spring from `from` to `to`. It eases in quickly and comes to rest without overshooting,
/// so the list glides back into place and never bounces back into view.
fn settle(from: f32, to: f32, start: std::time::Instant) -> f32 {
    let t = start.elapsed().as_secs_f32();
    if t >= SETTLE_SECONDS {
        return to;
    }
    let k = 13. * t;
    to + (from - to) * (-k).exp() * (1. + k)
}

const SETTLE_SECONDS: f32 = 0.7;

const MENU_WIDTH: f32 = 220.;
/// Transparent space around the menu inside its window, so its shadow is not cut off.
const MENU_MARGIN: f32 = 28.;

/// A toolbar menu, shown in a small window of its own (see `Nimbra::toggle_menu`).
struct MenuPopup {
    owner: gpui::WeakEntity<Nimbra>,
    menu: Menu,
    items: Vec<(&'static str, SharedString, Hsla)>,
    /// False for a menu that only carries a message (no labels yet).
    enabled: bool,
    dark: bool,
    was_active: bool,
    opened: std::time::Instant,
    _activation: gpui::Subscription,
}

/// The "Undo" pill shown after a thread is removed (see `Nimbra::offer_undo`).
struct UndoToast {
    owner: gpui::WeakEntity<Nimbra>,
    label: &'static str,
    dark: bool,
}

impl Render for UndoToast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        div()
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .size_full()
                    .pl(px(16.))
                    .pr(px(6.))
                    .rounded(px(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(t.shadow, 12., 32., -6.)])
                    .child(div().flex_1().child(self.label))
                    .child(
                        button(&t, ButtonKind::Ghost, "Undo", Some("⌘Z"))
                            .id("undo")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                owner.update(cx, |owner, cx| owner.undo(cx)).ok();
                            })),
                    ),
            )
    }
}

impl MenuPopup {
    /// Closes the menu without choosing anything, and tells the owner so a click on the
    /// menu's own button closes it and does not reopen it straight away.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let menu = self.menu;
        self.owner
            .update(cx, |owner, _| {
                owner.menu_window = None;
                owner.menu_dismissed = Some((menu, std::time::Instant::now()));
            })
            .ok();
        window.remove_window();
    }
}

impl Render for MenuPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let hover = t.hover;
        let enabled = self.enabled;
        div()
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(px(6.))
                    .rounded(px(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(t.shadow, 10., 24., -6.)])
                    .children(
                        self.items
                            .iter()
                            .enumerate()
                            .map(|(index, (name, label, tint))| {
                                div()
                                    .id(("menu-item", index))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .px(px(10.))
                                    .py(px(7.))
                                    .rounded(px(8.))
                                    .child(icon(name, 16., *tint))
                                    .child(div().flex_1().child(label.clone()))
                                    .when(!enabled, |d| d.text_color(t.text3))
                                    .when(enabled, |d| {
                                        d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                let menu = this.menu;
                                                window.remove_window();
                                                this.owner
                                                    .update(cx, |owner, cx| {
                                                        owner.menu_choice(menu, index, cx)
                                                    })
                                                    .ok();
                                            }),
                                        )
                                    })
                            }),
                    ),
            )
    }
}

/// The account and sidebar state saved alongside the mailboxes.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    account: String,
    initials: String,
    counts: HashMap<String, (u32, u32)>,
    labels: Vec<gmail::Label>,
    #[serde(default)]
    synced_to: HashMap<String, String>,
}

/// The store record a mailbox is saved under.
fn record(view: View) -> String {
    format!("view-{}", view_key(view))
}

/// The message a reply or forward is about.
struct Source {
    subject: String,
    sender: String,
    header: crate::data::Header,
    body: Vec<Block>,
    reply_to: Option<gmail::ReplyTo>,
    /// For sample data, which has no real headers.
    fallback_to: Option<String>,
    fallback_all: Vec<String>,
}

impl Source {
    /// The latest message of a thread.
    fn latest(mail: &Mail) -> Self {
        Source {
            subject: mail.subject.to_string(),
            sender: mail.sender.to_string(),
            header: mail.header.clone(),
            body: mail.body.clone(),
            reply_to: mail.reply.as_ref().map(|(_, reply)| reply.clone()),
            fallback_to: mail.reply.as_ref().map(|(address, _)| address.clone()),
            fallback_all: mail.reply_all.clone(),
        }
    }

    /// An earlier message in a thread.
    fn earlier(mail: &Mail, earlier: &crate::data::Earlier) -> Self {
        let reply_to = mail
            .id
            .clone()
            .filter(|_| !earlier.header.message_id.is_empty())
            .map(|thread_id| gmail::ReplyTo {
                thread_id,
                message_id: earlier.header.message_id.clone(),
            });
        Source {
            subject: mail.subject.to_string(),
            sender: earlier.sender.to_string(),
            header: earlier.header.clone(),
            body: earlier.blocks.clone(),
            reply_to,
            fallback_to: None,
            fallback_all: Vec::new(),
        }
    }
}

/// "Fri, Nov 21, 2026 at 09:41", in local time; empty when unknown.
fn long_date(timestamp_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    if timestamp_ms <= 0 {
        return String::new();
    }
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|when| when.format("%a, %b %-d, %Y at %H:%M").to_string())
        .unwrap_or_default()
}

/// A free path in Downloads for `name` ("Plan.pdf", then "Plan 2.pdf", ...).
fn unique_download_path(name: &str) -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("no home folder"))?;
    let dir = std::path::PathBuf::from(home).join("Downloads");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1..1000 {
        let file = if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem} {n}{ext}")
        };
        let path = dir.join(file);
        if !path.exists() {
            return Ok(path);
        }
    }
    anyhow::bail!("too many files with that name in Downloads")
}

/// A private temporary folder for Quick Look previews.
fn preview_path(name: &str) -> anyhow::Result<std::path::PathBuf> {
    let mut id = [0u8; 8];
    getrandom::getrandom(&mut id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let dir = std::env::temp_dir()
        .join("nimbra-previews")
        .join(id.iter().map(|b| format!("{b:02x}")).collect::<String>());
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir.join(name))
}

fn restrict_file(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

/// Text made safe to put in HTML.
fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// "1.2 MB", "340 KB".
fn file_size(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b / 1024),
        b => format!("{:.1} MB", b as f64 / (1024. * 1024.)),
    }
}

/// Which composer field an event came from.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    To,
    Cc,
    Bcc,
    Subject,
    Line(usize),
}

/// Dragging the top strip moves the window; a double click zooms it, as macOS expects.
fn drag_or_zoom(event: &gpui::MouseDownEvent, window: &mut Window, _: &mut App) {
    if event.click_count == 2 {
        window.titlebar_double_click();
    } else {
        window.start_window_move();
    }
}

pub struct Nimbra {
    focus: FocusHandle,
    status: Status,
    /// Which mailbox the list is showing.
    mailbox: View,
    /// The last listing of each mailbox this session, with downloaded thread contents.
    /// Switching back shows it at once while a refresh runs in the background.
    /// Backed by the encrypted store on disk (see `persist`).
    cache: HashMap<View, Vec<Mail>>,
    /// The message whose full header (all recipients, date) is expanded: thread id and
    /// message index (0 is the latest).
    details: Option<(String, usize)>,
    /// The formatted message editor (a web view), made the first time it is needed.
    editor: Option<std::rc::Rc<crate::editor::EditorView>>,
    /// No editor on this system: composers use the plain line fields.
    editor_failed: bool,
    /// The editor's page has loaded.
    editor_ready: bool,
    /// Which composer the editor shows.
    editor_shown: Option<u64>,
    /// The colours and layout last given to the editor.
    editor_theme: Option<(bool, bool)>,
    /// GPUI's keyboard focus while typing in the editor (its "Editor" key context).
    editor_focus: FocusHandle,
    /// The editor had the keyboard at the last frame.
    editor_keys: bool,
    settings: crate::settings::Settings,
    /// The Settings sheet is open.
    settings_open: bool,
    /// One field per line of the signature, while Settings is open.
    signature_lines: Vec<Entity<TextInput>>,
    _settings_subscriptions: Vec<gpui::Subscription>,
    /// "Sign out" was clicked once; a second click within a few seconds signs out.
    sign_out_armed: Option<std::time::Instant>,
    /// Composers that are closed but still saving, waiting to send, or discarded.
    parked: Vec<(Parked, Composer)>,
    composer_serial: u64,
    /// Messages to send later, from this Mac.
    scheduled: Vec<Scheduled>,
    /// Scheduled drafts being sent right now.
    sending_scheduled: HashSet<String>,
    schedule_watch: bool,
    /// The times offered by the open Send later menu.
    send_later_times: Vec<i64>,
    /// The account shown: "" for the first one, else its id (see `gmail::is_account_id`).
    account_id: String,
    /// Bumped on every account switch, so answers for the previous account are dropped.
    account_gen: u64,
    /// Gmail could not be reached the last time we tried.
    offline: bool,
    /// What is waiting to go out once Gmail can be reached.
    outbox: Outbox,
    /// The offline retry loop is running.
    retrying: bool,
    /// The checking-for-mail bar was showing at the last frame.
    bar_shown: bool,
    /// When the bar started fading out (and a number per fade, for its animation).
    bar_fade: Option<(std::time::Instant, usize)>,
    /// The number shown on the Dock icon.
    badge: Option<u32>,
    /// A document waiting to be printed (it is loaded into the web view first).
    print_pending: Option<String>,
    /// Nimbra opens at login (read when Settings opens).
    login_item: bool,
    /// Rows picked with ⌘-click, Shift-click or ⌘A, for actions on several threads.
    picked: std::collections::BTreeSet<usize>,
    /// How many rows each entry of the undo history covers (one, or a whole batch).
    history_groups: Vec<usize>,
    /// Threads snoozed or waiting for a reply, kept on this Mac.
    snoozed: Vec<Snooze>,
    /// Snoozed threads being brought back right now.
    waking: HashSet<String>,
    /// The choices in the open snooze palette.
    snooze_choices: Vec<SnoozeChoice>,
    /// The next page of the open mailbox, when Gmail has more.
    next_page: Option<String>,
    loading_more: bool,
    /// Threads moved out of the inbox to a label, so Undo can take the label off again.
    moved: HashMap<String, String>,
    /// The New label sheet's name field, while it is open.
    new_label: Option<Entity<TextInput>>,
    _new_label_subscription: Option<gpui::Subscription>,
    /// What each item of the open Move to menu does.
    move_targets: Vec<MoveTarget>,
    /// A thread to open once it appears in the list (from a clicked notification).
    open_when_listed: Option<String>,
    /// A background fetch of thread contents is running.
    prefetching: bool,
    /// The encrypted on-disk mail store, once opened (real mail only).
    store: Option<Arc<crate::store::Store>>,
    /// Where the new-mail check reads Gmail's change feed from; `None` until its first
    /// look, which only takes stock and notifies about nothing.
    notify_from: Option<String>,
    /// Per mailbox (by store record name): the change-feed point its listing is current
    /// to, so a refresh only fetches what changed since.
    synced_to: HashMap<String, String>,
    /// Sidebar counts per label id: (unread threads still in the inbox, total threads for Drafts).
    counts: HashMap<String, (u32, u32)>,
    /// The user's own Gmail labels, shown in the sidebar.
    labels: Vec<gmail::Label>,
    /// A mailbox request is in flight.
    fetching: bool,
    /// The search field in the sidebar, and the query the results are for.
    search: Entity<TextInput>,
    query: String,
    _search_events: gpui::Subscription,
    /// The toolbar button under the mouse (what it does, its shortcut). Shown in the strip
    /// above an email in the original view, where a floating tooltip would be hidden
    /// behind the native web view.
    hint: Option<(&'static str, &'static str)>,
    /// Where a row's right-click menu opens (window coordinates), and what its items do.
    menu_at: (f32, f32),
    row_actions: Vec<RowAction>,
    /// The "Undo" pill shown after a thread is removed, in a window of its own so it
    /// floats above the email's web view; `undo_gen` restarts its timer.
    undo_window: Option<gpui::WindowHandle<UndoToast>>,
    undo_gen: usize,
    /// The main window's frame, kept from the last draw, to place pop-ups without a window.
    frame: Option<gpui::Bounds<gpui::Pixels>>,
    /// The toolbar menu's own window, while it is open.
    menu_window: Option<gpui::WindowHandle<MenuPopup>>,
    /// The open toolbar menu, if any.
    menu: Option<Menu>,
    /// The menu a click outside just closed, and when. Lets a click on the menu's own
    /// button close it instead of closing and instantly reopening it.
    menu_dismissed: Option<(Menu, std::time::Instant)>,
    /// "Show original": the message's own HTML in a web view instead of the native reader.
    original: bool,
    /// Remote images allowed in the web view for the open thread. On by default; the
    /// strip above the message can block them.
    original_images: bool,
    /// The web view, created the first time it is needed.
    html_view: Option<std::rc::Rc<HtmlView>>,
    /// What the web view currently shows (thread, images allowed), to avoid reloading.
    html_shown: Option<(String, bool)>,
    /// The colour scheme the web view was last told to use.
    html_dark: Option<bool>,
    /// The message being written, if the composer is open.
    compose: Option<Composer>,
    /// Focus to hand over on the next frame (set where no window is at hand).
    pending_focus: Option<FocusHandle>,
    /// The list's Unread filter.
    unread_only: bool,
    /// What took each cleared thread out of the list, so undo can reverse it.
    removed_by: HashMap<String, gmail::Change>,
    config: Option<gmail::Config>,
    gmail: Option<Arc<gmail::Client>>,
    /// Shown in the sidebar footer.
    account: SharedString,
    me_initials: SharedString,
    /// A short message shown at the bottom of the window (errors, hints).
    notice: Option<SharedString>,
    mails: Vec<Mail>,
    done: Vec<bool>,
    history: Vec<usize>,
    selected: usize,
    palette: bool,
    palette_sel: usize,
    dark: bool,
    sidebar_open: bool,
    /// The message list: shown or hidden, its width, and an in-progress resize
    /// (mouse x at the start, width at the start).
    list_open: bool,
    list_width: f32,
    list_grab: Option<(f32, f32)>,
    /// When the list was last shown or hidden, to animate it and restart the animation.
    list_toggled: Option<std::time::Instant>,
    list_gen: usize,
    /// Pull to refresh: the list's scroll position, how far it has been pulled past the
    /// top, and whether the current trackpad gesture started at the top.
    list_scroll: gpui::ScrollHandle,
    pull: f32,
    pull_armed: bool,
    /// The indicator settling after release: (from height, to height). `pull_gen` restarts it.
    pull_anim: Option<(f32, f32, std::time::Instant)>,
    pull_gen: usize,
    /// The indicator is held open while the refresh it started is running.
    pull_hold: bool,
    /// Bumped on every toggle / reload so the matching animation replays.
    sidebar_gen: usize,
    reload_gen: usize,
    /// Remote images the user asked to load, by URL. Nothing is fetched before that.
    images: HashMap<String, Arc<gpui::Image>>,
    image_requests: HashSet<String>,
    /// Threads visited before / after the current one, for the back and forward buttons.
    back: Vec<usize>,
    forward: Vec<usize>,
}

impl Nimbra {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // NIMBRA_SCREEN=palette|zero and NIMBRA_THEME=light open the sample data in a given
        // state, which is handy for screenshots. NIMBRA_DEMO=1 forces the sample data.
        let screen = std::env::var("NIMBRA_SCREEN").unwrap_or_default();
        let config = if !screen.is_empty() || std::env::var_os("NIMBRA_DEMO").is_some() {
            None
        } else {
            gmail::Config::load()
        };
        let demo = config.is_none();
        let mails = if demo { inbox() } else { Vec::new() };
        let zero = screen == "zero";
        let search = cx
            .new(|cx| TextInput::new(cx, "Search mail", Theme::dark().text3, Theme::dark().violet));
        let search_events = cx.subscribe(&search, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Enter => this.run_search(cx),
            // Down or Tab hands the keyboard back to the list.
            InputEvent::Down | InputEvent::Tab => {
                this.pending_focus = Some(this.focus.clone());
                cx.notify();
            }
            _ => {}
        });
        let mut this = Self {
            search,
            query: String::new(),
            _search_events: search_events,
            focus: cx.focus_handle(),
            status: if demo {
                Status::Demo
            } else {
                Status::SignedOut
            },
            // NIMBRA_MAILBOX=sent (or starred, snoozed, drafts, done, all, important, spam, trash)
            // opens that mailbox first.
            mailbox: match std::env::var("NIMBRA_MAILBOX").as_deref() {
                Ok("starred") => View::Starred,
                Ok("snoozed") => View::Snoozed,
                Ok("drafts") => View::Drafts,
                Ok("sent") => View::Sent,
                Ok("done") => View::Done,
                Ok("all") => View::AllMail,
                Ok("important") => View::Important,
                Ok("spam") => View::Spam,
                Ok("trash") => View::Trash,
                _ => View::Inbox,
            },
            cache: HashMap::new(),
            prefetching: false,
            open_when_listed: None,
            account_id: String::new(),
            account_gen: 0,
            offline: false,
            outbox: Outbox::default(),
            retrying: false,
            bar_shown: false,
            bar_fade: None,
            badge: None,
            print_pending: None,
            login_item: false,
            picked: Default::default(),
            history_groups: Vec::new(),
            snoozed: Vec::new(),
            waking: HashSet::new(),
            snooze_choices: Vec::new(),
            next_page: None,
            loading_more: false,
            moved: HashMap::new(),
            new_label: None,
            _new_label_subscription: None,
            move_targets: Vec::new(),
            editor: None,
            editor_failed: false,
            editor_ready: false,
            editor_shown: None,
            editor_theme: None,
            editor_focus: cx.focus_handle(),
            editor_keys: false,
            settings: crate::settings::load(),
            settings_open: false,
            signature_lines: Vec::new(),
            _settings_subscriptions: Vec::new(),
            sign_out_armed: None,
            parked: Vec::new(),
            composer_serial: 0,
            scheduled: Vec::new(),
            sending_scheduled: HashSet::new(),
            schedule_watch: false,
            send_later_times: Vec::new(),
            details: None,
            store: None,
            notify_from: None,
            synced_to: HashMap::new(),
            counts: HashMap::new(),
            labels: Vec::new(),
            fetching: false,
            hint: None,
            menu_at: (0., 0.),
            row_actions: Vec::new(),
            menu_window: None,
            undo_window: None,
            undo_gen: 0,
            frame: None,
            menu: None,
            menu_dismissed: None,
            original: false,
            original_images: true,
            html_view: None,
            html_shown: None,
            html_dark: None,
            compose: None,
            pending_focus: None,
            unread_only: false,
            removed_by: HashMap::new(),
            config,
            gmail: None,
            account: if demo {
                "Luan Gjokaj".into()
            } else {
                "Gmail".into()
            },
            me_initials: if demo { "LG".into() } else { "ME".into() },
            notice: None,
            done: vec![zero; mails.len()],
            history: if zero {
                (0..mails.len()).collect()
            } else {
                Vec::new()
            },
            // NIMBRA_OPEN=<row> opens that thread first (screenshots).
            selected: std::env::var("NIMBRA_OPEN")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|i| *i < mails.len())
                .unwrap_or(0),
            mails,
            palette: screen == "palette",
            palette_sel: 0,
            dark: std::env::var("NIMBRA_THEME").as_deref() != Ok("light"),
            list_open: std::env::var("NIMBRA_LIST").as_deref() != Ok("closed"),
            // NIMBRA_LIST_WIDTH=<px> starts with the list at that width (screenshots).
            list_width: std::env::var("NIMBRA_LIST_WIDTH")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .map_or(452., |w| w.clamp(300., 720.)),
            list_grab: None,
            list_toggled: None,
            list_gen: 0,
            list_scroll: gpui::ScrollHandle::new(),
            pull: 0.,
            pull_armed: false,
            pull_anim: None,
            pull_gen: 0,
            pull_hold: false,
            sidebar_open: std::env::var("NIMBRA_SIDEBAR").as_deref() != Ok("closed"),
            sidebar_gen: 0,
            reload_gen: 0,
            images: HashMap::new(),
            image_requests: HashSet::new(),
            back: Vec::new(),
            forward: Vec::new(),
        };
        // NIMBRA_SCREEN=compose opens the composer on the sample data (screenshots).
        if screen == "compose" || screen == "compose-full" {
            this.open_compose(ComposeKind::Reply, cx);
            if let Some(composer) = &mut this.compose {
                composer.full = screen == "compose-full";
            }
        }
        // Mail opens in its original formatting by default; NIMBRA_READER=1 starts in the
        // native reader instead. Messages without HTML always use the reader.
        this.original = this.settings.original_view && std::env::var_os("NIMBRA_READER").is_none();
        this.original_images = this.settings.load_images;
        // NIMBRA_SCREEN=draft opens a sample draft in the composer (screenshots).
        if screen == "draft" {
            this.edit_draft(
                gmail::Draft {
                    id: "sample".into(),
                    to: "mira@example.com".into(),
                    cc: String::new(),
                    bcc: String::new(),
                    attachments: Vec::new(),
                    subject: "Fwd: Billing problem".into(),
                    body: "Hi Mira,\n\nCan you take a look at this one?\n\nLuan Gjokaj\nUX Engineer & Designer".into(),
                    quoted: Some("---------- Forwarded message ---------\nFrom: Stripe\nSubject: Billing problem\n\nYour last payment could not be processed.".into()),
                    html: None,
                    reply: None,
                },
                cx,
            );
            if let Some(composer) = &mut this.compose {
                composer.show_quoted = true;
            }
        }
        // NIMBRA_SCREEN=bulk picks three threads (screenshots).
        if screen == "bulk" {
            this.picked = this.visible().into_iter().take(3).collect();
        }
        // NIMBRA_SCREEN=settings opens the Settings sheet (screenshots).
        if screen == "settings" {
            this.open_settings(cx);
        }
        // NIMBRA_SCREEN=loading holds the loading skeleton on screen (screenshots).
        if screen == "loading" {
            this.status = Status::Loading;
            this.fetching = true;
        }
        this.watch_for_mail(cx);
        this.watch_notification_clicks(cx);
        // Already signed in on a previous run: go straight to the inbox. The keychain is
        // read off the main thread because macOS may show a permission prompt for it, and
        // the window should not freeze (or fail to appear) while that is open.
        let active = this.settings.active.clone();
        if gmail::is_account_id(&active)
            && (active.is_empty() || this.settings.accounts.iter().any(|a| a.id == active))
        {
            this.account_id = active;
        }
        if let Some(config) = this.config.clone() {
            this.status = Status::Loading;
            let account = this.account_id.clone();
            cx.spawn(async move |this, cx| {
                let id = account.clone();
                let token = cx
                    .background_executor()
                    .spawn(async move { gmail::stored_refresh_token(&id) })
                    .await;
                this.update(cx, |this, cx| match token {
                    Some(token) => {
                        this.gmail = Some(Arc::new(gmail::Client::new(config, token, account)));
                        this.open_store(cx);
                    }
                    None => {
                        this.status = Status::SignedOut;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        // NIMBRA_SEARCH=<query> starts with that search already run.
        if let Ok(query) = std::env::var("NIMBRA_SEARCH") {
            this.search
                .update(cx, |input, cx| input.set(query, usize::MAX, cx));
            this.run_search(cx);
        }
        this
    }

    fn is_demo(&self) -> bool {
        self.status == Status::Demo
    }

    fn inbox_name(&self) -> SharedString {
        self.title(self.mailbox)
    }

    fn title(&self, view: View) -> SharedString {
        match view {
            View::Inbox if self.is_demo() => "Important".into(),
            View::Inbox => "Inbox".into(),
            View::Primary => "Primary".into(),
            View::Updates => "Updates".into(),
            View::Promotions => "Promotions".into(),
            View::Social => "Social".into(),
            View::Starred => "Starred".into(),
            View::Snoozed => "Snoozed".into(),
            View::Drafts => "Drafts".into(),
            View::Sent => "Sent".into(),
            View::Done => "Done".into(),
            View::AllMail => "All Mail".into(),
            View::Important => "Important".into(),
            View::Spam => "Spam".into(),
            View::Trash => "Trash".into(),
            View::Search => "Search".into(),
            View::Label(index) => self
                .labels
                .get(index)
                .map(|label| SharedString::from(label.name.clone()))
                .unwrap_or_else(|| "Label".into()),
        }
    }

    fn focus_search(&mut self, cx: &mut Context<Self>) {
        if !self.sidebar_open {
            self.toggle_sidebar(cx);
        }
        self.pending_focus = Some(self.search.read(cx).focus_handle(cx));
        cx.notify();
    }

    /// Runs the typed query: against Gmail (its full search syntax works), or locally
    /// on the sample data.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).text().trim().to_string();
        if query.is_empty() {
            self.clear_search(cx);
            return;
        }
        if matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.remember_mailbox();
        // Results are never cached: every search is fetched fresh.
        self.cache.remove(&View::Search);
        self.query = query;
        self.mailbox = View::Search;
        self.back.clear();
        self.forward.clear();
        self.palette = false;
        self.unread_only = false;
        self.pending_focus = Some(self.focus.clone());
        if self.is_demo() {
            self.selected = self.visible().first().copied().unwrap_or(0);
        } else {
            self.mails.clear();
            self.done.clear();
            self.history.clear();
            self.selected = 0;
            self.fetch_inbox(cx);
        }
        cx.notify();
    }

    /// Empties the search field and goes back to the inbox.
    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set(String::new(), 0, cx));
        self.query.clear();
        self.pending_focus = Some(self.focus.clone());
        if self.mailbox == View::Search {
            self.select_mailbox(View::Inbox, cx);
        }
        cx.notify();
    }

    /// Saves the current listing (minus threads cleared from it) for a later return.
    fn remember_mailbox(&mut self) {
        if self.is_demo() || self.status != Status::Ready {
            return;
        }
        let kept = self
            .mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
            .collect();
        if self.mailbox != View::Search {
            self.cache.insert(self.mailbox, kept);
        }
    }

    /// Switches the list to another mailbox.
    fn select_mailbox(&mut self, view: View, cx: &mut Context<Self>) {
        if self.mailbox == view || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.remember_mailbox();
        self.mailbox = view;
        self.back.clear();
        self.forward.clear();
        self.palette = false;
        if self.is_demo() {
            // The sample data only has an inbox; Done shows what was cleared from it.
            self.selected = self.visible().first().copied().unwrap_or(0);
            cx.notify();
        } else {
            // Show what we already have for this mailbox, then refresh it quietly.
            self.mails = match self.cache.get(&view) {
                Some(mails) => mails.clone(),
                // Not visited this session: start from what is stored on disk.
                None => self.stored_mails(view),
            };
            self.done = vec![false; self.mails.len()];
            self.history.clear();
            self.selected = 0;
            if !self.mails.is_empty() {
                self.status = Status::Ready;
                self.load_selected(cx);
            }
            self.refresh(cx);
        }
    }

    // ------------------------------------------------------------ gmail

    // ------------------------------------------------------------ syncing

    /// Brings the open mailbox up to date: only what changed since it was last synced when
    /// that is known, otherwise a full fetch of its newest threads.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.sync_view(cx) {
            self.fetch_inbox(cx);
        }
    }

    /// Applies Gmail's change feed to the open mailbox. Returns false when it cannot (no
    /// starting point, nothing on screen, or a search-based mailbox) so the caller fetches.
    fn sync_view(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(client) = self.gmail.clone() else {
            return false;
        };
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            _ => None,
        };
        if view.holds(&[], label_id.as_deref()).is_none() || self.mails.is_empty() {
            return false;
        }
        let Some(start) = self.synced_to.get(&record(view)).cloned() else {
            return false;
        };
        if self.fetching {
            return true;
        }
        self.fetching = true;
        cx.notify();
        let recipient = matches!(view, View::Sent | View::Drafts);
        let count_ids: Vec<String> = [
            "INBOX",
            "DRAFT",
            "CATEGORY_PERSONAL",
            "CATEGORY_UPDATES",
            "CATEGORY_PROMOTIONS",
            "CATEGORY_SOCIAL",
        ]
        .map(String::from)
        .into_iter()
        .chain(self.labels.iter().take(8).map(|l| l.id.clone()))
        .collect();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let changes = client.history(&start)?;
                    if changes.threads.len() > 40 {
                        return Err(gmail::HistoryExpired.into());
                    }
                    // Fresh details for each changed thread; `None` when it is gone.
                    let mut updates = Vec::new();
                    for id in &changes.threads {
                        match client.summary(id, recipient) {
                            Ok(summary) => updates.push((id.clone(), Some(summary))),
                            Err(error) if error.to_string().starts_with("Gmail error 404") => {
                                updates.push((id.clone(), None))
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    let counts = (!updates.is_empty()).then(|| client.counts(&count_ids));
                    anyhow::Ok((changes.latest, updates, counts))
                })
                .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                if this.mailbox != view {
                    return;
                }
                this.fetching = false;
                match result {
                    Ok((latest, updates, counts)) => {
                        this.synced_to.insert(record(view), latest);
                        if let Some(counts) = counts.filter(|c| !c.is_empty()) {
                            this.counts = counts;
                        }
                        // Count only; never log subjects or addresses.
                        eprintln!(
                            "nimbra: {view:?} synced ({} changed threads)",
                            updates.len()
                        );
                        if updates.is_empty() {
                            this.persist(cx);
                            this.prefetch(cx);
                            cx.notify();
                            return;
                        }
                        let mut list: Vec<Mail> = this.mails.clone();
                        for (id, summary) in updates {
                            list.retain(|mail| mail.id.as_deref() != Some(id.as_str()));
                            if let Some(summary) = summary
                                && view.holds(&summary.labels, label_id.as_deref()) == Some(true)
                            {
                                list.push(Mail::from_summary(summary));
                            }
                        }
                        list.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
                        this.install(list, cx);
                    }
                    Err(error) if error.is::<gmail::HistoryExpired>() => this.fetch_inbox(cx),
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
        true
    }

    /// Fetches, in the background, the content of the newest threads on screen that have not
    /// been downloaded yet, and saves it, so opening them is instant now and after a restart.
    fn prefetch(&mut self, cx: &mut Context<Self>) {
        const AHEAD: usize = 20;
        const WORKERS: usize = 4;
        let Some(client) = self.gmail.clone() else {
            return;
        };
        if self.prefetching || self.mailbox == View::Search || self.status != Status::Ready {
            return;
        }
        let ids: Vec<String> = self
            .visible()
            .into_iter()
            .filter_map(|i| self.mails.get(i))
            .filter(|mail| !mail.loaded)
            .filter_map(|mail| mail.id.clone())
            .take(AHEAD)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.prefetching = true;
        let view = self.mailbox;
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut results: Vec<Option<anyhow::Result<Vec<gmail::Message>>>> =
                        (0..ids.len()).map(|_| None).collect();
                    let chunk = ids.len().div_ceil(WORKERS).max(1);
                    std::thread::scope(|scope| {
                        for (ids, out) in ids.chunks(chunk).zip(results.chunks_mut(chunk)) {
                            let client = &client;
                            scope.spawn(move || {
                                for (id, slot) in ids.iter().zip(out) {
                                    *slot = Some(client.thread(id));
                                }
                            });
                        }
                    });
                    ids.into_iter()
                        .zip(results.into_iter().flatten())
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                this.prefetching = false;
                let mut filled = 0;
                for (id, result) in fetched {
                    match result {
                        Ok(messages) => {
                            if let Some(mail) = this
                                .mails
                                .iter_mut()
                                .find(|m| m.id.as_deref() == Some(id.as_str()) && !m.loaded)
                            {
                                mail.fill(messages);
                                filled += 1;
                            }
                        }
                        Err(error) if error.is::<gmail::SignedOut>() => {
                            this.fail(error, cx);
                            return;
                        }
                        // One thread failing (offline, deleted) does not stop the rest.
                        Err(_) => {}
                    }
                }
                if filled > 0 && this.mailbox == view {
                    // Count only; never log subjects or addresses.
                    eprintln!("nimbra: prefetched {filled} threads");
                    this.cache.insert(view, this.mails.clone());
                    this.persist(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ local store

    /// Opens the encrypted mail store in the background and, if nothing has arrived from
    /// Gmail yet, shows what was saved last time straight away.
    fn open_store(&mut self, cx: &mut Context<Self>) {
        // Ask once for permission to notify about new mail (macOS remembers the answer).
        static ASKED: std::sync::Once = std::sync::Once::new();
        ASKED.call_once(|| {
            crate::notify::show_while_active();
            crate::notify::request_permission();
            crate::notify::log_permission();
        });
        // NIMBRA_NOTIFY_TEST=1 posts one sample notification, to check the setup.
        if std::env::var_os("NIMBRA_NOTIFY_TEST").is_some() {
            crate::notify::show("test", "Nimbra", "Notifications are working.");
        }
        if self.store.is_some() {
            self.refresh(cx);
            return;
        }
        let view = self.mailbox;
        let account = self.account_id.clone();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let store = crate::store::Store::open(&account)?;
                    let saved: Option<Saved> = store.load("meta");
                    let mails: Option<Vec<Mail>> = store.load(&record(view));
                    let scheduled: Option<Vec<Scheduled>> = store.load("scheduled");
                    let snoozed: Option<Vec<Snooze>> = store.load("snoozed");
                    let outbox: Option<Outbox> = store.load("outbox");
                    anyhow::Ok((store, saved, mails, scheduled, snoozed, outbox))
                })
                .await;
            let Ok((store, saved, mails, scheduled, snoozed, outbox)) = opened else {
                eprintln!("nimbra: mail store unavailable; continuing without it");
                this.update(cx, |this, cx| this.fetch_inbox(cx)).ok();
                return;
            };
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                this.store = Some(Arc::new(store));
                this.scheduled = scheduled.unwrap_or_default();
                this.snoozed = snoozed.unwrap_or_default();
                this.outbox = outbox.unwrap_or_default();
                if !this.outbox.changes.is_empty() || !this.outbox.mail.is_empty() {
                    this.went_offline(cx);
                }
                if !this.scheduled.is_empty() || !this.snoozed.is_empty() {
                    this.watch_schedule(cx);
                }
                // Something is already on screen (a sign-in in this session): just refresh.
                if !this.mails.is_empty() || this.mailbox != view {
                    this.refresh(cx);
                    return;
                }
                if let Some(saved) = saved {
                    if saved.account.contains('@') {
                        let generation = this.account_gen;
                        this.remember_account(&saved.account, cx);
                        if this.account_gen != generation {
                            return;
                        }
                    }
                    this.account = saved.account.into();
                    this.me_initials = saved.initials.into();
                    this.counts = saved.counts;
                    this.labels = saved.labels;
                    this.synced_to = saved.synced_to;
                }
                let mut mails = mails.unwrap_or_default();
                for mail in &mut mails {
                    mail.refresh_time();
                }
                if !mails.is_empty() {
                    eprintln!("nimbra: opened {} stored threads", mails.len());
                    this.done = vec![false; mails.len()];
                    this.mails = mails;
                    this.selected = 0;
                    this.status = Status::Ready;
                    this.load_selected(cx);
                }
                // Bring it up to date: only what changed, when the store knows from where.
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the store holds for a mailbox, or nothing.
    fn stored_mails(&self, view: View) -> Vec<Mail> {
        let mut mails: Vec<Mail> = self
            .store
            .as_ref()
            .and_then(|store| store.load(&record(view)))
            .unwrap_or_default();
        for mail in &mut mails {
            mail.refresh_time();
        }
        mails
    }

    /// Saves the open mailbox and the sidebar state, in the background. Search results are
    /// never stored.
    fn persist(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.mailbox == View::Search || self.status != Status::Ready {
            return;
        }
        // Keep the store small: the newest 300 threads of a mailbox, and the full text of
        // the newest 100 (older ones are downloaded again when opened).
        const KEEP_THREADS: usize = 300;
        const KEEP_BODIES: usize = 100;
        let mails: Vec<Mail> = self
            .mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
            .take(KEEP_THREADS)
            .enumerate()
            .map(|(index, mut mail)| {
                if index >= KEEP_BODIES && mail.loaded {
                    mail.loaded = false;
                    mail.body.clear();
                    mail.earlier.clear();
                    mail.html = None;
                }
                mail
            })
            .collect();
        let name = record(self.mailbox);
        let saved = Saved {
            account: self.account.to_string(),
            initials: self.me_initials.to_string(),
            counts: self.counts.clone(),
            labels: self.labels.clone(),
            synced_to: self.synced_to.clone(),
        };
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = store
                    .save(&name, &mails)
                    .and_then(|_| store.save("meta", &saved))
                {
                    eprintln!("nimbra: saving the mail store failed: {error}");
                }
            })
            .detach();
    }

    /// Opens the thread behind a clicked notification. macOS brings the app forward itself.
    fn watch_notification_clicks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let clicked = crate::notify::take_clicked();
                let Some(id) = clicked.into_iter().rev().find(|id| id.as_str() != "test") else {
                    if this.update(cx, |_, _| ()).is_err() {
                        break; // the window is gone
                    }
                    continue;
                };
                // The window may be hidden (closed with ⌘W): bring it back first.
                cx.update(crate::show_main_window).ok();
                this.update(cx, |this, cx| this.open_thread(id, cx)).ok();
            }
        })
        .detach();
    }

    /// Shows a thread by its Gmail id: switches to the Inbox, and opens the thread now if
    /// it is listed, or as soon as the list has caught up.
    fn open_thread(&mut self, id: String, cx: &mut Context<Self>) {
        if self.compose.as_ref().is_some_and(|c| !c.full) {
            // Leave a message being written alone; just bring the app forward.
            return;
        }
        if self.mailbox != View::Inbox {
            self.select_mailbox(View::Inbox, cx);
        }
        self.unread_only = false;
        // "more" stands for several new threads: showing the Inbox is the answer.
        if id == "more" {
            return;
        }
        if !self.open_listed(&id, cx) {
            self.open_when_listed = Some(id);
            self.refresh(cx);
        }
    }

    /// Opens the thread with this id if it is in the list. Returns whether it was.
    fn open_listed(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.mails.iter().position(|m| m.id.as_deref() == Some(id)) else {
            return false;
        };
        if self.done.get(index) == Some(&true) {
            return false;
        }
        self.open(index);
        self.open_selected(cx);
        cx.notify();
        true
    }

    /// Reads Gmail's change feed every 30 seconds while the app is open: shows a
    /// notification for each thread with a new unread message in the inbox, and applies
    /// any change to the open mailbox.
    fn watch_for_mail(&mut self, cx: &mut Context<Self>) {
        const EVERY: Duration = Duration::from_secs(30);
        cx.spawn(async move |this, cx| {
            // The first look happens as soon as the account is ready, so it only takes stock
            // of what is already unread; anything arriving after that is announced.
            let mut first = true;
            loop {
                let ready = this
                    .update(cx, |this, _| this.gmail.is_some())
                    .unwrap_or(false);
                let wait = if first && !ready {
                    Duration::from_secs(3)
                } else if first {
                    Duration::ZERO
                } else {
                    EVERY
                };
                cx.background_executor().timer(wait).await;
                let Ok(client) = this.update(cx, |this, _| {
                    this.gmail.clone().map(|c| (c, this.account_gen))
                }) else {
                    break; // the window is gone
                };
                let Some((client, generation)) = client else {
                    continue; // not signed in
                };
                let from = this
                    .update(cx, |this, _| this.notify_from.clone())
                    .ok()
                    .flatten();
                let check = {
                    let client = client.clone();
                    cx.background_executor()
                        .spawn(async move {
                            match from {
                                // First look: just remember where the feed is now.
                                None => client.profile().map(|(_, at)| (at, None)),
                                Some(from) => match client.history(&from) {
                                    Ok(changes) => Ok((changes.latest.clone(), Some(changes))),
                                    Err(error) if error.is::<gmail::HistoryExpired>() => {
                                        client.profile().map(|(_, at)| (at, None))
                                    }
                                    Err(error) => Err(error),
                                },
                            }
                        })
                        .await
                };
                let Ok((at, changes)) = check else {
                    continue; // offline or signed out: try again next time
                };
                first = false;
                let fresh = this
                    .update(cx, |this, cx| {
                        // Switched accounts meanwhile: this feed belongs to the other one.
                        if this.account_gen != generation {
                            return Vec::new();
                        }
                        this.notify_from = Some(at);
                        let Some(changes) = changes else {
                            return Vec::new();
                        };
                        // Anything changed: bring the open mailbox up to date, cheaply.
                        if !changes.threads.is_empty() && this.compose.is_none() {
                            this.sync_view(cx);
                        }
                        changes.new_unread_inbox
                    })
                    .unwrap_or_default();
                if fresh.is_empty() {
                    continue;
                }
                // Sender and subject for the first few; the rest are summed up.
                let shown: Vec<String> = fresh.iter().take(3).cloned().collect();
                let summaries = cx
                    .background_executor()
                    .spawn(async move {
                        shown
                            .iter()
                            .filter_map(|id| client.summary(id, false).ok())
                            .collect::<Vec<_>>()
                    })
                    .await;
                let extra = fresh.len().saturating_sub(summaries.len());
                this.update(cx, |this, _| {
                    if !this.settings.notifications {
                        return;
                    }
                    for mail in &summaries {
                        crate::notify::show(&mail.id, &mail.sender, &mail.subject);
                    }
                    if extra > 0 {
                        crate::notify::show(
                            "more",
                            "Nimbra",
                            &format!("{extra} more new messages"),
                        );
                    }
                    // Count only; never log senders or subjects.
                    eprintln!("nimbra: {} new unread threads", fresh.len());
                })
                .ok();
            }
        })
        .detach();
    }

    /// Opens Google's consent page in the browser and waits for it in the background.
    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        match gmail::SignIn::start(&config) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.status = Status::SigningIn;
                self.notice = None;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.gmail = Some(Arc::new(client));
                            this.open_store(cx);
                        }
                        Err(error) => {
                            this.status = Status::Failed(error.to_string().into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.status = Status::Failed(error.to_string().into()),
        }
        cx.notify();
    }

    /// Add account: signs in to another Gmail account in the browser, then switches to it.
    /// The account shown stays as it is until that sign-in finishes.
    fn add_account(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            self.notice = Some("Connecting Gmail needs the Google sign-in set up first.".into());
            cx.notify();
            return;
        };
        let Ok(account) = gmail::new_account_id() else {
            return;
        };
        self.close_settings(cx);
        match gmail::SignIn::start(&config) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice = Some("Sign in to the other account in your browser.".into());
                cx.spawn(async move |this, cx| {
                    let id = account.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &id) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.reset_account(account, cx);
                            this.gmail = Some(Arc::new(client));
                            this.open_store(cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("No account was added: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Shows another of the accounts signed in on this Mac.
    fn switch_account(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        if id == self.account_id || !gmail::is_account_id(&id) {
            return;
        }
        self.reset_account(id.clone(), cx);
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let token = cx
                .background_executor()
                .spawn(async move { gmail::stored_refresh_token(&key) })
                .await;
            this.update(cx, |this, cx| {
                if this.account_id != id {
                    return;
                }
                match token {
                    Some(token) => {
                        this.gmail = Some(Arc::new(gmail::Client::new(config, token, id)));
                        this.open_store(cx);
                    }
                    None => this.status = Status::SignedOut,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Clears everything that belongs to the account shown, and makes `id` the account shown.
    fn reset_account(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_menu_window(cx);
        self.close_undo(cx);
        self.account_gen += 1;
        self.account_id = id.clone();
        self.settings.active = id;
        self.save_settings(cx);
        self.gmail = None;
        self.store = None;
        self.status = Status::Loading;
        self.fetching = false;
        self.mailbox = View::Inbox;
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.history_groups.clear();
        self.cache.clear();
        self.counts.clear();
        self.labels.clear();
        self.synced_to.clear();
        self.scheduled.clear();
        self.sending_scheduled.clear();
        self.snoozed.clear();
        self.waking.clear();
        self.outbox = Outbox::default();
        self.offline = false;
        self.parked.clear();
        self.compose = None;
        self.picked.clear();
        self.next_page = None;
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        self.removed_by.clear();
        self.moved.clear();
        self.notify_from = None;
        self.details = None;
        self.open_when_listed = None;
        self.html_shown = None;
        self.account = "".into();
        self.me_initials = "".into();
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Remembers the account shown in the account list (once its address is known). Signing
    /// in to an account that is already listed keeps the existing entry.
    fn remember_account(&mut self, email: &str, cx: &mut Context<Self>) {
        let id = self.account_id.clone();
        if let Some(other) = self
            .settings
            .accounts
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(email) && a.id != id)
            .map(|a| a.id.clone())
        {
            // The same Google account twice: drop the new sign-in, show the existing one.
            let duplicate = id.clone();
            std::thread::spawn(move || {
                gmail::forget_refresh_token(&duplicate);
                crate::store::wipe(&duplicate);
            });
            self.notice = Some("That account is already in Nimbra.".into());
            self.switch_account(other, cx);
            return;
        }
        match self.settings.accounts.iter_mut().find(|a| a.id == id) {
            Some(entry) if entry.email == email => return,
            Some(entry) => entry.email = email.to_string(),
            None => self.settings.accounts.push(crate::settings::AccountRef {
                id: id.clone(),
                email: email.to_string(),
            }),
        }
        self.settings.active = id;
        self.save_settings(cx);
    }

    fn fetch_inbox(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            return;
        };
        if self.mails.is_empty() {
            self.status = Status::Loading;
        }
        self.fetching = true;
        cx.notify();
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            View::Search => Some(self.query.clone()),
            _ => None,
        };
        let need_labels = self.labels.is_empty();
        let known_labels: Vec<String> = self.labels.iter().map(|l| l.id.clone()).collect();
        let sent_query = self.query.clone();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // Read the history point first, so nothing that changes during the
                    // fetch is missed by the next sync.
                    let (email, history_id) = client.profile()?;
                    let (threads, next_page) =
                        client.threads_page(view, label_id.as_deref(), 30, None)?;
                    // Labels are a nicety; the mailbox still loads without them.
                    let labels = need_labels.then(|| client.labels().unwrap_or_default());
                    // Sidebar counts: the system mailboxes plus the labels that are listed.
                    let mut ids: Vec<String> = [
                        "INBOX",
                        "DRAFT",
                        "CATEGORY_PERSONAL",
                        "CATEGORY_UPDATES",
                        "CATEGORY_PROMOTIONS",
                        "CATEGORY_SOCIAL",
                    ]
                    .map(String::from)
                    .to_vec();
                    ids.extend(match &labels {
                        Some(labels) => labels.iter().take(8).map(|l| l.id.clone()).collect(),
                        None => known_labels.into_iter().take(8).collect::<Vec<_>>(),
                    });
                    let counts = client.counts(&ids);
                    anyhow::Ok((email, history_id, threads, labels, counts, next_page))
                })
                .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                // Ignore the answer if the user has moved to another mailbox or search meanwhile.
                if this.mailbox != view || (view == View::Search && this.query != sent_query) {
                    return;
                }
                this.fetching = false;
                match result {
                    Ok((email, history_id, threads, labels, counts, next_page)) => {
                        this.next_page = next_page;
                        if view.holds(&[], None).is_some() {
                            this.synced_to.insert(record(view), history_id);
                        }
                        if !counts.is_empty() {
                            this.counts = counts;
                        }
                        if let Some(labels) = labels {
                            this.labels = labels;
                        }
                        this.show_inbox(email, threads, cx)
                    }
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows a freshly fetched listing, reusing thread contents already in memory and
    /// keeping the open thread and any just-cleared threads as they were.
    fn show_inbox(&mut self, email: String, threads: Vec<gmail::Summary>, cx: &mut Context<Self>) {
        // A different Google account than the one the store was saved for: start clean.
        if self.account.contains('@') && self.account.as_ref() != email.as_str() {
            if let Some(store) = &self.store {
                store.clear();
            }
            self.cache.clear();
            self.counts.clear();
            self.labels.clear();
        }
        self.me_initials = email
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();
        let generation = self.account_gen;
        self.remember_account(&email, cx);
        if self.account_gen != generation {
            return; // it was a second sign-in to an account already here
        }
        self.account = email.into();

        let fresh = threads.into_iter().map(Mail::from_summary).collect();
        self.install(fresh, cx);
    }

    /// Puts a new listing on screen, keeping downloaded thread contents, the open thread and
    /// threads cleared in the meantime, then saves it.
    fn install(&mut self, fresh: Vec<Mail>, cx: &mut Context<Self>) {
        let mut fresh = fresh;
        // Snoozed lists Gmail's own snoozed threads and the ones snoozed here.
        if self.mailbox == View::Snoozed {
            for snooze in &self.snoozed {
                if fresh
                    .iter()
                    .any(|m| m.id.as_deref() == Some(snooze.thread_id.as_str()))
                {
                    continue;
                }
                let mut mail = snooze.mail.clone();
                mail.time = chrono::DateTime::from_timestamp_millis(snooze.until_ms)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%a %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default()
                    .into();
                fresh.push(mail);
            }
        }
        let open_id = self.mails.get(self.selected).and_then(|m| m.id.clone());
        // Threads cleared while this request was in flight must stay cleared.
        let cleared: Vec<Option<String>> = self
            .history
            .iter()
            .filter_map(|i| self.mails.get(*i).map(|m| m.id.clone()))
            .collect();
        self.mails = crate::data::merge(&self.mails, fresh);
        let position = |id: &Option<String>| self.mails.iter().position(|m| &m.id == id);
        self.history = cleared.iter().filter_map(position).collect();
        self.selected = position(&open_id)
            .filter(|_| open_id.is_some())
            .unwrap_or(0);
        self.done = vec![false; self.mails.len()];
        for i in &self.history {
            self.done[*i] = true;
        }
        if self.done.get(self.selected) == Some(&true) {
            self.selected = self.visible().first().copied().unwrap_or(0);
        }
        self.back.clear();
        self.forward.clear();
        self.notice = None;
        self.status = Status::Ready;
        self.cache.insert(self.mailbox, self.mails.clone());
        self.persist(cx);
        self.prefetch(cx);
        if let Some(id) = self.open_when_listed.take() {
            self.open_listed(&id, cx);
        }
        // Counts only; never log addresses, subjects or message text.
        eprintln!(
            "nimbra: {:?} listed ({} threads, {} already in memory, {} sidebar counts)",
            self.mailbox,
            self.mails.len(),
            self.mails.iter().filter(|m| m.loaded).count(),
            self.counts.len()
        );
        self.load_selected(cx);
        cx.notify();
    }

    /// Fetches the messages of the open thread the first time it is shown.
    /// The user opened this thread themselves (clicked it, moved to it with the keyboard,
    /// or clicked its text): mark it read, here and in Gmail, and fetch its content.
    ///
    /// A thread that is only shown because a mailbox was just opened stays unread until
    /// then; see `load_selected`.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.set_unread(self.selected, false, cx);
        self.load_selected(cx);
    }

    /// Fetches the content of the thread on show, without marking it read.
    fn load_selected(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(mail)) = (self.gmail.clone(), self.mails.get(self.selected)) else {
            return;
        };
        let (false, Some(id)) = (mail.loaded, mail.id.clone()) else {
            return;
        };
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let thread_id = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { client.thread(&thread_id) })
                .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                match result {
                    Ok(messages) => {
                        // The inbox may have been reloaded meanwhile, so look the thread up again.
                        if let Some(mail) =
                            this.mails.iter_mut().find(|m| m.id.as_deref() == Some(&id))
                        {
                            mail.fill(messages);
                            this.persist(cx);
                            cx.notify();
                        }
                    }
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sends a change to Gmail. If it removed the thread from the list and Gmail refuses,
    /// the thread is put back.
    fn sync(&mut self, index: usize, change: gmail::Change, removed: bool, cx: &mut Context<Self>) {
        let (Some(client), Some(id)) = (
            self.gmail.clone(),
            self.mails.get(index).and_then(|m| m.id.clone()),
        ) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let thread_id = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { client.change(&thread_id, change) })
                .await;
            let Err(error) = result else {
                return;
            };
            this.update(cx, |this, cx| {
                // Offline: keep the change here and send it when the connection is back.
                if error.is::<gmail::Offline>() {
                    this.outbox.changes.push((id, change));
                    this.went_offline(cx);
                    return;
                }
                // Only roll back if this row is still the same thread.
                if removed && this.mails.get(index).and_then(|m| m.id.as_deref()) == Some(&id) {
                    this.done[index] = false;
                    this.history.retain(|i| *i != index);
                }
                this.fail(error, cx);
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ composing

    fn new_field(
        &mut self,
        field: Field,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> (Entity<TextInput>, gpui::Subscription) {
        let t = self.theme();
        let placeholder = placeholder.to_string();
        let input = cx.new(|cx| TextInput::new(cx, &placeholder, t.faint, t.violet));
        // Lines move around as the body is edited, so events look their line up by entity.
        let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
            let field = match field {
                Field::Line(_) => this
                    .compose
                    .as_ref()
                    .and_then(|c| c.lines.iter().position(|line| *line == input))
                    .map(Field::Line),
                other => Some(other),
            };
            if let Some(field) = field {
                match event {
                    InputEvent::Changed => this.field_changed(field, cx),
                    event => this.field_event(field, event, cx),
                }
            }
        });
        (input, subscription)
    }

    /// Opens the composer, empty or as a reply to the open thread.
    fn open_compose(&mut self, kind: ComposeKind, cx: &mut Context<Self>) {
        let source = self
            .mails
            .get(self.selected)
            .filter(|_| !self.visible().is_empty())
            .map(|mail| Source::latest(mail));
        self.open_compose_from(kind, source, cx);
    }

    /// Opens the composer to reply to, reply to all on, or forward one message.
    fn open_compose_from(
        &mut self,
        kind: ComposeKind,
        source: Option<Source>,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        if self.compose.is_some() || self.palette {
            return;
        }
        let (to, s1) = self.new_field(Field::To, "name@example.com", cx);
        let (subject, s2) = self.new_field(Field::Subject, "Subject", cx);
        let (line, s3) = self.new_field(Field::Line(0), "Write your message…", cx);
        let (cc, s4) = self.new_field(Field::Cc, "", cx);
        let (bcc, s5) = self.new_field(Field::Bcc, "", cx);
        self.composer_serial += 1;
        let mut composer = Composer {
            serial: self.composer_serial,
            to,
            cc,
            bcc,
            show_cc: false,
            chips: Default::default(),
            suggest: None,
            files: Vec::new(),
            kept: Vec::new(),
            edits: 0,
            saved_edits: 0,
            resave: false,
            after_save: None,
            rich: !self.editor_failed && cfg!(any(target_os = "macos", target_os = "windows")),
            html: String::new(),
            text: String::new(),
            load: None,
            format: [false; 5],
            focus_body: false,
            subject,
            lines: vec![line],
            reply: None,
            full: false,
            position: None,
            size: (720., 520.),
            grab: None,
            draft_id: None,
            quoted: None,
            show_quoted: false,
            saving: false,
            saved: false,
            sending: false,
            error: None,
            _subscriptions: vec![s1, s2, s3, s4, s5],
        };
        let mut focus = composer.to.read(cx).focus_handle(cx);
        if kind != ComposeKind::New
            && let Some(source) = source
        {
            let prefix = if kind == ComposeKind::Forward {
                "Fwd: "
            } else {
                "Re: "
            };
            let subject = if source
                .subject
                .to_lowercase()
                .starts_with(&prefix.trim().to_lowercase())
            {
                source.subject.clone()
            } else {
                format!("{prefix}{}", source.subject)
            };
            composer
                .subject
                .update(cx, |input, cx| input.set(subject, usize::MAX, cx));
            let header = &source.header;
            let when = long_date(header.date_ms);
            let from = if header.from.address.is_empty() {
                source.sender.clone()
            } else if header.from.name.is_empty() {
                header.from.address.clone()
            } else {
                format!("{} <{}>", header.from.name, header.from.address)
            };
            let body: Vec<String> = source
                .body
                .iter()
                .flat_map(|block| {
                    block
                        .text
                        .lines()
                        .map(str::to_string)
                        .chain([String::new()])
                })
                .take(2000)
                .collect();
            if kind == ComposeKind::Forward {
                // The forwarded message is folded behind "•••" and added when sent.
                let to = header
                    .to
                    .iter()
                    .map(|p| p.address.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut quoted = vec![
                    "---------- Forwarded message ----------".to_string(),
                    format!("From: {from}"),
                ];
                if !when.is_empty() {
                    quoted.push(format!("Date: {when}"));
                }
                quoted.push(format!("Subject: {}", source.subject));
                if !to.is_empty() {
                    quoted.push(format!("To: {to}"));
                }
                quoted.push(String::new());
                quoted.extend(body);
                composer.quoted = Some(quoted.join("\n").trim_end().to_string());
            } else {
                let address = if header.from.address.is_empty() {
                    source.fallback_to.clone()
                } else {
                    Some(header.reply_address().to_string())
                };
                if let Some(address) = address {
                    let mut to = vec![address];
                    if kind == ComposeKind::ReplyAll {
                        let others = if header.from.address.is_empty() {
                            source.fallback_all.clone()
                        } else {
                            header.everyone()
                        };
                        for other in others {
                            let me = other.eq_ignore_ascii_case(&self.account);
                            if !me && !to.iter().any(|known| known.eq_ignore_ascii_case(&other)) {
                                to.push(other);
                            }
                        }
                    }
                    composer
                        .to
                        .update(cx, |input, cx| input.set(to.join(", "), usize::MAX, cx));
                    composer.chipify(Field::To, true, cx);
                }
                composer.reply = source.reply_to.clone();
                // The message being answered is quoted below, folded behind "•••".
                let intro = if when.is_empty() {
                    format!("{from} wrote:")
                } else {
                    format!("On {when}, {from} wrote:")
                };
                let quoted: Vec<String> = std::iter::once(intro)
                    .chain(body.iter().map(|line| format!("> {line}")))
                    .collect();
                composer.quoted = Some(quoted.join("\n").trim_end().to_string());
                focus = composer.lines[0].read(cx).focus_handle(cx);
            }
        }
        // The editor starts with the signature, below room to write.
        let signature = self.settings.signature.trim_end();
        let mut start = String::from("<div><br></div>");
        if !signature.is_empty() {
            start.push_str("<div><br></div><div class=\"sig\">");
            start.push_str(&crate::editor::text_to_html(signature));
            start.push_str("</div>");
        }
        composer.load = Some(start);
        if composer.rich
            && composer
                .lines
                .first()
                .is_some_and(|l| l.read(cx).focus_handle(cx) == focus)
        {
            composer.focus_body = true;
            focus = self.editor_focus.clone();
        }
        self.pending_focus = Some(focus);
        self.compose = Some(composer);
        cx.notify();
    }

    /// Creates, fills, shows or hides the message editor to match the composer.
    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(serial) = self.compose.as_ref().filter(|c| c.rich).map(|c| c.serial) else {
            if self.editor_shown.take().is_some()
                && let Some(editor) = &self.editor
            {
                editor.set_visible(false);
                editor.blur();
            }
            return;
        };
        if self.editor.is_none() && !self.editor_failed {
            let (events, mut incoming) = futures::channel::mpsc::unbounded();
            match crate::editor::EditorView::new(window, events) {
                Some(view) => {
                    self.editor = Some(std::rc::Rc::new(view));
                    cx.spawn(async move |this, cx| {
                        use futures::StreamExt as _;
                        while let Some(event) = incoming.next().await {
                            if this
                                .update(cx, |this, cx| this.editor_event(event, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                None => self.editor_failed = true,
            }
        }
        let Some(editor) = self.editor.clone().filter(|_| !self.editor_failed) else {
            // No editor here: fall back to the plain fields, which hold the same text.
            if let Some(composer) = &mut self.compose {
                composer.rich = false;
            }
            return;
        };
        if !self.editor_ready {
            return;
        }
        // Focus moved from the editor to one of the app's own fields (a click, Tab, ↑):
        // the web view must also give up the keyboard, or typing would still go there.
        let focused = self.editor_focus.is_focused(window);
        if self.editor_keys && !focused {
            editor.blur();
        }
        self.editor_keys = focused;
        let t = self.theme();
        let full = self.compose.as_ref().is_some_and(|c| c.full);
        if self.editor_theme != Some((self.dark, full)) {
            let css = |color: Hsla| {
                let c = color.to_rgb();
                format!(
                    "rgba({},{},{},{:.3})",
                    (c.r * 255.).round(),
                    (c.g * 255.).round(),
                    (c.b * 255.).round(),
                    c.a
                )
            };
            editor.theme(serde_json::json!({
                "--bg": css(if full { t.surface } else { t.raised }),
                "--text": css(t.text),
                "--dim": css(t.text3),
                "--accent": css(t.violet),
                "--line": css(t.border_strong),
                "--max": if full { "860px" } else { "none" },
                "--pad": if full { "24px" } else { "14px" },
            }));
            self.editor_theme = Some((self.dark, full));
        }
        let shown = self.editor_shown;
        let Some(composer) = self.compose.as_mut() else {
            return;
        };
        let focus = std::mem::take(&mut composer.focus_body);
        if shown != Some(serial) || composer.load.is_some() {
            let html = composer
                .load
                .take()
                .unwrap_or_else(|| composer.html.clone());
            editor.load(
                serial,
                &html,
                composer.quoted.as_deref().unwrap_or(""),
                focus,
            );
            self.editor_shown = Some(serial);
        } else if focus {
            editor.focus();
        }
        editor.set_visible(true);
    }

    fn editor_event(&mut self, event: crate::editor::EditorEvent, cx: &mut Context<Self>) {
        use crate::editor::EditorEvent;
        match event {
            EditorEvent::Ready => {
                self.editor_ready = true;
                self.editor_shown = None;
                self.editor_theme = None;
            }
            EditorEvent::Content {
                session,
                html,
                text,
                edited,
            } => {
                let on_screen = self.compose.as_ref().is_some_and(|c| c.serial == session);
                let Some(composer) = self.composer_mut(session) else {
                    return;
                };
                let differs = composer.html != html;
                composer.html = html;
                composer.text = text;
                if edited && differs && on_screen {
                    if let Some(composer) = &mut self.compose {
                        composer.suggest = None;
                    }
                    self.composer_changed(cx);
                }
            }
            EditorEvent::Format { session, state } => {
                if let Some(composer) = self.compose.as_mut().filter(|c| c.serial == session) {
                    composer.format = state;
                }
            }
            EditorEvent::Focused => {
                if let Some(composer) = &mut self.compose {
                    composer.suggest = None;
                }
                self.pending_focus = Some(self.editor_focus.clone());
            }
            EditorEvent::Up => self.focus_field(Field::Subject, None, cx),
            EditorEvent::Escape => self.close_compose(cx),
            EditorEvent::Send => self.send_mail(cx),
            EditorEvent::Save => self.save_draft(cx),
            EditorEvent::ToggleFull => self.toggle_full_compose(cx),
            EditorEvent::Dropped(paths) => self.attach_files(paths, cx),
        }
        cx.notify();
    }

    /// Starts moving or resizing the floating composer.
    fn grab_composer(&mut self, grab: Grab, event: &gpui::MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.compose
            && let Some(position) = composer.position
            && !composer.full
        {
            let mouse = (f32::from(event.position.x), f32::from(event.position.y));
            composer.grab = Some((grab, mouse, position, composer.size));
            cx.notify();
        }
    }

    /// Follows the mouse while the composer is grabbed, keeping it reachable on screen.
    fn drag_composer(
        &mut self,
        event: &gpui::MouseMoveEvent,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        let Some((grab, start, position, size)) = composer.grab else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            composer.grab = None;
            return;
        }
        let dx = f32::from(event.position.x) - start.0;
        let dy = f32::from(event.position.y) - start.1;
        match grab {
            Grab::Move => {
                composer.position = Some((
                    (position.0 + dx).clamp(80. - size.0, viewport.0 - 80.),
                    (position.1 + dy).clamp(0., viewport.1 - 60.),
                ));
            }
            Grab::Resize => {
                composer.size = (
                    (size.0 + dx).clamp(
                        COMPOSER_MIN.0,
                        (viewport.0 - position.0).max(COMPOSER_MIN.0),
                    ),
                    (size.1 + dy).clamp(
                        COMPOSER_MIN.1,
                        (viewport.1 - position.1).max(COMPOSER_MIN.1),
                    ),
                );
            }
        }
        cx.notify();
    }

    /// Switches the open thread between the native reader and its original HTML.
    fn toggle_original(&mut self, cx: &mut Context<Self>) {
        let has_html = self
            .mails
            .get(self.selected)
            .is_some_and(|mail| mail.html.is_some());
        if !self.original && !has_html {
            self.notice = Some("This message has no HTML version to show.".into());
        } else {
            self.original = !self.original;
        }
        cx.notify();
    }

    /// The thread the web view should be showing right now, if any. It stays hidden
    /// whenever something is drawn over the reading pane, because a native web view
    /// always sits on top of everything GPUI paints.
    fn original_target(&self) -> Option<(String, SharedString)> {
        let ready = matches!(self.status, Status::Demo | Status::Ready);
        // (A toolbar menu does not hide it: menus open in a window of their own, above it.)
        let covered = self.palette
            || self.picked.len() >= 2
            || self.compose.is_some()
            || self.notice.is_some()
            || self.settings_open
            || self.new_label.is_some();
        if !self.original
            || !ready
            || covered
            || self.mailbox == View::Drafts
            || !self.visible().contains(&self.selected)
        {
            return None;
        }
        let mail = self.mails.get(self.selected)?;
        let key = format!("{}:{}", self.selected, mail.id.clone().unwrap_or_default());
        Some((key, mail.html.clone()?))
    }

    /// Creates, loads, shows or hides the web view to match the current state.
    fn sync_html_view(&mut self, window: &mut Window) {
        // A print job borrows the web view: load the document, then open the print panel.
        if let Some(doc) = self.print_pending.take() {
            if self.html_view.is_none() {
                self.html_view = HtmlView::new(window).map(std::rc::Rc::new);
            }
            if let Some(view) = &self.html_view {
                view.show(&doc, self.original_images);
                self.html_shown = None;
            }
        }
        let Some((key, html)) = self.original_target() else {
            if let Some(view) = &self.html_view {
                view.set_visible(false);
            }
            return;
        };
        if self.html_view.is_none() {
            self.html_view = HtmlView::new(window).map(std::rc::Rc::new);
        }
        let Some(view) = &self.html_view else {
            // No web view on this platform: fall back to the native reader.
            self.original = false;
            self.notice = Some("The original view is not available on this system.".into());
            return;
        };
        // The page follows Nimbra's light or dark theme, so emails with dark styles use them.
        if self.html_dark != Some(self.dark) {
            view.set_dark(self.dark);
            self.html_dark = Some(self.dark);
        }
        // A different thread starts with remote images on again.
        if self
            .html_shown
            .as_ref()
            .is_some_and(|(shown, _)| *shown != key)
        {
            self.original_images = self.settings.load_images;
        }
        let wanted = (key, self.original_images);
        if self.html_shown.as_ref() != Some(&wanted) {
            view.show(&html, self.original_images);
            self.html_shown = Some(wanted);
        }
        view.set_visible(true);
    }

    fn set_hint(
        &mut self,
        hovered: bool,
        label: &'static str,
        key: &'static str,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.hint = Some((label, key));
        } else if self.hint.is_some_and(|(current, _)| current == label) {
            // Only clear our own hint: moving straight to the next button sets its hint
            // first, and that one must stay.
            self.hint = None;
        }
        cx.notify();
    }

    /// Opens a toolbar menu as a small borderless window of its own, placed under the
    /// button. A separate window floats above the email's native web view, so the menu can
    /// overlap the email without moving or hiding it.
    fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        let just_dismissed = self.menu_dismissed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(400)
        });
        self.close_menu_window(cx);
        if just_dismissed {
            return;
        }
        let t = self.theme();
        let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
        let items: Vec<(&'static str, SharedString, Hsla)> = match menu {
            Menu::Labels if self.is_demo() => vec![(
                "tag",
                "Labels need a connected Gmail account.".into(),
                t.text3,
            )],
            Menu::Labels => self
                .labels
                .iter()
                .enumerate()
                .take(12)
                .map(|(index, label)| ("tag", short(&label.name, 28), tints[index % tints.len()]))
                .chain(std::iter::once(("tag", "New label…".into(), t.text2)))
                .collect(),
            Menu::MoveTo => {
                let mut targets = Vec::new();
                if matches!(
                    self.mailbox,
                    View::Done
                        | View::Spam
                        | View::Trash
                        | View::Label(_)
                        | View::Starred
                        | View::Important
                        | View::AllMail
                        | View::Search
                ) {
                    targets.push(MoveTarget::Inbox);
                }
                targets.extend(
                    self.labels
                        .iter()
                        .take(12)
                        .map(|l| MoveTarget::Label(l.id.clone())),
                );
                if self.mailbox != View::Spam && !self.is_demo() {
                    targets.push(MoveTarget::Spam);
                }
                let items = targets
                    .iter()
                    .map(|target| match target {
                        MoveTarget::Inbox => ("inbox", "Inbox".into(), t.text2),
                        MoveTarget::Spam => ("ban", "Spam".into(), t.text2),
                        MoveTarget::Label(id) => {
                            let index = self.labels.iter().position(|l| &l.id == id).unwrap_or(0);
                            let name = self
                                .labels
                                .get(index)
                                .map(|l| l.name.clone())
                                .unwrap_or_default();
                            ("folder", short(&name, 28), tints[index % tints.len()])
                        }
                    })
                    .collect::<Vec<_>>();
                self.move_targets = targets;
                if items.is_empty() {
                    vec![("folder", "Nowhere to move this to yet.".into(), t.text3)]
                } else {
                    items
                }
            }
            Menu::Accounts => self
                .settings
                .accounts
                .iter()
                .map(|a| {
                    let active = a.id == self.account_id;
                    (
                        if active { "done" } else { "mail" },
                        short(&a.email, 30),
                        if active { t.violet } else { t.text2 },
                    )
                })
                .chain([
                    ("users", "Add account…".into(), t.text2),
                    ("sun", "Settings…".into(), t.text2),
                ])
                .collect(),
            Menu::SendLater => {
                let choices = Self::send_later_choices();
                self.send_later_times = choices.iter().map(|(_, at)| *at).collect();
                choices
                    .into_iter()
                    .map(|(label, _)| ("clock", label.into(), t.text2))
                    .collect()
            }
            Menu::More if self.is_demo() => vec![("inbox", "Mark as unread".into(), t.text2)],
            Menu::More => vec![
                ("inbox", "Mark as unread".into(), t.text2),
                ("link", "Open in Gmail".into(), t.text2),
                ("bell", "Report spam".into(), t.text2),
            ],
            Menu::Row(index) => {
                let Some(mail) = self.mails.get(index) else {
                    return;
                };
                let archivable = matches!(
                    self.mailbox,
                    View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social
                );
                let mut actions = vec![
                    if mail.unread {
                        RowAction::MarkRead
                    } else {
                        RowAction::MarkUnread
                    },
                    if mail.starred {
                        RowAction::Unstar
                    } else {
                        RowAction::Star
                    },
                ];
                if archivable {
                    actions.push(RowAction::Done);
                }
                if self.mailbox == View::Done {
                    actions.push(RowAction::MoveToInbox);
                }
                actions.push(RowAction::Trash);
                if !self.is_demo() {
                    actions.push(RowAction::ReportSpam);
                }
                if !self.is_demo() {
                    actions.push(RowAction::OpenInGmail);
                }
                let items = actions
                    .iter()
                    .map(|action| {
                        let (name, label) = action.label();
                        (name, label.into(), t.text2)
                    })
                    .collect();
                self.row_actions = actions;
                items
            }
        };
        let enabled = !(menu == Menu::Labels && self.is_demo())
            && !(menu == Menu::MoveTo && self.move_targets.is_empty());

        // Where the menu goes, in window coordinates: under its button.
        let viewport = window.viewport_size();
        let pane_left = if self.sidebar_open { 236. } else { 0. }
            + if self.list_open { self.list_width } else { 0. };
        let (width, height) = (MENU_WIDTH, items.len() as f32 * 32. + 14.);
        let (left, top) = match menu {
            Menu::Labels => (pane_left + 150., 50.),
            Menu::MoveTo => (pane_left + 186., 50.),
            // Above the account row at the bottom of the sidebar.
            Menu::Accounts => (14., f32::from(viewport.height) - 64. - height),
            Menu::More => (f32::from(viewport.width) - 20. - width, 50.),
            // Above the composer's Send later button.
            Menu::SendLater => {
                let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
                match &self.compose {
                    Some(c) if !c.full => {
                        let (w, h) = c.size;
                        let (l, t) = c.position.unwrap_or((((vw - w) / 2.).max(0.), 96.));
                        (l + w - width - 90., t + h - height - 56.)
                    }
                    _ => (vw - width - 110., vh - height - 64.),
                }
            }
            // At the pointer, nudged back inside the window if it would hang over an edge.
            Menu::Row(_) => (
                self.menu_at.0.min(f32::from(viewport.width) - width - 8.),
                self.menu_at.1.min(f32::from(viewport.height) - height - 8.),
            ),
        };
        let origin = window.bounds().origin;
        let bounds = gpui::Bounds::new(
            gpui::point(
                origin.x + px(left - MENU_MARGIN),
                origin.y + px(top - MENU_MARGIN),
            ),
            gpui::size(px(width + 2. * MENU_MARGIN), px(height + 2. * MENU_MARGIN)),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                focus: true,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|cx| {
                    // Close on any click outside the menu. Polled, because clicks on the
                    // email's web view or on another app never reach this app's handlers.
                    cx.spawn_in(window, async move |this, cx| {
                        // The click that opened the menu may still be held down (a
                        // right-click opens it on the way down). Only a press that starts
                        // after every button has been let go counts as an outside click.
                        let mut armed = false;
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(30))
                                .await;
                            if !crate::blur::mouse_down() {
                                armed = true;
                            }
                            let open = this.update_in(cx, |this: &mut MenuPopup, window, cx| {
                                if armed
                                    && crate::blur::mouse_pressed_outside(
                                        window,
                                        MENU_MARGIN as f64,
                                    )
                                {
                                    this.dismiss(window, cx);
                                    false
                                } else {
                                    true
                                }
                            });
                            if !matches!(open, Ok(true)) {
                                break;
                            }
                        }
                    })
                    .detach();
                    MenuPopup {
                        owner,
                        menu,
                        items,
                        enabled,
                        dark,
                        was_active: false,
                        opened: std::time::Instant::now(),
                        // Losing focus (switching apps with the keyboard, say) closes it too.
                        _activation: cx.observe_window_activation(
                            window,
                            |this: &mut MenuPopup, window, cx| {
                                if window.is_window_active() {
                                    this.was_active = true;
                                } else if this.was_active
                                    && this.opened.elapsed() > Duration::from_millis(500)
                                {
                                    // Focus moving away right as the menu opens is part of
                                    // the click that opened it, not a reason to close.
                                    this.dismiss(window, cx);
                                }
                            },
                        ),
                    }
                })
            },
        );
        match opened {
            Ok(handle) => self.menu_window = Some(handle),
            // No pop-up windows here: fall back to the menu drawn inside the pane.
            Err(_) => self.show_drawn_menu(menu, cx),
        }
    }

    fn close_menu_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.menu_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    fn menu_choice(&mut self, menu: Menu, index: usize, cx: &mut Context<Self>) {
        self.menu_window = None;
        match (menu, index) {
            (Menu::Labels, index) if index == self.labels.len().min(12) => self.open_new_label(cx),
            (Menu::Labels, index) => self.apply_label(index, cx),
            (Menu::Accounts, index) => {
                let accounts = self.settings.accounts.len();
                if index < accounts {
                    let id = self.settings.accounts[index].id.clone();
                    self.switch_account(id, cx);
                } else if index == accounts {
                    self.add_account(cx);
                } else {
                    self.open_settings(cx);
                }
            }
            (Menu::MoveTo, index) => {
                if let Some(target) = self.move_targets.get(index).cloned() {
                    self.move_selected(target, cx);
                }
            }
            (Menu::More, 0) => self.mark_unread(cx),
            (Menu::More, 1) => self.open_in_gmail(cx),
            (Menu::More, _) => self.remove_selected(gmail::Change::Spam, cx),
            (Menu::SendLater, index) => {
                if let Some(at) = self.send_later_times.get(index).copied() {
                    self.schedule_send(at, cx);
                }
            }
            (Menu::Row(row), index) => {
                if let Some(action) = self.row_actions.get(index).copied() {
                    self.row_action(row, action, cx);
                }
            }
        }
    }

    fn show_drawn_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let just_dismissed = self.menu_dismissed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(400)
        });
        self.menu = if self.menu == Some(menu) || just_dismissed {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    /// Gmail could not be reached: remember what is waiting, and keep checking.
    fn went_offline(&mut self, cx: &mut Context<Self>) {
        self.offline = true;
        self.save_outbox(cx);
        if self.retrying {
            cx.notify();
            return;
        }
        self.retrying = true;
        eprintln!("nimbra: offline");
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(15)).await;
                let Ok(Some((client, outbox))) =
                    this.update(cx, |this, _| this.gmail.clone().map(|c| (c, this.outbox.clone())))
                else {
                    break;
                };
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        // Is Gmail reachable again?
                        if let Err(error) = client.profile() {
                            return Err(error);
                        }
                        let mut changes = 0;
                        for (id, change) in &outbox.changes {
                            match client.change(id, *change) {
                                Err(error) if error.is::<gmail::Offline>() => break,
                                // Anything else (the thread is gone, say) is dropped.
                                _ => changes += 1,
                            }
                        }
                        let mut sent = 0;
                        let mut kept = Vec::new();
                        if changes == outbox.changes.len() {
                            for item in &outbox.mail {
                                let result = match &item.draft_id {
                                    Some(id) => client.send_draft(id, &item.outgoing),
                                    None => client.send_message(&item.outgoing),
                                };
                                match result {
                                    Err(error) if error.is::<gmail::Offline>() => break,
                                    Err(_) => {
                                        // Not lost: kept as a draft to look at.
                                        let _ = client.save_draft(item.draft_id.as_deref(), &item.outgoing);
                                        kept.push(item.outgoing.subject.clone());
                                    }
                                    Ok(()) => {}
                                }
                                sent += 1;
                            }
                        }
                        anyhow::Ok((changes, sent, kept))
                    })
                    .await;
                let done = this.update(cx, |this, cx| {
                    match result {
                        Ok((changes, sent, kept)) => {
                            this.outbox.changes.drain(..changes.min(this.outbox.changes.len()));
                            this.outbox.mail.drain(..sent.min(this.outbox.mail.len()));
                            this.save_outbox(cx);
                            if this.outbox.changes.is_empty() && this.outbox.mail.is_empty() {
                                this.offline = false;
                                this.retrying = false;
                                eprintln!("nimbra: back online ({changes} changes, {sent} messages caught up)");
                                this.notice = Some(if !kept.is_empty() {
                                    format!("Back online. “{}” could not be sent; it is in Drafts.", short(&kept[0], 40)).into()
                                } else if sent > 0 {
                                    format!("Back online. {sent} waiting message{} sent.", if sent == 1 { " was" } else { "s were" }).into()
                                } else {
                                    "Back online.".into()
                                });
                                this.refresh(cx);
                                cx.notify();
                                return true;
                            }
                        }
                        Err(error) if error.is::<gmail::SignedOut>() => {
                            this.retrying = false;
                            this.fail(error, cx);
                            return true;
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                    false
                });
                if !matches!(done, Ok(false)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn save_outbox(&self, cx: &mut Context<Self>) {
        let (Some(store), outbox) = (self.store.clone(), self.outbox.clone()) else {
            return;
        };
        cx.background_executor()
            .spawn(async move {
                if store.save("outbox", &outbox).is_err() {
                    eprintln!("nimbra: could not save the offline queue");
                }
            })
            .detach();
    }

    /// ⌘P: prints the open message with its sender, recipients and date.
    fn print_selected(&mut self, cx: &mut Context<Self>) {
        let Some(mail) = self
            .mails
            .get(self.selected)
            .filter(|_| self.visible().contains(&self.selected))
        else {
            return;
        };
        if !mail.loaded {
            self.notice = Some("The message is still loading. Try again in a moment.".into());
            cx.notify();
            return;
        }
        let h = &mail.header;
        let person = |p: &gmail::Person| {
            if p.name.is_empty() {
                p.address.clone()
            } else {
                format!("{} <{}>", p.name, p.address)
            }
        };
        let list =
            |people: &[gmail::Person]| people.iter().map(person).collect::<Vec<_>>().join(", ");
        let mut doc = format!(
            "<div style=\"font:13px/1.5 -apple-system,sans-serif;border-bottom:1px solid #ccc;padding-bottom:10px;margin-bottom:16px\"><div style=\"font-size:18px;font-weight:600;margin-bottom:6px\">{}</div>",
            esc(&mail.subject)
        );
        let from = if h.from.address.is_empty() {
            mail.sender.to_string()
        } else {
            person(&h.from)
        };
        let mut row = |name: &str, value: String| {
            if !value.is_empty() {
                doc.push_str(&format!("<div><b>{name}:</b> {}</div>", esc(&value)));
            }
        };
        row("From", from);
        row("To", list(&h.to));
        row("Cc", list(&h.cc));
        row("Date", long_date(h.date_ms));
        doc.push_str("</div>");
        match &mail.html {
            Some(html) => doc.push_str(html),
            None => {
                for block in &mail.body {
                    doc.push_str(&format!(
                        "<p>{}</p>",
                        esc(&block.text).replace('\n', "<br>")
                    ));
                }
            }
        }
        self.print_pending = Some(doc);
        cx.notify();
        // The document is loaded on the next frame; give it a moment, then print.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(700))
                .await;
            this.update(cx, |this, _| {
                if let Some(view) = &this.html_view {
                    view.print();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The rows an action applies to: the picked ones (in list order), or the open one.
    fn targets(&self) -> Vec<usize> {
        let visible = self.visible();
        if self.picked.is_empty() {
            visible
                .into_iter()
                .filter(|i| *i == self.selected)
                .collect()
        } else {
            visible
                .into_iter()
                .filter(|i| self.picked.contains(i))
                .collect()
        }
    }

    /// A click on a row: plain opens it; ⌘ adds or removes it from the picked rows;
    /// Shift picks everything from the open row to this one.
    fn click_row(&mut self, i: usize, modifiers: gpui::Modifiers, cx: &mut Context<Self>) {
        if modifiers.platform || modifiers.control {
            if self.picked.is_empty() && self.visible().contains(&self.selected) {
                self.picked.insert(self.selected);
            }
            if !self.picked.remove(&i) {
                self.picked.insert(i);
            }
            if self.picked.len() == 1 {
                let only = *self.picked.iter().next().unwrap_or(&i);
                self.picked.clear();
                self.open(only);
                self.open_selected(cx);
            }
        } else if modifiers.shift {
            let visible = self.visible();
            let (Some(a), Some(b)) = (
                visible.iter().position(|x| *x == self.selected),
                visible.iter().position(|x| *x == i),
            ) else {
                return;
            };
            self.picked = visible[a.min(b)..=a.max(b)].iter().copied().collect();
        } else {
            self.picked.clear();
            self.open(i);
            self.open_selected(cx);
        }
        cx.notify();
    }

    /// Takes every picked thread out of the list at once; one Undo brings them all back.
    fn remove_picked(&mut self, change: gmail::Change, cx: &mut Context<Self>) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        for &index in &targets {
            self.done[index] = true;
            self.history.push(index);
            if let Some(id) = self.mails[index].id.clone() {
                self.removed_by.insert(id, change);
            }
            self.sync(index, change, true, cx);
        }
        self.history_groups.push(targets.len());
        self.picked.clear();
        self.selected = self.visible().first().copied().unwrap_or(0);
        self.open_selected(cx);
        self.offer_undo(
            match change {
                gmail::Change::Trash => "Moved to trash",
                gmail::Change::Unarchive | gmail::Change::NotSpam => "Moved to inbox",
                gmail::Change::Spam => "Reported as spam",
                gmail::Change::Untrash => "Restored",
                _ => "Marked done",
            },
            cx,
        );
        cx.notify();
    }

    /// The times offered when snoozing.
    fn snooze_choices() -> Vec<SnoozeChoice> {
        use chrono::{Datelike, Duration as Days, Local, TimeZone, Timelike, Weekday};
        let now = Local::now();
        let at = |date: chrono::NaiveDate, hour: u32| {
            Local
                .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap_or_default())
                .single()
        };
        let label = |when: chrono::DateTime<Local>| when.format("%a %-d %b, %H:%M").to_string();
        let mut choices = Vec::new();
        let mut push =
            |icon, name: &str, when: Option<chrono::DateTime<Local>>, header, no_reply| {
                if let Some(when) = when {
                    choices.push(SnoozeChoice {
                        icon,
                        label: name.to_string(),
                        hint: label(when),
                        header,
                        until_ms: when.timestamp_millis(),
                        no_reply,
                    });
                }
            };
        let today = now.date_naive();
        if now.hour() < 17 {
            push("clock", "Later today", at(today, 18), Some("SNOOZE"), false);
        } else {
            push(
                "clock",
                "In three hours",
                Some(now + Days::hours(3)),
                Some("SNOOZE"),
                false,
            );
        }
        let tomorrow = today + Days::days(1);
        push("clock", "Tomorrow morning", at(tomorrow, 8), None, false);
        if !matches!(now.weekday(), Weekday::Fri | Weekday::Sat | Weekday::Sun) {
            let mut saturday = today;
            while saturday.weekday() != Weekday::Sat {
                saturday += Days::days(1);
            }
            push("calendar", "This weekend", at(saturday, 9), None, false);
        }
        let mut monday = today + Days::days(1);
        while monday.weekday() != Weekday::Mon {
            monday += Days::days(1);
        }
        push("calendar", "Next week", at(monday, 8), None, false);
        push(
            "bell",
            "Remind me if no reply in 3 days",
            at(today + Days::days(3), 9),
            Some("REMINDER"),
            true,
        );
        push(
            "bell",
            "Remind me if no reply in a week",
            at(today + Days::days(7), 9),
            None,
            true,
        );
        choices
    }

    /// Snoozes the open (or picked) threads: out of the inbox now, back at the chosen time,
    /// unread and with a notification. A reminder only comes back if nobody has replied.
    fn snooze(&mut self, choice: SnoozeChoice, cx: &mut Context<Self>) {
        let targets = self.targets();
        for &index in &targets {
            let mail = &self.mails[index];
            let Some(thread_id) = mail.id.clone() else {
                continue;
            };
            self.snoozed.retain(|s| s.thread_id != thread_id);
            self.snoozed.push(Snooze {
                thread_id,
                until_ms: choice.until_ms,
                no_reply: choice.no_reply,
                since_ms: mail.timestamp_ms,
                mail: mail.clone(),
            });
        }
        self.save_snoozed(cx);
        self.watch_schedule(cx);
        let inbox_like = matches!(
            self.mailbox,
            View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social
        );
        if inbox_like {
            self.remove_selected(gmail::Change::Archive, cx);
        }
        self.notice = Some(
            if choice.no_reply {
                format!("I’ll bring it back on {} if nobody replies.", choice.hint)
            } else {
                format!("Snoozed until {}.", choice.hint)
            }
            .into(),
        );
        cx.notify();
    }

    fn save_snoozed(&self, cx: &mut Context<Self>) {
        let (Some(store), snoozed) = (self.store.clone(), self.snoozed.clone()) else {
            return;
        };
        cx.background_executor()
            .spawn(async move {
                if store.save("snoozed", &snoozed).is_err() {
                    eprintln!("nimbra: could not save the snoozed list");
                }
            })
            .detach();
    }

    /// Brings back snoozed threads whose time has come (checked with the send-later list).
    fn wake_due(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            return;
        };
        let now = chrono::Utc::now().timestamp_millis();
        let due: Vec<Snooze> = self
            .snoozed
            .iter()
            .filter(|s| s.until_ms <= now && !self.waking.contains(&s.thread_id))
            .cloned()
            .collect();
        for item in due {
            self.waking.insert(item.thread_id.clone());
            let client = client.clone();
            cx.spawn(async move |this, cx| {
                let (id, since, no_reply) = (item.thread_id.clone(), item.since_ms, item.no_reply);
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        if no_reply && client.summary(&id, false)?.timestamp_ms > since {
                            return anyhow::Ok(false); // someone wrote: no reminder needed
                        }
                        client.relabel(&id, &["INBOX", "UNREAD"], &[])?;
                        anyhow::Ok(true)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.waking.remove(&item.thread_id);
                    let done = match &result {
                        Ok(back) => {
                            if *back {
                                let title = if item.no_reply {
                                    "No reply yet"
                                } else {
                                    "Back from snooze"
                                };
                                if this.settings.notifications {
                                    crate::notify::show(&item.thread_id, title, &item.mail.subject);
                                }
                                this.notice = Some(
                                    format!("{title}: {}", short(&item.mail.subject, 50)).into(),
                                );
                                if matches!(
                                    this.mailbox,
                                    View::Inbox | View::Primary | View::Snoozed
                                ) {
                                    this.refresh(cx);
                                }
                            }
                            true
                        }
                        Err(error) => error.to_string().contains("404"),
                    };
                    if done {
                        this.snoozed.retain(|s| s.thread_id != item.thread_id);
                        this.save_snoozed(cx);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Several threads picked: what can be done to all of them.
    fn bulk_pane(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.targets().len();
        let action = |id: &'static str, name: &'static str, label: &'static str| {
            button(t, ButtonKind::Secondary, label, None)
                .id(id)
                .child(icon(name, 14., t.text2))
        };
        let inbox_like = matches!(
            self.mailbox,
            View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social
        );
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(18.))
            .bg(t.surface)
            .child(serif(
                &format!("{count} conversations, selected"),
                44.,
                t.text,
            ))
            .child(meta(
                "⌘-click adds or removes one, Shift-click picks a range, Esc clears.",
                t.text3,
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .max_w(px(620.))
                    .when(
                        inbox_like
                            || matches!(
                                self.mailbox,
                                View::Done | View::Spam | View::Trash | View::Snoozed
                            ),
                        |d| {
                            d.child(
                                action(
                                    "bulk-done",
                                    "done",
                                    if inbox_like {
                                        "Mark done"
                                    } else {
                                        "Move to inbox"
                                    },
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                            )
                        },
                    )
                    .child(
                        action("bulk-read", "mail", "Read or unread")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
                    )
                    .child(
                        action("bulk-star", "star", "Star")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(action("bulk-label", "tag", "Label").on_click(cx.listener(
                        |this, _, window, cx| this.toggle_menu(Menu::Labels, window, cx),
                    )))
                    .child(
                        action("bulk-move", "folder", "Move to").on_click(cx.listener(
                            |this, _, window, cx| this.toggle_menu(Menu::MoveTo, window, cx),
                        )),
                    )
                    .when(inbox_like && !self.is_demo(), |d| {
                        d.child(
                            action("bulk-snooze", "clock", "Snooze").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.snooze_choices = Self::snooze_choices();
                                    this.palette = true;
                                    this.palette_sel = 0;
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .when(self.mailbox != View::Spam && !self.is_demo(), |d| {
                        d.child(action("bulk-spam", "ban", "Spam").on_click(cx.listener(
                            |this, _, _, cx| this.remove_selected(gmail::Change::Spam, cx),
                        )))
                    })
                    .child(
                        action("bulk-trash", "trash", "Trash")
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    ),
            )
            .child(
                button(t, ButtonKind::Ghost, "Clear selection", Some("esc"))
                    .id("bulk-clear")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picked.clear();
                        cx.notify();
                    })),
            )
    }

    /// "Load more" at the end of the list: the next page of the mailbox.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(page)) = (self.gmail.clone(), self.next_page.clone()) else {
            return;
        };
        if self.loading_more {
            return;
        }
        self.loading_more = true;
        cx.notify();
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            View::Search => Some(self.query.clone()),
            _ => None,
        };
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { client.threads_page(view, label_id.as_deref(), 30, Some(&page)) },
                )
                .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                this.loading_more = false;
                if this.mailbox != view {
                    return;
                }
                match result {
                    Ok((threads, next)) => {
                        this.next_page = next;
                        let known: HashSet<String> =
                            this.mails.iter().filter_map(|m| m.id.clone()).collect();
                        let fresh: Vec<Mail> = threads
                            .into_iter()
                            .filter(|s| !known.contains(&s.id))
                            .map(Mail::from_summary)
                            .collect();
                        this.done.extend(std::iter::repeat_n(false, fresh.len()));
                        this.mails.extend(fresh);
                        this.cache.insert(this.mailbox, this.mails.clone());
                        this.persist(cx);
                    }
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The user labels on a thread, with their position in the label list (for a tint).
    fn thread_labels(&self, mail: &Mail) -> Vec<(usize, gmail::Label)> {
        self.labels
            .iter()
            .enumerate()
            .filter(|(_, label)| mail.labels.iter().any(|id| *id == label.id))
            .map(|(index, label)| {
                (
                    index,
                    gmail::Label {
                        id: label.id.clone(),
                        name: label.name.clone(),
                    },
                )
            })
            .collect()
    }

    /// Takes a label off the open thread.
    fn remove_label(&mut self, label_id: String, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get_mut(self.selected) else {
            return;
        };
        mail.labels.retain(|id| *id != label_id);
        let (Some(client), Some(id)) = (self.gmail.clone(), mail.id.clone()) else {
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.remove_label(&id, &label_id) })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| this.fail(error, cx)).ok();
            }
        })
        .detach();
        cx.notify();
    }

    /// Opens the New label sheet.
    fn open_new_label(&mut self, cx: &mut Context<Self>) {
        if self.gmail.is_none() {
            self.notice = Some("Labels need a connected Gmail account.".into());
            cx.notify();
            return;
        }
        let t = self.theme();
        let input = cx.new(|cx| TextInput::new(cx, "Label name", t.faint, t.violet));
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Enter) {
                this.create_label(cx);
            }
        });
        self.pending_focus = Some(input.read(cx).focus_handle(cx));
        self.new_label = Some(input);
        self._new_label_subscription = Some(subscription);
        if let Some(view) = &self.html_view {
            view.set_visible(false);
        }
        cx.notify();
    }

    fn close_new_label(&mut self, cx: &mut Context<Self>) {
        self.new_label = None;
        self._new_label_subscription = None;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Creates the label typed in the sheet and puts it on the open thread.
    fn create_label(&mut self, cx: &mut Context<Self>) {
        let (Some(input), Some(client)) = (self.new_label.clone(), self.gmail.clone()) else {
            return;
        };
        let name = input.read(cx).text().trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(existing) = self
            .labels
            .iter()
            .position(|l| l.name.eq_ignore_ascii_case(&name))
        {
            self.close_new_label(cx);
            self.apply_label(existing, cx);
            return;
        }
        self.close_new_label(cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.create_label(&name) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(label) => {
                    let id = label.id.clone();
                    this.labels.push(label);
                    this.labels.sort_by_key(|l| l.name.to_lowercase());
                    if let Some(index) = this.labels.iter().position(|l| l.id == id) {
                        this.apply_label(index, cx);
                    }
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The New label sheet.
    fn new_label_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.new_label.clone();
        div()
            .id("new-label-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_new_label(cx)),
            )
            .child(
                div()
                    .id("new-label")
                    .w(px(400.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .p(px(22.))
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(
                        c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                        30.,
                        80.,
                        -10.,
                    )])
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(serif("New label", 28., t.text))
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(px(10.))
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border)
                            .text_size(px(14.))
                            .children(input),
                    )
                    .child(meta(
                        "It is added to this conversation and to Gmail’s label list.",
                        t.text3,
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", None)
                                    .id("label-cancel")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_new_label(cx)),
                                    ),
                            )
                            .child(
                                button(t, ButtonKind::Primary, "Create", Some("↵"))
                                    .id("label-create")
                                    .on_click(cx.listener(|this, _, _, cx| this.create_label(cx))),
                            ),
                    ),
            )
    }

    /// Move to: back to the inbox, into a label (out of the inbox), or to Spam.
    fn move_selected(&mut self, target: MoveTarget, cx: &mut Context<Self>) {
        let inbox_like = matches!(
            self.mailbox,
            View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social
        );
        match target {
            MoveTarget::Spam => self.remove_selected(gmail::Change::Spam, cx),
            MoveTarget::Inbox => match self.mailbox {
                View::Spam => self.remove_selected(gmail::Change::NotSpam, cx),
                View::Trash => self.remove_selected(gmail::Change::Untrash, cx),
                _ => self.remove_selected(gmail::Change::Unarchive, cx),
            },
            MoveTarget::Label(label_id) => {
                let Some(index) = self.labels.iter().position(|l| l.id == label_id) else {
                    return;
                };
                let thread = self.mails.get(self.selected).and_then(|m| m.id.clone());
                self.apply_label(index, cx);
                if inbox_like {
                    if let Some(thread) = thread {
                        self.moved.insert(thread, label_id);
                    }
                    self.remove_selected(gmail::Change::Archive, cx);
                }
            }
        }
    }

    /// Adds one of the user's labels to the open thread.
    fn apply_label(&mut self, label: usize, cx: &mut Context<Self>) {
        self.menu = None;
        if self.picked.len() >= 2 {
            let selected = self.selected;
            let targets = self.targets();
            let picked = std::mem::take(&mut self.picked);
            for index in targets {
                self.selected = index;
                self.apply_label(label, cx);
            }
            self.selected = selected;
            self.picked = picked;
            return;
        }
        if let (Some(label), Some(mail)) =
            (self.labels.get(label), self.mails.get_mut(self.selected))
            && !mail.labels.contains(&label.id)
        {
            mail.labels.push(label.id.clone());
        }
        let (Some(label), Some(mail)) = (
            self.labels.get(label).cloned(),
            self.mails.get(self.selected),
        ) else {
            return;
        };
        let (Some(client), Some(id)) = (self.gmail.clone(), mail.id.clone()) else {
            return;
        };
        let name = label.name.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.add_label(&id, &label.id) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.notice = Some(format!("Labelled “{}”.", short(&name, 30)).into());
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// `U`, or "Mark as unread" in a menu: the open thread goes back to unread.
    fn mark_unread(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.set_unread(self.selected, true, cx);
    }

    /// The toolbar button and `U`: flips the open thread between unread and read.
    fn toggle_unread(&mut self, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            let targets = self.targets();
            let unread = !targets.iter().all(|i| self.mails[*i].unread);
            for index in targets {
                self.set_unread(index, unread, cx);
            }
            return;
        }
        let index = self.selected;
        if let Some(unread) = self.mails.get(index).map(|m| m.unread) {
            self.set_unread(index, !unread, cx);
        }
    }

    /// Marks a thread read or unread, here and in Gmail.
    fn set_unread(&mut self, index: usize, unread: bool, cx: &mut Context<Self>) {
        if self.visible().contains(&index) && self.mails[index].unread != unread {
            self.mails[index].unread = unread;
            let change = if unread {
                gmail::Change::MarkUnread
            } else {
                gmail::Change::MarkRead
            };
            self.sync(index, change, false, cx);
        }
        cx.notify();
    }

    /// Downloads an attachment, then either previews it with Quick Look (from a private
    /// temporary folder) or saves it to Downloads and shows it in Finder. Nothing is opened
    /// with another app automatically.
    fn fetch_attachment(&mut self, att: gmail::Attachment, save: bool, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            self.notice = Some("Attachments need a connected Gmail account.".into());
            cx.notify();
            return;
        };
        self.notice = Some(format!("Downloading {}…", short(&att.name, 40)).into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = client.download(&att.message_id, &att.attachment_id)?;
                    let name = gmail::safe_file_name(&att.name);
                    let path = if save {
                        unique_download_path(&name)?
                    } else {
                        preview_path(&name)?
                    };
                    std::fs::write(&path, &bytes)?;
                    restrict_file(&path);
                    anyhow::Ok(path)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(path) => {
                        let tool = if save {
                            ("/usr/bin/open", "-R")
                        } else {
                            ("/usr/bin/qlmanage", "-p")
                        };
                        let _ = std::process::Command::new(tool.0)
                            .arg(tool.1)
                            .arg(&path)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                        this.notice = save.then(|| {
                            format!(
                                "Saved to Downloads: {}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            )
                            .into()
                        });
                    }
                    Err(error) => {
                        this.notice = Some(format!("Could not get the attachment: {error}").into())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Leaves a mailing list: an https link opens in the browser (where the list asks for
    /// confirmation); a mailto address opens a prepared message in the composer.
    fn unsubscribe(&mut self, link: String, cx: &mut Context<Self>) {
        if link.to_ascii_lowercase().starts_with("https://") {
            cx.open_url(&link);
            return;
        }
        let rest = &link["mailto:".len().min(link.len())..];
        let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
        let subject = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("subject="))
            .map(|s| s.replace("%20", " ").replace('+', " "))
            .unwrap_or_else(|| "unsubscribe".to_string());
        self.open_compose(ComposeKind::New, cx);
        if let Some(composer) = &self.compose {
            let (to, subj) = (composer.to.clone(), composer.subject.clone());
            let address = address.to_string();
            to.update(cx, |input, cx| input.set(address, usize::MAX, cx));
            if let Some(composer) = &mut self.compose {
                composer.chipify(Field::To, true, cx);
            }
            subj.update(cx, |input, cx| input.set(subject, usize::MAX, cx));
        }
    }

    /// Opens the right-click menu for a row of the list.
    fn open_row_menu(
        &mut self,
        index: usize,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_at = (f32::from(at.x), f32::from(at.y));
        // A second right-click should always open the menu again, never just close it.
        self.menu_dismissed = None;
        self.toggle_menu(Menu::Row(index), window, cx);
    }

    /// Runs one action from a row's right-click menu on that row.
    fn row_action(&mut self, row: usize, action: RowAction, cx: &mut Context<Self>) {
        if !self.visible().contains(&row) {
            return;
        }
        match action {
            RowAction::MarkRead => self.set_unread(row, false, cx),
            RowAction::MarkUnread => self.set_unread(row, true, cx),
            RowAction::Star | RowAction::Unstar => {
                let starred = action == RowAction::Star;
                self.mails[row].starred = starred;
                let change = if starred {
                    gmail::Change::Star
                } else {
                    gmail::Change::Unstar
                };
                self.sync(row, change, false, cx);
            }
            // These take the row out of the list, which works on the open thread.
            RowAction::Done | RowAction::MoveToInbox | RowAction::Trash | RowAction::ReportSpam => {
                self.selected = row;
                match action {
                    RowAction::Trash => self.trash(cx),
                    RowAction::ReportSpam => self.remove_selected(gmail::Change::Spam, cx),
                    _ => self.mark_done(cx),
                }
            }
            RowAction::OpenInGmail => {
                if let Some(id) = self.mails[row].id.as_ref()
                    && id.chars().all(|c| c.is_ascii_alphanumeric())
                {
                    cx.open_url(&format!("https://mail.google.com/mail/u/0/#all/{id}"));
                }
            }
        }
        cx.notify();
    }

    fn open_in_gmail(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        // Thread ids are validated as plain alphanumerics before they get here.
        if let Some(id) = self.mails.get(self.selected).and_then(|m| m.id.as_ref())
            && id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            cx.open_url(&format!("https://mail.google.com/mail/u/0/#all/{id}"));
        }
        cx.notify();
    }

    fn toggle_full_compose(&mut self, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.compose {
            composer.full = !composer.full;
            cx.notify();
        }
    }

    /// Opens the draft behind a row of the Drafts mailbox in the composer.
    fn open_draft(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(client), Some(thread_id)) = (
            self.gmail.clone(),
            self.mails.get(index).and_then(|m| m.id.clone()),
        ) else {
            return;
        };
        if self.compose.is_some() {
            return;
        }
        self.notice = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.draft_for_thread(&thread_id) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(draft) => this.edit_draft(draft, cx),
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    fn edit_draft(&mut self, draft: gmail::Draft, cx: &mut Context<Self>) {
        self.open_compose(ComposeKind::New, cx);
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer
            .to
            .update(cx, |input, cx| input.set(draft.to, usize::MAX, cx));
        composer
            .subject
            .update(cx, |input, cx| input.set(draft.subject, usize::MAX, cx));
        // One field per line of the part the user wrote.
        for (n, text) in draft.body.lines().take(400).enumerate() {
            let text = text.to_string();
            if n == 0 {
                composer.lines[0].update(cx, |input, cx| input.set(text, 0, cx));
            } else {
                let (line, subscription) = self.new_field(Field::Line(n), "", cx);
                line.update(cx, |input, cx| input.set(text, 0, cx));
                composer.lines.push(line);
                composer._subscriptions.push(subscription);
            }
        }
        composer.show_cc = !draft.cc.trim().is_empty() || !draft.bcc.trim().is_empty();
        let (cc, bcc) = (draft.cc, draft.bcc);
        composer
            .cc
            .update(cx, |input, cx| input.set(cc, usize::MAX, cx));
        composer
            .bcc
            .update(cx, |input, cx| input.set(bcc, usize::MAX, cx));
        for field in [Field::To, Field::Cc, Field::Bcc] {
            composer.chipify(field, true, cx);
        }
        composer.kept = draft.attachments;
        let html = draft
            .html
            .as_deref()
            .and_then(|html| {
                if draft.quoted.is_some() {
                    crate::editor::own_html(html)
                } else {
                    Some(html.to_string())
                }
            })
            .unwrap_or_else(|| crate::editor::text_to_html(&draft.body));
        composer.load = Some(html);
        composer.focus_body = composer.rich;
        composer.draft_id = Some(draft.id);
        composer.quoted = draft.quoted;
        composer.reply = draft.reply;
        composer.full = true;
        self.pending_focus = Some(if composer.rich {
            self.editor_focus.clone()
        } else {
            composer.lines[0].read(cx).focus_handle(cx)
        });
        self.compose = Some(composer);
        cx.notify();
    }

    /// Saves the composer's contents to Gmail's Drafts without sending anything.
    /// ⌘S: saves the draft now.
    fn save_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(serial) = self.compose.as_ref().map(|c| c.serial) {
            self.save_composer(serial, true, cx);
        }
    }

    /// The composer with this serial, on screen or parked.
    fn composer_mut(&mut self, serial: u64) -> Option<&mut Composer> {
        if self.compose.as_ref().is_some_and(|c| c.serial == serial) {
            return self.compose.as_mut();
        }
        self.parked
            .iter_mut()
            .find(|(_, c)| c.serial == serial)
            .map(|(_, c)| c)
    }

    /// Something in the composer changed: save the draft once typing pauses.
    fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        composer.edits += 1;
        composer.saved = false;
        let (serial, edits) = (composer.serial, composer.edits);
        // Attachments make every save an upload, so those wait longer.
        let pause = if composer.files.is_empty() && composer.kept.is_empty() {
            2500
        } else {
            10_000
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(pause))
                .await;
            this.update(cx, |this, cx| {
                if this
                    .compose
                    .as_ref()
                    .is_some_and(|c| c.serial == serial && c.edits == edits)
                {
                    this.save_composer(serial, false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Saves a composer's message as a Gmail draft (new, or over the one it has).
    /// `explicit` is ⌘S or a step that needs the draft (Send later); otherwise only unsaved
    /// edits with something in them are saved.
    fn save_composer(&mut self, serial: u64, explicit: bool, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            if explicit && let Some(composer) = self.composer_mut(serial) {
                composer.error = Some("Sample mode: drafts are not saved.".into());
                cx.notify();
            }
            return;
        };
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        if composer.sending {
            return;
        }
        if composer.saving {
            composer.resave = true;
            return;
        }
        if !explicit
            && composer.after_save.is_none()
            && (!composer.dirty() || !composer.has_content(cx))
        {
            return;
        }
        let outgoing = composer.outgoing(cx);
        let draft_id = composer.draft_id.clone();
        let edits = composer.edits;
        composer.saving = true;
        composer.resave = false;
        if explicit {
            composer.error = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.save_draft(draft_id.as_deref(), &outgoing) })
                .await;
            this.update(cx, |this, cx| {
                this.composer_saved(serial, edits, explicit, result, cx)
            })
            .ok();
        })
        .detach();
    }

    fn composer_saved(
        &mut self,
        serial: u64,
        edits: u64,
        explicit: bool,
        result: anyhow::Result<String>,
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Err(error) if error.is::<gmail::SignedOut>() => {
                self.compose = None;
                self.parked.clear();
                self.fail(error, cx);
                return;
            }
            other => other,
        };
        let kind = self
            .parked
            .iter()
            .find(|(_, c)| c.serial == serial)
            .map(|(k, _)| *k);
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        composer.saving = false;
        let ok = match result {
            Ok(id) => {
                composer.draft_id = Some(id);
                composer.saved_edits = composer.saved_edits.max(edits);
                composer.saved = !composer.dirty();
                true
            }
            Err(error) => {
                composer.error = Some(format!("The draft was not saved: {error}").into());
                composer.after_save = None;
                false
            }
        };
        let again = ok && (composer.dirty() || std::mem::take(&mut composer.resave));
        let after = if ok && !again {
            composer.after_save.take()
        } else {
            None
        };
        let mut refresh = explicit && ok;
        match (kind, after) {
            (Some(_), Some(After::Schedule(at))) => {
                if let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) {
                    let (_, composer) = self.parked.remove(pos);
                    if let Some(draft_id) = composer.draft_id.clone() {
                        self.scheduled.push(Scheduled {
                            draft_id,
                            at_ms: at,
                            subject: composer.subject.read(cx).text(),
                        });
                        self.save_scheduled(cx);
                        self.notice = Some(format!("Scheduled for {}.", long_date(at)).into());
                        self.watch_schedule(cx);
                    }
                    refresh = true;
                }
            }
            (Some(Parked::Closing), _) if !ok => self.unpark(serial, cx),
            (Some(Parked::Closing), _) if again => self.save_composer(serial, false, cx),
            (Some(Parked::Closing), _) => {
                self.parked.retain(|(_, c)| c.serial != serial);
                refresh = true;
            }
            (None, _) if again => self.save_composer(serial, false, cx),
            _ => {}
        }
        if refresh && self.mailbox == View::Drafts {
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Brings a parked composer back on screen (when nothing else is open there).
    fn unpark(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) else {
            return;
        };
        if self.compose.is_some() {
            let (_, composer) = self.parked.remove(pos);
            let subject = composer.subject.read(cx).text();
            self.notice = Some(
                format!(
                    "“{}” needs attention: {}",
                    short(&subject, 40),
                    composer.error.clone().unwrap_or_default()
                )
                .into(),
            );
            return;
        }
        let (_, mut composer) = self.parked.remove(pos);
        composer.sending = false;
        if composer.rich {
            composer.focus_body = true;
            self.pending_focus = Some(self.editor_focus.clone());
        } else if let Some(first) = composer.lines.first() {
            self.pending_focus = Some(first.read(cx).focus_handle(cx));
        }
        self.compose = Some(composer);
        cx.notify();
    }

    /// Esc: closes the composer. Its last edits are still saved to Drafts.
    fn close_compose(&mut self, cx: &mut Context<Self>) {
        self.pending_focus = Some(self.focus.clone());
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        let unsaved = composer.saving || (composer.dirty() && composer.has_content(cx));
        if self.gmail.is_some() && unsaved {
            let serial = composer.serial;
            self.parked.push((Parked::Closing, composer));
            self.save_composer(serial, false, cx);
        }
        cx.notify();
    }

    /// The Discard button: closes the composer and deletes its draft, unless undone.
    fn discard_compose(&mut self, cx: &mut Context<Self>) {
        self.pending_focus = Some(self.focus.clone());
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        if !composer.has_content(cx) && composer.draft_id.is_none() && !composer.saving {
            cx.notify();
            return;
        }
        let serial = composer.serial;
        self.parked.push((Parked::Discarded, composer));
        self.offer_undo("Draft discarded", cx);
        self.finish_parked_later(serial, UNDO_DELAY, cx);
        cx.notify();
    }

    fn finish_parked_later(&mut self, serial: u64, delay: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| this.finish_parked(serial, cx))
                .ok();
        })
        .detach();
    }

    /// The undo delay of a sent or discarded message is over: send it, or delete its draft.
    fn finish_parked(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) else {
            return; // undone
        };
        if self.parked[pos].1.saving {
            // Wait for the draft save that is running, so nothing is left behind.
            self.finish_parked_later(serial, Duration::from_millis(400), cx);
            return;
        }
        match self.parked[pos].0 {
            Parked::Sending => {
                let (_, composer) = self.parked.remove(pos);
                self.dispatch(composer, cx);
            }
            Parked::Discarded => {
                let (_, composer) = self.parked.remove(pos);
                if let (Some(id), Some(client)) = (composer.draft_id.clone(), self.gmail.clone()) {
                    cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move { client.delete_draft(&id) })
                            .await;
                        this.update(cx, |this, cx| {
                            if let Err(error) = result {
                                this.notice =
                                    Some(format!("The draft was not deleted: {error}").into());
                            }
                            if this.mailbox == View::Drafts {
                                this.refresh(cx);
                            }
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            Parked::Closing | Parked::InFlight => {}
        }
    }

    /// Sends a message whose undo delay is over.
    fn dispatch(&mut self, mut composer: Composer, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            return;
        };
        let outgoing = composer.outgoing(cx);
        let draft_id = composer.draft_id.clone();
        let queued = QueuedMail {
            draft_id: draft_id.clone(),
            outgoing: outgoing.clone(),
        };
        let serial = composer.serial;
        composer.sending = true;
        self.parked.push((Parked::InFlight, composer));
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match draft_id {
                        Some(id) => client.send_draft(&id, &outgoing),
                        None => client.send_message(&outgoing),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.parked.retain(|(_, c)| c.serial != serial);
                        this.notice = Some("Message sent.".into());
                        if matches!(this.mailbox, View::Sent | View::Drafts) {
                            this.refresh(cx);
                        }
                    }
                    Err(error) if error.is::<gmail::SignedOut>() => {
                        this.parked.clear();
                        this.compose = None;
                        this.fail(error, cx);
                    }
                    Err(error) if error.is::<gmail::Offline>() => {
                        this.parked.retain(|(_, c)| c.serial != serial);
                        this.outbox.mail.push(queued);
                        this.notice = Some(
                            "You’re offline. The message will be sent when the connection is back."
                                .into(),
                        );
                        this.went_offline(cx);
                    }
                    Err(error) => {
                        if let Some(composer) = this.composer_mut(serial) {
                            composer.error = Some(format!("Not sent: {error}").into());
                        }
                        this.unpark(serial, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A recipient field or the message changed.
    fn field_changed(&mut self, field: Field, cx: &mut Context<Self>) {
        // A comma (or semicolon) finishes an address: it becomes a pill.
        if let Some(composer) = &mut self.compose
            && Composer::slot(field).is_some()
            && composer
                .input(field)
                .is_some_and(|input| input.read(cx).text().contains([',', ';']))
        {
            composer.chipify(field, false, cx);
        }
        let suggest = match field {
            Field::To | Field::Cc | Field::Bcc => self
                .compose
                .as_ref()
                .and_then(|c| c.input(field))
                .and_then(|input| {
                    let text = input.read(cx).text();
                    let token = text
                        .rsplit([',', ';'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_lowercase();
                    if token.is_empty() {
                        return None;
                    }
                    let present: Vec<String> = text
                        .split([',', ';'])
                        .map(|p| p.trim().to_lowercase())
                        .collect();
                    let people = self.match_contacts(&token, &present);
                    (!people.is_empty()).then_some(Suggest {
                        field,
                        people,
                        selected: 0,
                    })
                }),
            _ => None,
        };
        if let Some(composer) = &mut self.compose {
            composer.suggest = suggest;
        }
        self.composer_changed(cx);
        cx.notify();
    }

    /// People from the mail on this Mac whose name or address starts with `token`, most
    /// often seen first.
    fn match_contacts(&self, token: &str, present: &[String]) -> Vec<gmail::Person> {
        let me = self.account.to_lowercase();
        let mut seen: HashMap<String, (gmail::Person, u32)> = HashMap::new();
        let mut add = |p: &gmail::Person| {
            let key = p.address.to_lowercase();
            if key.is_empty() || key == me {
                return;
            }
            let entry = seen.entry(key).or_insert_with(|| (p.clone(), 0));
            entry.1 += 1;
            if entry.0.name.is_empty() && !p.name.is_empty() {
                entry.0.name = p.name.clone();
            }
        };
        for mail in self.mails.iter().chain(self.cache.values().flatten()) {
            for header in
                std::iter::once(&mail.header).chain(mail.earlier.iter().map(|e| &e.header))
            {
                add(&header.from);
                header.to.iter().for_each(&mut add);
                header.cc.iter().for_each(&mut add);
            }
        }
        let mut found: Vec<(gmail::Person, u32)> = seen
            .into_values()
            .filter(|(p, _)| {
                let address = p.address.to_lowercase();
                let name = p.name.to_lowercase();
                !present.contains(&address)
                    && (address.starts_with(token)
                        || name.starts_with(token)
                        || name.split_whitespace().any(|w| w.starts_with(token)))
            })
            .collect();
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.address.cmp(&b.0.address)));
        found.into_iter().take(5).map(|(p, _)| p).collect()
    }

    /// Puts the chosen suggestion in place of what was being typed.
    fn accept_suggestion(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        let Some(suggest) = composer.suggest.take() else {
            return;
        };
        let (Some(person), Some(input)) = (
            suggest
                .people
                .get(index.unwrap_or(suggest.selected))
                .cloned(),
            composer.input(suggest.field),
        ) else {
            return;
        };
        let text = input.read(cx).text();
        let cut = text.rfind([',', ';']).unwrap_or(0);
        let head = text[..cut].trim().to_string();
        input.update(cx, |input, cx| input.set(head, usize::MAX, cx));
        if let Some(slot) = Composer::slot(suggest.field)
            && !composer.chips[slot]
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&person.address))
        {
            composer.chips[slot].push(person.address.clone());
        }
        composer.sync_placeholder(suggest.field, cx);
        self.pending_focus = Some(input.read(cx).focus_handle(cx));
        self.composer_changed(cx);
        cx.notify();
    }

    fn move_suggestion(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(suggest) = self.compose.as_mut().and_then(|c| c.suggest.as_mut()) {
            let len = suggest.people.len() as isize;
            suggest.selected = (suggest.selected as isize + delta).rem_euclid(len) as usize;
            cx.notify();
        }
    }

    /// Opens the file picker to attach files.
    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                this.update(cx, |this, cx| this.attach_files(paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Reads picked or dropped files into the composer, within Gmail's 25 MB limit.
    fn attach_files(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        let serial = composer.serial;
        let used = composer
            .files
            .iter()
            .map(|f| f.bytes.len() as u64)
            .sum::<u64>()
            + composer.kept.iter().map(|a| a.size).sum::<u64>();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut files = Vec::new();
                    let mut total = used;
                    for path in paths.into_iter().take(20) {
                        let meta = std::fs::metadata(&path)?;
                        if !meta.is_file() {
                            anyhow::bail!("Folders can’t be attached. Zip them first.");
                        }
                        total += meta.len();
                        if total > gmail::ATTACHMENT_LIMIT {
                            anyhow::bail!("Attachments can be 25 MB in total.");
                        }
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let bytes = std::fs::read(&path)?;
                        files.push(gmail::OutgoingFile {
                            mime: gmail::mime_for(&name).into(),
                            name: gmail::safe_file_name(&name),
                            bytes,
                        });
                    }
                    anyhow::Ok(files)
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(composer) = this.composer_mut(serial) else {
                    return;
                };
                match result {
                    Ok(files) => {
                        composer.files.extend(files);
                        composer.error = None;
                    }
                    Err(error) => composer.error = Some(error.to_string().into()),
                }
                if this.compose.as_ref().is_some_and(|c| c.serial == serial) {
                    this.composer_changed(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The times offered by Send later, with their labels.
    fn send_later_choices() -> Vec<(String, i64)> {
        use chrono::{Datelike, Duration as Days, Local, TimeZone, Timelike, Weekday};
        let now = Local::now();
        let at = |date: chrono::NaiveDate, hour: u32| {
            Local
                .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap_or_default())
                .single()
                .map(|t| t.timestamp_millis())
        };
        let mut choices = Vec::new();
        let hour = (now + Days::hours(1)).with_second(0).unwrap_or(now);
        choices.push((
            format!("In an hour · {}", hour.format("%H:%M")),
            hour.timestamp_millis(),
        ));
        if now.hour() < 17
            && let Some(ms) = at(now.date_naive(), 18)
        {
            choices.push(("This evening · 18:00".into(), ms));
        }
        let tomorrow = now.date_naive() + Days::days(1);
        if let Some(ms) = at(tomorrow, 8) {
            choices.push((
                format!("Tomorrow morning · {} 8:00", tomorrow.format("%a")),
                ms,
            ));
        }
        let mut monday = now.date_naive() + Days::days(1);
        while monday.weekday() != Weekday::Mon {
            monday += Days::days(1);
        }
        if monday != tomorrow
            && let Some(ms) = at(monday, 8)
        {
            choices.push((
                format!("Monday morning · {} 8:00", monday.format("%b %-d")),
                ms,
            ));
        }
        choices
    }

    /// Send later: saves the draft, then sends it from this Mac at `at`.
    fn schedule_send(&mut self, at: i64, cx: &mut Context<Self>) {
        let demo = self.gmail.is_none();
        let Some(composer) = &mut self.compose else {
            return;
        };
        if demo {
            composer.error = Some("Sample mode: nothing is scheduled.".into());
            cx.notify();
            return;
        }
        let outgoing = composer.outgoing(cx);
        if let Err(error) = outgoing.recipients() {
            composer.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        composer.after_save = Some(After::Schedule(at));
        let (serial, saving) = (composer.serial, composer.saving);
        self.parked.push((Parked::Closing, composer));
        if !saving {
            self.save_composer(serial, true, cx);
        }
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    fn save_scheduled(&self, cx: &mut Context<Self>) {
        let (Some(store), scheduled) = (self.store.clone(), self.scheduled.clone()) else {
            return;
        };
        cx.background_executor()
            .spawn(async move {
                if store.save("scheduled", &scheduled).is_err() {
                    eprintln!("nimbra: could not save the send-later list");
                }
            })
            .detach();
    }

    /// Checks for scheduled messages that are due, every 20 seconds while Nimbra runs.
    fn watch_schedule(&mut self, cx: &mut Context<Self>) {
        self.send_due(cx);
        self.wake_due(cx);
        if self.schedule_watch {
            return;
        }
        self.schedule_watch = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(20))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.send_due(cx);
                        this.wake_due(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn send_due(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            return;
        };
        let now = chrono::Utc::now().timestamp_millis();
        let due: Vec<Scheduled> = self
            .scheduled
            .iter()
            .filter(|s| s.at_ms <= now && !self.sending_scheduled.contains(&s.draft_id))
            .cloned()
            .collect();
        for item in due {
            self.sending_scheduled.insert(item.draft_id.clone());
            let client = client.clone();
            cx.spawn(async move |this, cx| {
                let id = item.draft_id.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { client.send_saved_draft(&id) })
                    .await;
                this.update(cx, |this, cx| {
                    this.sending_scheduled.remove(&item.draft_id);
                    let done = match &result {
                        Ok(()) => {
                            this.notice = Some(
                                format!("Scheduled message sent: {}", short(&item.subject, 40))
                                    .into(),
                            );
                            true
                        }
                        // The draft was sent or deleted elsewhere.
                        Err(error) if error.to_string().contains("404") => {
                            this.notice = Some(
                                format!(
                                    "“{}” was not sent: its draft is gone.",
                                    short(&item.subject, 40)
                                )
                                .into(),
                            );
                            true
                        }
                        // Offline and the like: try again on the next check.
                        Err(_) => false,
                    };
                    if done {
                        this.scheduled.retain(|s| s.draft_id != item.draft_id);
                        this.save_scheduled(cx);
                        if matches!(this.mailbox, View::Sent | View::Drafts) {
                            this.refresh(cx);
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    fn focus_field(&mut self, field: Field, cursor: Option<usize>, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        if composer.rich && matches!(field, Field::Line(_)) {
            if let Some(editor) = &self.editor {
                editor.focus();
            }
            self.pending_focus = Some(self.editor_focus.clone());
            cx.notify();
            return;
        }
        if let Some(editor) = &self.editor {
            editor.blur();
        }
        let Some(input) = composer.input(field) else {
            return;
        };
        if let Some(composer) = &mut self.compose
            && composer.suggest.as_ref().is_some_and(|s| s.field != field)
        {
            composer.suggest = None;
        }
        if let Some(cursor) = cursor {
            input.update(cx, |input, cx| {
                let text = input.text();
                input.set(text, cursor, cx)
            });
        }
        self.pending_focus = Some(input.read(cx).focus_handle(cx));
        cx.notify();
    }

    /// Enter, arrows, Tab and Backspace-at-start make the separate fields behave like one
    /// form with a multi-line body.
    fn field_event(&mut self, field: Field, event: &InputEvent, cx: &mut Context<Self>) {
        let Some(line_count) = self.compose.as_ref().map(|c| c.lines.len()) else {
            return;
        };
        let show_cc = self.compose.as_ref().is_some_and(|c| c.show_cc);
        let suggesting = self
            .compose
            .as_ref()
            .and_then(|c| c.suggest.as_ref())
            .is_some_and(|s| s.field == field);
        match (field, event) {
            (_, InputEvent::Enter | InputEvent::Tab) if suggesting => {
                self.accept_suggestion(None, cx)
            }
            (_, InputEvent::Down) if suggesting => self.move_suggestion(1, cx),
            (_, InputEvent::Up) if suggesting => self.move_suggestion(-1, cx),
            // Backspace in an empty recipient field takes the last pill back as text.
            (Field::To | Field::Cc | Field::Bcc, InputEvent::BackspaceAtStart) => {
                if let Some(composer) = &mut self.compose
                    && let (Some(slot), Some(input)) =
                        (Composer::slot(field), composer.input(field))
                    && input.read(cx).text().is_empty()
                    && let Some(address) = composer.chips[slot].pop()
                {
                    input.update(cx, |input, cx| input.set(address, usize::MAX, cx));
                    composer.sync_placeholder(field, cx);
                    self.composer_changed(cx);
                    cx.notify();
                }
            }
            (
                Field::To | Field::Cc | Field::Bcc,
                InputEvent::Enter | InputEvent::Tab | InputEvent::Down,
            ) => {
                if let Some(composer) = &mut self.compose {
                    composer.chipify(field, true, cx);
                }
                let next = match field {
                    Field::To if show_cc => Field::Cc,
                    Field::Cc => Field::Bcc,
                    _ => Field::Subject,
                };
                self.focus_field(next, None, cx)
            }
            (Field::Cc, InputEvent::Up) => self.focus_field(Field::To, None, cx),
            (Field::Bcc, InputEvent::Up) => self.focus_field(Field::Cc, None, cx),
            (Field::Subject, InputEvent::Enter | InputEvent::Tab | InputEvent::Down) => {
                self.focus_field(Field::Line(0), None, cx)
            }
            (Field::Subject, InputEvent::Up) => {
                self.focus_field(if show_cc { Field::Bcc } else { Field::To }, None, cx)
            }
            (Field::Line(0), InputEvent::Up) => self.focus_field(Field::Subject, None, cx),
            (Field::Line(index), InputEvent::Up) => {
                let cursor = self
                    .compose
                    .as_ref()
                    .map(|c| c.lines[index].read(cx).cursor());
                self.focus_field(Field::Line(index - 1), cursor, cx)
            }
            (Field::Line(index), InputEvent::Down) if index + 1 < line_count => {
                let cursor = self
                    .compose
                    .as_ref()
                    .map(|c| c.lines[index].read(cx).cursor());
                self.focus_field(Field::Line(index + 1), cursor, cx)
            }
            // Enter splits the line at the cursor.
            (Field::Line(index), InputEvent::Enter) => {
                let (new_line, subscription) = self.new_field(Field::Line(index + 1), "", cx);
                let Some(composer) = &mut self.compose else {
                    return;
                };
                let current = composer.lines[index].clone();
                let (text, cursor) = {
                    let input = current.read(cx);
                    (input.text(), input.cursor())
                };
                let tail = text[cursor..].to_string();
                current.update(cx, |input, cx| {
                    input.set(text[..cursor].to_string(), cursor, cx)
                });
                new_line.update(cx, |input, cx| input.set(tail, 0, cx));
                composer.lines.insert(index + 1, new_line);
                composer._subscriptions.push(subscription);
                self.focus_field(Field::Line(index + 1), None, cx);
            }
            // Backspace at the start joins the line onto the one above.
            (Field::Line(index), InputEvent::BackspaceAtStart) if index > 0 => {
                let Some(composer) = &mut self.compose else {
                    return;
                };
                let removed = composer.lines.remove(index);
                let tail = removed.read(cx).text();
                let previous = composer.lines[index - 1].clone();
                let head = previous.read(cx).text();
                let join = head.len();
                previous.update(cx, |input, cx| input.set(format!("{head}{tail}"), join, cx));
                self.focus_field(Field::Line(index - 1), None, cx);
            }
            _ => {}
        }
    }

    /// Sends what was written, after a few seconds in which it can be undone. Only ever
    /// runs from the Send button or its shortcut.
    fn send_mail(&mut self, cx: &mut Context<Self>) {
        let demo = self.gmail.is_none();
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.sending {
            return;
        }
        let outgoing = composer.outgoing(cx);
        if let Err(error) = outgoing.recipients() {
            composer.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        if outgoing.subject.trim().is_empty()
            && outgoing.body.trim().is_empty()
            && outgoing.files.is_empty()
        {
            composer.error = Some("Write a subject or a message first.".into());
            cx.notify();
            return;
        }
        if outgoing.attachment_size() > gmail::ATTACHMENT_LIMIT {
            composer.error = Some("Attachments can be 25 MB in total.".into());
            cx.notify();
            return;
        }
        if demo {
            // Sample data: nothing leaves this computer.
            self.compose = None;
            self.pending_focus = Some(self.focus.clone());
            self.notice = Some("Sample mode: the message was not sent anywhere.".into());
            cx.notify();
            return;
        }
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        composer.error = None;
        let serial = composer.serial;
        self.parked.push((Parked::Sending, composer));
        self.pending_focus = Some(self.focus.clone());
        self.offer_undo("Sending…", cx);
        self.finish_parked_later(serial, UNDO_DELAY, cx);
        cx.notify();
    }

    /// Fetches the remote images of a message after the user asked for them.
    fn load_images(&mut self, sources: Vec<String>, cx: &mut Context<Self>) {
        for url in sources.into_iter().take(16) {
            if !self.image_requests.insert(url.clone()) {
                continue;
            }
            cx.spawn(async move |this, cx| {
                let fetch_url = url.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { rich::fetch_image(&fetch_url) })
                    .await;
                let Ok((kind, bytes)) = result else {
                    return;
                };
                let format = match kind {
                    rich::ImageKind::Png => gpui::ImageFormat::Png,
                    rich::ImageKind::Jpeg => gpui::ImageFormat::Jpeg,
                    rich::ImageKind::Gif => gpui::ImageFormat::Gif,
                    rich::ImageKind::Webp => gpui::ImageFormat::Webp,
                };
                this.update(cx, |this, cx| {
                    this.images
                        .insert(url, Arc::new(gpui::Image::from_bytes(format, bytes)));
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// How many rows the palette has.
    fn palette_len(&self) -> usize {
        if self.is_demo() {
            COMMANDS.len()
        } else {
            self.snooze_choices.len()
        }
    }

    /// Runs the highlighted palette command. Snoozing is only simulated on the sample data;
    /// on a real mailbox it must not archive anything, so it just says so.
    fn run_command(&mut self, cx: &mut Context<Self>) {
        self.palette = false;
        if self.is_demo() {
            self.mark_done(cx);
        } else if let Some(choice) = self.snooze_choices.get(self.palette_sel).cloned() {
            self.snooze(choice, cx);
        }
    }

    /// ⌘⇧L and the sun/moon button: switch to the other theme, and remember it.
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark = !self.dark;
        self.settings.theme = if self.dark {
            crate::settings::ThemeChoice::Dark
        } else {
            crate::settings::ThemeChoice::Light
        };
        self.save_settings(cx);
        cx.notify();
    }

    fn save_settings(&self, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        cx.background_executor()
            .spawn(async move {
                if crate::settings::save(&settings).is_err() {
                    eprintln!("nimbra: could not save settings");
                }
            })
            .detach();
    }

    /// ⌘, or the account row: the Settings sheet.
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.compose.is_some() || self.settings_open {
            return;
        }
        self.menu = None;
        self.palette = false;
        self.sign_out_armed = None;
        let t = self.theme();
        let mut lines = Vec::new();
        let mut subscriptions = Vec::new();
        let text: Vec<String> = self
            .settings
            .signature
            .lines()
            .map(str::to_string)
            .collect();
        for index in 0..6 {
            let placeholder = if index == 0 { "Your name" } else { "" };
            let input = cx.new(|cx| TextInput::new(cx, placeholder, t.faint, t.violet));
            if let Some(line) = text.get(index).cloned() {
                input.update(cx, |input, cx| input.set(line, usize::MAX, cx));
            }
            subscriptions.push(
                cx.subscribe(&input, move |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.signature_changed(cx),
                    InputEvent::Enter | InputEvent::Down | InputEvent::Tab => {
                        if let Some(next) = this.signature_lines.get(index + 1) {
                            this.pending_focus = Some(next.read(cx).focus_handle(cx));
                            cx.notify();
                        }
                    }
                    InputEvent::Up | InputEvent::BackspaceAtStart if index > 0 => {
                        let previous = this.signature_lines[index - 1].clone();
                        this.pending_focus = Some(previous.read(cx).focus_handle(cx));
                        cx.notify();
                    }
                    _ => {}
                }),
            );
            lines.push(input);
        }
        self.signature_lines = lines;
        self._settings_subscriptions = subscriptions;
        self.login_item = crate::system::opens_at_login();
        self.settings_open = true;
        if let Some(view) = &self.html_view {
            view.set_visible(false);
        }
        cx.notify();
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.signature_lines.clear();
        self._settings_subscriptions.clear();
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    fn signature_changed(&mut self, cx: &mut Context<Self>) {
        let lines: Vec<String> = self
            .signature_lines
            .iter()
            .map(|line| line.read(cx).text().trim_end().to_string())
            .collect();
        let signature = lines.join("\n").trim_end().to_string();
        if signature != self.settings.signature {
            self.settings.signature = signature;
            self.save_settings(cx);
        }
    }

    /// Signs out: forgets the Google sign-in and removes the mail kept on this Mac and its
    /// key. Settings (signature, theme) stay.
    fn sign_out(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id.clone();
        let gone = account.clone();
        std::thread::spawn(move || {
            gmail::forget_refresh_token(&gone);
            crate::store::wipe(&gone);
        });
        self.settings.accounts.retain(|a| a.id != account);
        // Another account is signed in: show that one.
        if let Some(next) = self.settings.accounts.first().map(|a| a.id.clone()) {
            self.close_settings(cx);
            self.switch_account(next, cx);
            self.notice =
                Some("Signed out of that account. Its mail was removed from this Mac.".into());
            return;
        }
        self.settings.active = String::new();
        self.account_id = String::new();
        self.save_settings(cx);
        self.gmail = None;
        self.store = None;
        self.cache.clear();
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.back.clear();
        self.forward.clear();
        self.scheduled.clear();
        self.parked.clear();
        self.compose = None;
        self.selected = 0;
        self.palette = false;
        self.account = "".into();
        self.status = Status::SignedOut;
        self.close_settings(cx);
        self.notice = Some("Signed out. The mail kept on this Mac was removed.".into());
        cx.notify();
    }

    /// The Settings sheet.
    fn settings_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::settings::ThemeChoice;
        let section = |title: &str| {
            div()
                .pt(px(18.))
                .pb(px(8.))
                .child(overline(title.to_uppercase(), t.text3))
        };
        let switch = |on: bool| {
            div()
                .flex_none()
                .w(px(34.))
                .h(px(20.))
                .rounded_full()
                .p(px(2.))
                .flex()
                .when(on, |d| d.justify_end().bg(t.violet))
                .when(!on, |d| {
                    d.bg(t.hover).border_1().border_color(t.border_strong)
                })
                .child(div().size(px(16.)).rounded_full().bg(gpui::white()))
        };
        let toggle = |id: &'static str, label: &'static str, detail: &'static str, on: bool| {
            let hover = t.hover;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(9.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(label))
                        .child(meta(detail, t.text3)),
                )
                .child(switch(on))
        };
        let choice = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .flex_1()
                .flex()
                .justify_center()
                .py(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(on, |d| {
                    d.bg(t.raised)
                        .border_1()
                        .border_color(t.border_strong)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!on, |d| d.text_color(t.text2))
                .child(label)
        };
        let theme = self.settings.theme;
        let armed = self
            .sign_out_armed
            .is_some_and(|at| at.elapsed() < Duration::from_secs(4));
        let connected = self.gmail.is_some();
        let card = div()
            .id("settings")
            .w(px(560.))
            .max_h(px(720.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .px(px(24.))
            .pt(px(22.))
            .pb(px(20.))
            .rounded(px(18.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![
                shadow(c(0x8b7cff, 0.18), 0., 120., 10.),
                shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
            ])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(serif("Settings", 30., t.text).flex_1())
                    .child(kbd(t, "esc")),
            )
            .child(section("Account"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(10.))
                    .child(avatar(Tone::Graphite, &self.me_initials, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(if connected {
                                short(&self.account, 40)
                            } else {
                                "Not connected".into()
                            }))
                            .child(meta(
                                if connected {
                                    "Gmail. Mail is kept on this Mac, encrypted."
                                } else {
                                    "Sample mail. Connect Gmail from the main window."
                                },
                                t.text3,
                            )),
                    )
                    .when(connected && self.config.is_some(), |d| {
                        d.child(
                            button(t, ButtonKind::Ghost, "Add account", None)
                                .id("add-account")
                                .on_click(cx.listener(|this, _, _, cx| this.add_account(cx))),
                        )
                    })
                    .when(connected, |d| {
                        d.child(
                            button(
                                t,
                                ButtonKind::Secondary,
                                if armed {
                                    "Click again to sign out"
                                } else {
                                    "Sign out"
                                },
                                None,
                            )
                            .id("sign-out")
                            .when(armed, |d| d.text_color(t.rose))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let armed = this
                                    .sign_out_armed
                                    .is_some_and(|at| at.elapsed() < Duration::from_secs(4));
                                if armed {
                                    this.sign_out(cx);
                                } else {
                                    this.sign_out_armed = Some(std::time::Instant::now());
                                    cx.notify();
                                }
                            })),
                        )
                    }),
            )
            .child(section("Signature"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .mx(px(10.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .bg(t.hover)
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(14.))
                    .line_height(px(22.))
                    // The used lines and one more, at least three.
                    .children({
                        let used = self
                            .signature_lines
                            .iter()
                            .rposition(|line| !line.read(cx).text().is_empty())
                            .map_or(0, |last| last + 1);
                        self.signature_lines.iter().take((used + 1).max(3)).cloned()
                    }),
            )
            .child(div().px(px(10.)).pt(px(6.)).child(meta(
                "Added below new messages, replies and forwards.",
                t.text3,
            )))
            .child(section("Appearance"))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .mx(px(10.))
                    .p(px(3.))
                    .rounded(px(10.))
                    .bg(t.hover)
                    .child(
                        choice("theme-system", "System", theme == ThemeChoice::System).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.settings.theme = ThemeChoice::System;
                                this.save_settings(cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        choice("theme-light", "Light", theme == ThemeChoice::Light).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.settings.theme = ThemeChoice::Light;
                                this.dark = false;
                                this.save_settings(cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        choice("theme-dark", "Dark", theme == ThemeChoice::Dark).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.settings.theme = ThemeChoice::Dark;
                                this.dark = true;
                                this.save_settings(cx);
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(section("Reading"))
            .child(
                toggle(
                    "set-original",
                    "Open mail in its original formatting",
                    "Off shows every message in Nimbra’s reader. V switches either way.",
                    self.settings.original_view,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.original_view = !this.settings.original_view;
                    this.original = this.settings.original_view;
                    this.save_settings(cx);
                    cx.notify();
                })),
            )
            .child(
                toggle(
                    "set-images",
                    "Show remote images",
                    "Off waits until you ask: remote images can tell a sender you opened the mail.",
                    self.settings.load_images,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.load_images = !this.settings.load_images;
                    this.original_images = this.settings.load_images;
                    this.save_settings(cx);
                    cx.notify();
                })),
            )
            .child(section("Mac"))
            .child(
                toggle(
                    "set-login",
                    "Open Nimbra at login",
                    "So new mail is checked and notified from the moment you sign in to your Mac.",
                    self.login_item,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    match crate::system::set_open_at_login(!this.login_item) {
                        Ok(()) => this.login_item = !this.login_item,
                        Err(message) => this.notice = Some(message.into()),
                    }
                    cx.notify();
                })),
            )
            .child(section("Notifications"))
            .child(
                toggle(
                    "set-notify",
                    "Notify me about new mail",
                    "New unread mail in the inbox, while Nimbra is running.",
                    self.settings.notifications,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.notifications = !this.settings.notifications;
                    this.save_settings(cx);
                    cx.notify();
                })),
            )
            .child(
                div().flex().justify_end().pt(px(20.)).child(
                    button(t, ButtonKind::Primary, "Done", None)
                        .id("settings-done")
                        .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))),
                ),
            );
        div()
            .id("settings-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_settings(cx)),
            )
            .child(card)
    }

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        // Offline is a state, not an error: keep showing what is here, and catch up later.
        if error.is::<gmail::Offline>() {
            self.fetching = false;
            if self.mails.is_empty() && !matches!(self.status, Status::Ready) {
                self.status = Status::Failed(error.to_string().into());
            }
            self.went_offline(cx);
            cx.notify();
            return;
        }
        eprintln!("nimbra: {error}");
        if error.is::<gmail::SignedOut>() {
            self.gmail = None;
            // The stored mail belongs to that sign-in: remove it, and its key.
            self.store = None;
            let account = self.account_id.clone();
            std::thread::spawn(move || crate::store::wipe(&account));
            self.cache.clear();
            self.mails.clear();
            self.done.clear();
            self.history.clear();
            self.back.clear();
            self.forward.clear();
            self.selected = 0;
            self.palette = false;
            self.status = Status::SignedOut;
            self.notice = Some("Your Google sign-in expired. Connect again to continue.".into());
        } else if self.mails.is_empty() {
            self.status = Status::Failed(error.to_string().into());
        } else {
            self.notice = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn theme(&self) -> Theme {
        if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    fn visible(&self) -> Vec<usize> {
        (0..self.mails.len())
            .filter(|i| match (self.is_demo(), self.mailbox) {
                (true, View::Done) => self.done[*i],
                // The sample data is searched locally.
                (true, View::Search) => {
                    let mail = &self.mails[*i];
                    let query = self.query.to_lowercase();
                    !self.done[*i]
                        && [&mail.sender, &mail.subject, &mail.snippet]
                            .iter()
                            .any(|field| field.to_lowercase().contains(&query))
                }
                (true, View::Inbox) | (false, _) => {
                    // The open thread stays listed under Unread even once it is read.
                    !self.done[*i]
                        && (!self.unread_only || self.mails[*i].unread || *i == self.selected)
                }
                (true, _) => false,
            })
            .collect()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.palette {
            let max = self.palette_len() as isize - 1;
            self.palette_sel = (self.palette_sel as isize + delta).clamp(0, max) as usize;
        } else {
            self.picked.clear();
            let visible = self.visible();
            if let Some(pos) = visible.iter().position(|i| *i == self.selected) {
                let next = (pos as isize + delta).clamp(0, visible.len() as isize - 1);
                self.open(visible[next as usize]);
                self.open_selected(cx);
            }
        }
        cx.notify();
    }

    /// Opens a thread and records the previous one so Back can return to it.
    fn open(&mut self, i: usize) {
        if i != self.selected {
            self.back.push(self.selected);
            self.forward.clear();
            self.selected = i;
        }
    }

    fn can_go_back(&self) -> bool {
        self.back.iter().any(|i| !self.done[*i])
    }

    fn can_go_forward(&self) -> bool {
        self.forward.iter().any(|i| !self.done[*i])
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        // Skip threads that were marked done in the meantime.
        while let Some(i) = self.back.pop() {
            if !self.done[i] {
                self.forward.push(self.selected);
                self.selected = i;
                break;
            }
        }
        self.open_selected(cx);
        cx.notify();
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        while let Some(i) = self.forward.pop() {
            if !self.done[i] {
                self.back.push(self.selected);
                self.selected = i;
                break;
            }
        }
        self.open_selected(cx);
        cx.notify();
    }

    /// Pull to refresh, in the style of the system's: the list stretches with rubber-band
    /// resistance, letting go past the threshold starts the refresh, and the indicator
    /// springs back when it is done. Only a trackpad gesture that begins with the list
    /// already at the top counts, so the momentum of a normal scroll up never triggers it.
    fn pull_list(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let at_top = f32::from(self.list_scroll.offset().y) >= -0.5;
        match event.touch_phase {
            gpui::TouchPhase::Started => {
                self.pull_armed = at_top && !self.pull_hold;
                return;
            }
            gpui::TouchPhase::Ended => {
                self.pull_armed = false;
                if self.pull <= 0. {
                    return;
                }
                let shown = rubber_band(self.pull);
                self.pull = 0.;
                self.pull_gen += 1;
                if shown >= PULL_TRIGGER && !self.fetching {
                    // Settle at the resting height and stay there while mail is fetched.
                    self.pull_anim = Some((shown, PULL_REST, std::time::Instant::now()));
                    self.pull_hold = true;
                    self.reload(cx);
                } else {
                    self.pull_anim = Some((shown, 0., std::time::Instant::now()));
                }
                cx.notify();
                return;
            }
            gpui::TouchPhase::Moved => {}
        }
        if !self.pull_armed {
            return;
        }
        let dy = f32::from(event.delta.pixel_delta(px(18.)).y);
        if (dy > 0. && at_top) || (dy < 0. && self.pull > 0.) {
            self.pull = (self.pull + dy).max(0.);
            self.pull_anim = None;
            // While the list is being pulled it must not scroll as well, or the two
            // movements fight each other.
            self.list_scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }
    }

    /// The strip at the top of the list while pulling, refreshing or settling.
    fn pull_indicator(&self, t: &Theme) -> Option<AnyElement> {
        let strip = |height: f32, label: &'static str, spin: bool, turn: f32| {
            let arrow = icon("reload", 15., t.violet);
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .h(px(height.max(0.)))
                .overflow_hidden()
                .child(if spin {
                    arrow
                        .with_animation(
                            "pull-spin",
                            Animation::new(Duration::from_millis(800)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(Transformation::rotate(percentage(delta)))
                            },
                        )
                        .into_any_element()
                } else {
                    arrow
                        .with_transformation(Transformation::rotate(percentage(turn)))
                        .into_any_element()
                })
                .child(meta(label, t.text3))
        };
        if self.pull > 0. {
            // Following the finger: no animation, just the stretched height.
            let shown = rubber_band(self.pull);
            let ready = shown >= PULL_TRIGGER;
            return Some(
                strip(
                    shown,
                    if ready {
                        "Release to refresh"
                    } else {
                        "Pull to refresh"
                    },
                    false,
                    (shown / PULL_TRIGGER).min(1.) * 0.75,
                )
                .opacity(0.45 + 0.55 * (shown / PULL_TRIGGER).min(1.))
                .into_any_element(),
            );
        }
        let (from, to, start) = self.pull_anim?;
        let label = if self.pull_hold {
            "Checking for mail…"
        } else {
            ""
        };
        Some(
            strip(settle(from, to, start), label, self.pull_hold, 0.75)
                .with_animation(
                    ("pull-settle", self.pull_gen),
                    // The animation only keeps frames coming; the height follows the spring.
                    Animation::new(Duration::from_secs_f32(SETTLE_SECONDS + 0.1)),
                    move |strip, _| strip.h(px(settle(from, to, start))),
                )
                .into_any_element(),
        )
    }

    /// The message list in a wrapper whose width slides open or shut after a toggle, the
    /// same way the sidebar does. Once it has finished closing it is not drawn at all.
    fn list_slot(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        const SLIDE: Duration = Duration::from_millis(260);
        let moving = self.list_toggled.is_some_and(|at| at.elapsed() < SLIDE);
        if !self.list_open && !moving {
            return None;
        }
        let width = self.list_width;
        let slot = div()
            .flex_none()
            .h_full()
            .overflow_hidden()
            .child(self.list(t, cx));
        if !moving {
            return Some(slot.w(px(width)).into_any_element());
        }
        let (from, to) = if self.list_open { (0., 1.) } else { (1., 0.) };
        Some(
            slot.with_animation(
                ("list-width", self.list_gen),
                Animation::new(SLIDE).with_easing(ease_out_quint()),
                move |slot, delta| slot.w(px(width * (from + (to - from) * delta))),
            )
            .into_any_element(),
        )
    }

    /// Hides or shows the message list, leaving the reading pane the full width.
    fn toggle_list(&mut self, cx: &mut Context<Self>) {
        self.list_open = !self.list_open;
        self.list_grab = None;
        self.list_toggled = Some(std::time::Instant::now());
        self.list_gen += 1;
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.sidebar_gen += 1;
        cx.notify();
    }

    /// Runs `apply` with how open the sidebar is (0 = hidden, 1 = shown), animating
    /// towards the current state after a toggle.
    fn with_sidebar_progress<E: IntoElement + 'static>(
        &self,
        key: &'static str,
        element: E,
        apply: impl Fn(E, f32) -> E + 'static,
    ) -> AnyElement {
        let (from, to) = if self.sidebar_open {
            (0., 1.)
        } else {
            (1., 0.)
        };
        if self.sidebar_gen == 0 {
            return apply(element, to).into_any_element();
        }
        element
            .with_animation(
                (key, self.sidebar_gen),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                move |element, delta| apply(element, from + (to - from) * delta),
            )
            .into_any_element()
    }

    /// Checks for mail. With the sample data, everything that was cleared arrives again.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_gen += 1;
        if !self.is_demo() {
            self.refresh(cx);
            return;
        }
        self.done.fill(false);
        self.history.clear();
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        cx.notify();
    }

    /// `E`: archive from the inbox, or move back to the inbox from Done.
    fn mark_done(&mut self, cx: &mut Context<Self>) {
        let change = match self.mailbox {
            View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social => {
                gmail::Change::Archive
            }
            View::Done => gmail::Change::Unarchive,
            View::Spam => gmail::Change::NotSpam,
            View::Trash => gmail::Change::Untrash,
            // Back to the inbox now, instead of at the snooze time.
            View::Snoozed => {
                let threads: Vec<String> = self
                    .targets()
                    .iter()
                    .filter_map(|i| self.mails[*i].id.clone())
                    .collect();
                self.snoozed.retain(|s| !threads.contains(&s.thread_id));
                self.save_snoozed(cx);
                gmail::Change::Unarchive
            }
            _ => return,
        };
        self.remove_selected(change, cx);
    }

    /// Moves the open thread to Gmail's trash (kept for 30 days, and undoable with `Z`).
    fn trash(&mut self, cx: &mut Context<Self>) {
        self.remove_selected(gmail::Change::Trash, cx);
    }

    /// Takes the open thread out of the list and applies `change` to it in Gmail.
    fn remove_selected(&mut self, change: gmail::Change, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            self.remove_picked(change, cx);
            return;
        }
        self.picked.clear();
        let visible = self.visible();
        let Some(pos) = visible.iter().position(|i| *i == self.selected) else {
            return;
        };
        let index = self.selected;
        self.done[index] = true;
        self.history.push(index);
        self.history_groups.push(1);
        if let Some(id) = self.mails[index].id.clone() {
            self.removed_by.insert(id, change);
        }
        if let Some(next) = visible
            .get(pos + 1)
            .or(pos.checked_sub(1).map(|p| &visible[p]))
        {
            self.selected = *next;
        }
        self.sync(index, change, true, cx);
        self.open_selected(cx);
        self.offer_undo(
            match change {
                gmail::Change::Trash => "Moved to trash",
                gmail::Change::Unarchive | gmail::Change::NotSpam => "Moved to inbox",
                gmail::Change::Spam => "Reported as spam",
                gmail::Change::Untrash => "Restored",
                _ => "Marked done",
            },
            cx,
        );
        cx.notify();
    }

    /// Shows the "Undo" pill at the bottom centre of the window for a few seconds.
    fn offer_undo(&mut self, label: &'static str, cx: &mut Context<Self>) {
        const SHOWN: Duration = Duration::from_secs(6);
        const SIZE: (f32, f32) = (300., 44.);
        self.close_undo(cx);
        let Some(frame) = self.frame else {
            return;
        };
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - px(SIZE.0)) / 2. - px(MENU_MARGIN),
            frame.origin.y + frame.size.height - px(SIZE.1 + 24.) - px(MENU_MARGIN),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(px(SIZE.0 + 2. * MENU_MARGIN), px(SIZE.1 + 2. * MENU_MARGIN)),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                // Keyboard focus stays in the main window, so ⌘Z still reaches it.
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|_| UndoToast { owner, label, dark })
            },
        );
        let Ok(handle) = opened else {
            return;
        };
        self.undo_window = Some(handle);
        self.undo_gen += 1;
        let shown_gen = self.undo_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOWN).await;
            this.update(cx, |this, cx| {
                if this.undo_gen == shown_gen {
                    this.close_undo(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn close_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.undo_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    /// `Z`: brings back the last thread that was archived, moved or trashed.
    fn undo(&mut self, cx: &mut Context<Self>) {
        self.close_undo(cx);
        // A message being sent or a discarded draft comes back first.
        if self.compose.is_none()
            && let Some(serial) = self
                .parked
                .iter()
                .rev()
                .find(|(kind, _)| matches!(kind, Parked::Sending | Parked::Discarded))
                .map(|(_, c)| c.serial)
        {
            if let Some(composer) = self.composer_mut(serial) {
                composer.error = None;
            }
            self.unpark(serial, cx);
            return;
        }
        // A batch (several threads at once) comes back together.
        let group = self
            .history_groups
            .pop()
            .unwrap_or(1)
            .clamp(1, self.history.len().max(1));
        for _ in 1..group {
            if let Some(index) = self.history.pop() {
                self.undo_one(index, cx);
            }
        }
        let Some(index) = self.history.pop() else {
            return;
        };
        self.undo_one(index, cx);
        self.selected = index;
        self.load_selected(cx);
        cx.notify();
    }

    /// Puts one removed thread back.
    fn undo_one(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.mails.len() {
            return;
        }
        self.done[index] = false;
        let change = self.mails[index]
            .id
            .as_ref()
            .and_then(|id| self.removed_by.remove(id))
            .unwrap_or(gmail::Change::Archive);
        // Undoing a snooze cancels it.
        if let Some(thread) = self.mails[index].id.clone()
            && self.snoozed.iter().any(|s| s.thread_id == thread)
        {
            self.snoozed.retain(|s| s.thread_id != thread);
            self.save_snoozed(cx);
        }
        if let Some(inverse) = change.inverse() {
            self.sync(index, inverse, false, cx);
        }
        if let Some(thread) = self.mails[index].id.clone()
            && let Some(label) = self.moved.remove(&thread)
        {
            self.mails[index].labels.retain(|id| *id != label);
            if let Some(client) = self.gmail.clone() {
                cx.background_executor()
                    .spawn(async move {
                        let _ = client.remove_label(&thread, &label);
                    })
                    .detach();
            }
        }
    }

    fn toggle_star(&mut self, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            let targets = self.targets();
            let starred = !targets.iter().all(|i| self.mails[*i].starred);
            for index in targets {
                if self.mails[index].starred != starred {
                    self.mails[index].starred = starred;
                    let change = if starred {
                        gmail::Change::Star
                    } else {
                        gmail::Change::Unstar
                    };
                    self.sync(index, change, false, cx);
                }
            }
            cx.notify();
            return;
        }
        let index = self.selected;
        if !self.visible().contains(&index) {
            return;
        }
        let starred = !self.mails[index].starred;
        self.mails[index].starred = starred;
        let change = if starred {
            gmail::Change::Star
        } else {
            gmail::Change::Unstar
        };
        self.sync(index, change, false, cx);
        cx.notify();
    }

    fn set_unread_only(&mut self, unread_only: bool, cx: &mut Context<Self>) {
        self.unread_only = unread_only;
        if !self.visible().contains(&self.selected) {
            self.selected = self.visible().first().copied().unwrap_or(0);
        }
        self.load_selected(cx);
        cx.notify();
    }
}

// ---------------------------------------------------------------- helpers

fn serif(text: &str, size: f32, color: Hsla) -> Div {
    // The tail of the title (after the last comma, or the last word) is set in italic.
    let start = text
        .rfind(", ")
        .map(|i| i + 2)
        .or(text.rfind(' ').map(|i| i + 1))
        .unwrap_or(text.len());
    let italic = HighlightStyle {
        font_style: Some(FontStyle::Italic),
        ..Default::default()
    };
    div()
        .font_family(SERIF)
        .text_size(px(size))
        .line_height(px((size * 1.14).round()))
        .text_color(color)
        .child(StyledText::new(text.to_string()).with_highlights([(start..text.len(), italic)]))
}

/// Shortens text to `max` characters with an ellipsis.
///
/// Used instead of GPUI's own ellipsis truncation, which in 0.2.2 can panic when the cut
/// lands inside a multi-byte character such as an emoji. The element still clips whatever
/// does not fit.
fn short(text: &str, max: usize) -> SharedString {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end()).into()
    } else {
        head.into()
    }
}

/// A stable number per mailbox, for element ids.
fn view_key(view: View) -> usize {
    match view {
        View::Inbox => 0,
        View::Primary => 1,
        View::Updates => 2,
        View::Promotions => 3,
        View::Social => 4,
        View::Starred => 5,
        View::Snoozed => 6,
        View::Drafts => 7,
        View::Sent => 8,
        View::Done => 9,
        View::Search => 10,
        View::AllMail => 11,
        View::Important => 12,
        View::Spam => 13,
        View::Trash => 14,
        View::Label(index) => 100 + index,
    }
}

fn section(t: &Theme, label: &str) -> Div {
    div()
        .pl(px(10.))
        .pt(px(18.))
        .pb(px(8.))
        .child(overline(label.to_string(), t.text3))
}

fn nav(
    t: &Theme,
    name: &str,
    label: &str,
    count: Option<String>,
    selected: bool,
    tint: Option<Hsla>,
) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(t.selected).font_weight(FontWeight::MEDIUM)
        })
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .child(icon(
            name,
            16.,
            tint.unwrap_or(if selected { t.violet } else { t.text2 }),
        ))
        .child(
            div()
                .flex_1()
                .text_color(if selected { t.text } else { t.text2 })
                .child(label.to_string()),
        )
        .children(count.map(|n| mono(n, if selected { t.violet } else { t.text3 })))
}

fn card(t: &Theme) -> Div {
    div()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .rounded(px(12.))
}

// ---------------------------------------------------------------- panes

impl Nimbra {
    /// Sidebar toggle, back, forward and reload, sitting beside the traffic lights.
    /// Drawn above the panes so it stays put when the sidebar is hidden.
    fn window_controls(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = t.hover;
        let control = |name: &str, enabled: bool| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .child(icon(name, 17., if enabled { t.text2 } else { t.faint }))
        };
        div()
            .absolute()
            .top(px(11.))
            .left(px(84.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                control("sidebar", true)
                    .id("toggle-sidebar")
                    .tooltip(tip(t.dark, "Show or hide sidebar", "⌘B"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                control("back", self.can_go_back())
                    .id("go-back")
                    .tooltip(tip(t.dark, "Back", "⌘["))
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
            )
            .child(
                control("next", self.can_go_forward())
                    .id("go-forward")
                    .tooltip(tip(t.dark, "Forward", "⌘]"))
                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                div()
                    .id("reload")
                    .tooltip(tip(t.dark, "Check for mail", "⌘R"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                    .child(if self.fetching {
                        // Keeps turning for as long as a mailbox request is in flight.
                        icon("reload", 17., t.violet)
                            .with_animation(
                                "reload-loop",
                                Animation::new(Duration::from_millis(900)).repeat(),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    } else if self.reload_gen == 0 {
                        icon("reload", 17., t.text2).into_any_element()
                    } else {
                        // One full turn each time mail is checked.
                        icon("reload", 17., t.text2)
                            .with_animation(
                                ("reload-spin", self.reload_gen),
                                Animation::new(Duration::from_millis(700))
                                    .with_easing(ease_out_quint()),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    }),
            )
            // Offline: say so, and how much is waiting to go out.
            .when(self.offline, |d| {
                let waiting = self.outbox.changes.len() + self.outbox.mail.len();
                d.child(
                    div()
                        .id("offline")
                        .ml(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(2.))
                        .rounded_full()
                        .bg(t.amber.opacity(0.14))
                        .border_1()
                        .border_color(t.amber.opacity(0.4))
                        .tooltip(tip(
                            t.dark,
                            "Changes are kept and sent when the connection is back",
                            "",
                        ))
                        .child(div().size(px(6.)).rounded_full().bg(t.amber))
                        .child(meta(
                            if waiting == 0 {
                                "Offline".to_string()
                            } else {
                                format!("Offline · {waiting} waiting")
                            },
                            t.text2,
                        )),
                )
            })
    }

    fn sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let left = self.visible().len();
        let unread = self
            .visible()
            .iter()
            .filter(|i| self.mails[**i].unread)
            .count();
        let bars = [9., 14., 8., 17., 12., 20., 15.];
        // The other mailboxes are not connected yet, so their counts only show in the demo.
        let demo = self.is_demo();
        let fake = |n: &str| demo.then(|| n.to_string());
        // Real counts from Gmail: unread in the inbox for mailboxes and labels (archived
        // "Done" threads left out), total for Drafts.
        let unread_in = |id: &str| {
            self.counts
                .get(id)
                .map(|(unread, _)| *unread)
                .filter(|n| *n > 0)
                .map(|n| n.to_string())
        };
        let drafts = self
            .counts
            .get("DRAFT")
            .map(|(_, total)| *total)
            .filter(|n| *n > 0)
            .map(|n| n.to_string());
        let mailbox = |view: View, icon_name: &'static str, count: Option<String>| {
            nav(
                t,
                icon_name,
                &self.title(view),
                count,
                self.mailbox == view,
                None,
            )
            .id(("mailbox", view_key(view)))
            .on_click(cx.listener(move |this, _, _, cx| this.select_mailbox(view, cx)))
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(236.))
            .h_full()
            .px(px(12.))
            .pb(px(14.))
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.border)
            // Space for the native traffic lights; dragging here moves the window.
            .child(
                div()
                    .h(px(52.))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, drag_or_zoom),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .py(px(6.))
                    .rounded(px(9.))
                    .bg(t.hover)
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .id("jump")
                    .cursor(gpui::CursorStyle::IBeam)
                    .on_click(cx.listener(|this, _, _, cx| this.focus_search(cx)))
                    .when(self.mailbox == View::Search, |d| d.border_color(t.violet))
                    .child(icon("search", 16., t.text3))
                    .child(div().flex_1().min_w(px(0.)).child(self.search.clone()))
                    .child(kbd(t, "/")),
            )
            .child(section(t, "SPLIT INBOX"))
            .child(mailbox(
                View::Inbox,
                "inbox",
                if demo {
                    (unread > 0 && self.mailbox == View::Inbox).then(|| unread.to_string())
                } else {
                    unread_in("INBOX")
                },
            ))
            .when(demo, |d| {
                d.child(nav(t, "users", "Team", fake("4"), false, None))
                    .child(nav(t, "calendar", "Calendar", fake("2"), false, None))
                    .child(nav(t, "news", "Newsletters", fake("31"), false, None))
                    .child(nav(t, "zap", "Other", fake("23"), false, None))
            })
            // With real mail the split inbox is Gmail's own categories.
            .when(!demo, |d| {
                d.child(mailbox(
                    View::Primary,
                    "users",
                    unread_in("CATEGORY_PERSONAL"),
                ))
                .child(mailbox(
                    View::Updates,
                    "bell",
                    unread_in("CATEGORY_UPDATES"),
                ))
                .child(mailbox(
                    View::Promotions,
                    "tag",
                    unread_in("CATEGORY_PROMOTIONS"),
                ))
                .child(mailbox(
                    View::Social,
                    "news",
                    unread_in("CATEGORY_SOCIAL"),
                ))
            })
            .child(section(t, "MAILBOX"))
            .child(mailbox(View::Starred, "star", None))
            .when(!demo, |d| d.child(mailbox(View::Important, "pulse", None)))
            .child(mailbox(View::Snoozed, "clock", fake("3")))
            .child(nav(t, "bell", "Reminders", fake("2"), false, None))
            .child(mailbox(
                View::Drafts,
                "draft",
                if demo { fake("1") } else { drafts },
            ))
            .child(mailbox(View::Sent, "send", None))
            .child(mailbox(
                View::Done,
                "done",
                (!self.history.is_empty()).then(|| self.history.len().to_string()),
            ))
            .when(!demo, |d| {
                d.child(mailbox(View::AllMail, "archive", None))
                    .child(mailbox(View::Spam, "ban", None))
                    .child(mailbox(View::Trash, "trash", None))
            })
            .child(section(t, "LABELS"))
            .when(demo, |d| {
                d.child(nav(t, "tag", "Product", None, false, Some(t.violet)))
                    .child(nav(t, "tag", "Design", None, false, Some(t.rose)))
                    .child(nav(t, "tag", "Finance", None, false, Some(t.amber)))
                    .child(nav(t, "tag", "Travel", None, false, Some(t.sky)))
            })
            .when(!demo, |d| {
                let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                d.children(
                    self.labels
                        .iter()
                        .enumerate()
                        .take(8)
                        .map(|(index, label)| {
                            let view = View::Label(index);
                            nav(
                                t,
                                "tag",
                                &short(&label.name, 22),
                                unread_in(&label.id),
                                self.mailbox == view,
                                Some(tints[index % tints.len()]),
                            )
                            .id(("mailbox", view_key(view)))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_mailbox(view, cx)),
                            )
                        }),
                )
            })
            .child(div().flex_1())
            // The streak card is sample content, so it only shows with the demo data.
            .when(demo, |d| {
                d.child(
                    card(t)
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .p(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(icon("zap", 16., t.amber))
                                .child(
                                    div()
                                        .flex_1()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Inbox zero streak"),
                                )
                                .child(mono("7d", t.text3)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_end()
                                .gap(px(5.))
                                .h(px(22.))
                                .children(bars.map(|h: f32| {
                                    div().flex_1().h(px(h)).rounded(px(2.)).bg(gradient(
                                        180.,
                                        c(0xff7ab6, 1.),
                                        c(0x8b7cff, 1.),
                                    ))
                                })),
                        )
                        .child(meta(
                            if left == 0 {
                                "Done at 10:52 · 28 min faster".to_string()
                            } else {
                                format!("{left} left · usually done by 11:20")
                            },
                            t.text3,
                        )),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mt(px(10.))
                    .p(px(6.))
                    .rounded(px(10.))
                    .child(
                        div()
                            .id("account")
                            .flex()
                            .flex_1()
                            .min_w(px(0.))
                            .items_center()
                            .gap(px(10.))
                            .cursor_pointer()
                            .tooltip(tip(t.dark, "Accounts and settings", ""))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.settings.accounts.is_empty() {
                                    this.open_settings(cx)
                                } else {
                                    this.toggle_menu(Menu::Accounts, window, cx)
                                }
                            }))
                            .child(avatar(Tone::Graphite, &self.me_initials, 28.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .child(short(&self.account, 22)),
                                    )
                                    .child(meta(
                                        match (demo, self.settings.accounts.len()) {
                                            (true, _) => "Work · 3 accounts".to_string(),
                                            (false, 0 | 1) => "Gmail".to_string(),
                                            (false, n) => format!("Gmail · {n} accounts"),
                                        },
                                        t.text3,
                                    )),
                            ),
                    )
                    .child(
                        icon_button(t, if self.dark { "sun" } else { "moon" })
                            .id("theme")
                            .tooltip(tip(t.dark, "Light or dark theme", "⌘⇧L"))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
                    ),
            )
    }

    fn row(&self, t: &Theme, i: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let m = &self.mails[i];
        let picked = self.picked.contains(&i);
        let selected = i == self.selected && self.picked.len() < 2;
        let hover = t.hover;
        // How many characters fit on a row at the list's current width (the text is cut
        // with an ellipsis by character count, so this follows the resize handle).
        let line = ((self.list_width - 116.) / 7.4).max(8.) as usize;
        // On a narrow list the tag gives way so the sender's name stays readable.
        let tag = m
            .tag
            .clone()
            .or_else(|| {
                let accents = [
                    Accent::Violet,
                    Accent::Rose,
                    Accent::Amber,
                    Accent::Sky,
                    Accent::Mint,
                ];
                self.thread_labels(m)
                    .into_iter()
                    .next()
                    .map(|(index, label)| (short(&label.name, 18), accents[index % accents.len()]))
            })
            .filter(|_| line >= 36);
        let tag_chars = tag
            .as_ref()
            .map_or(0, |(label, _)| label.chars().count() + 6);
        let extras =
            13 + tag_chars + if m.thread.is_some() { 3 } else { 0 } + if m.starred { 3 } else { 0 };
        let sender_max = line.saturating_sub(extras).max(6);
        let subject = short(m.subject.split(',').next().unwrap_or(&m.subject), line);
        let snippet_max = line.saturating_sub(subject.chars().count() + 2);
        let snippet = if snippet_max >= 8 {
            short(&m.snippet, snippet_max)
        } else {
            SharedString::default()
        };

        div()
            .id(("row", i))
            .flex()
            .gap(px(12.))
            .pl(px(10.))
            .pr(px(14.))
            .py(px(12.))
            .rounded(px(12.))
            .cursor_pointer()
            .when(selected || picked, |d| d.bg(t.selected))
            .when(picked, |d| d.border_1().border_color(t.violet.opacity(0.5)))
            .when(!selected && !picked, |d| d.hover(move |s| s.bg(hover)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.open_row_menu(i, event.position, window, cx)
                }),
            )
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                this.click_row(i, event.modifiers(), cx)
            }))
            .child(
                div().w(px(6.)).pt(px(11.)).flex_none().child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .when(m.unread, |d| d.bg(t.violet)),
                ),
            )
            .child(avatar(m.tone, &m.initials, 28.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(22.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(short(&m.sender, sender_max)),
                            )
                            .children(m.thread.clone().map(|n| mono(n, t.text3)))
                            .when(m.starred, |d| d.child(icon("star", 13., t.amber)))
                            .children(tag.map(|(label, a)| pill(t, &label, t.accent(a))))
                            .child(div().flex_1())
                            .when(!selected, |d| d.child(mono(m.time.clone(), t.text3)))
                            .when(selected, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .gap(px(8.))
                                        .child(icon("done", 15., t.text2))
                                        .child(icon("clock", 15., t.text2))
                                        .child(icon("bell", 15., t.text2)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                // Long subjects shrink with an ellipsis instead of
                                // spilling out of the row; the snippet takes what is left.
                                div()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if selected { t.text } else { t.text2 })
                                    .child(subject),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(t.text3)
                                    .child(snippet),
                            ),
                    ),
            )
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible();
        let unread = visible.iter().filter(|i| self.mails[**i].unread).count();
        let group = |label: &str| {
            div()
                .pl(px(14.))
                .pt(px(14.))
                .pb(px(6.))
                .child(overline(label.to_string(), t.faint))
        };
        let today: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| !self.mails[*i].yesterday)
            .collect();
        let earlier: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| self.mails[*i].yesterday)
            .collect();

        let segment = |label: &str, count: Option<(&str, Hsla)>, active: bool| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(12.))
                .py(px(4.))
                .rounded(px(7.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if active { t.text } else { t.text3 })
                .when(active, |d| {
                    d.bg(t.raised)
                        .border_1()
                        .border_color(t.border)
                        .shadow(vec![shadow(
                            c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                            1.,
                            3.,
                            0.,
                        )])
                })
                .child(label.to_string())
                .children(count.map(|(n, color)| mono(n.to_string(), color)))
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .w(px(self.list_width))
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
            // Drag the right edge to resize the list; double-click it to hide the list.
            .child(
                div()
                    .id("list-resize")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-3.))
                    .w(px(7.))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .hover(|s| s.bg(gpui::white().opacity(0.06)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            if event.click_count == 2 {
                                this.toggle_list(cx);
                            } else {
                                this.list_grab =
                                    Some((f32::from(event.position.x), this.list_width));
                            }
                        }),
                    ),
            )
            // Makes room for the window controls while the sidebar is hidden.
            .child(
                self.with_sidebar_progress("list-top", div().flex_none(), |d, open| {
                    d.h(px(34. * (1. - open)))
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .pl(px(24.))
                    .pr(px(18.))
                    .pt(px(26.))
                    .pb(px(10.))
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    // Small mailbox name, then filters and Compose on one row.
                    .child(overline(self.inbox_name().to_uppercase(), t.text3))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .mt(px(8.))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(2.))
                                    .p(px(3.))
                                    .rounded(px(10.))
                                    .bg(t.hover)
                                    .border_1()
                                    .border_color(t.border)
                                    .child(
                                        segment("All", None, !self.unread_only)
                                            .id("filter-all")
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_unread_only(false, cx)
                                            })),
                                    )
                                    .child(
                                        segment(
                                            "Unread",
                                            Some((&unread.to_string(), t.faint)),
                                            self.unread_only,
                                        )
                                        .id("filter-unread")
                                        .cursor_pointer()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.set_unread_only(true, cx)
                                            }),
                                        ),
                                    )
                                    .when(self.is_demo(), |d| {
                                        d.child(segment("Needs reply", Some(("2", t.rose)), false))
                                    }),
                            )
                            .child(div().flex_1())
                            .child(
                                button(t, ButtonKind::Secondary, "Compose", Some("C"))
                                    .id("compose")
                                    .tooltip(tip(t.dark, "New message", "C"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_compose(ComposeKind::New, cx)
                                    })),
                            ),
                    ),
            )
            // While mail is checked, a thin bar glides along the top of the list. Its 2px
            // strip is always there, so nothing below it moves.
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(2.))
                    .mx(px(18.))
                    .rounded_full()
                    .overflow_hidden()
                    .when(self.bar_shown || self.bar_fade.is_some(), |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .bg(t.border)
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .h_full()
                                        .w(relative(0.45))
                                        .rounded_full()
                                        .bg(gradient(90., c(0x8b7cff, 0.), c(0xff7ab6, 1.)))
                                        .with_animation(
                                            "checking-mail",
                                            Animation::new(Duration::from_millis(1150))
                                                .repeat()
                                                .with_easing(gpui::ease_in_out),
                                            |bar, delta| bar.left(relative(-0.45 + 1.45 * delta)),
                                        ),
                                )
                                .map(|strip| match self.bar_fade {
                                    // Done: fade the whole strip out.
                                    Some((_, n)) => strip
                                        .with_animation(
                                            ("checking-mail-fade", n),
                                            Animation::new(Duration::from_millis(450))
                                                .with_easing(ease_out_quint()),
                                            |strip, delta| strip.opacity(1. - delta),
                                        )
                                        .into_any_element(),
                                    None => strip.into_any_element(),
                                }),
                        )
                    }),
            )
            .child(
                div()
                    .id("rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.))
                    .px(px(10.))
                    .pb(px(10.))
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.pull_list(event, cx)))
                    .children(self.pull_indicator(t))
                    .when(!today.is_empty(), |d| d.child(group("TODAY")))
                    .children(today.into_iter().map(|i| self.row(t, i, cx)))
                    .when(!earlier.is_empty(), |d| d.child(group("EARLIER")))
                    .children(earlier.into_iter().map(|i| self.row(t, i, cx)))
                    .when(self.next_page.is_some(), |d| {
                        d.child(
                            div().flex().justify_center().pt(px(14.)).child(
                                button(
                                    t,
                                    ButtonKind::Secondary,
                                    if self.loading_more {
                                        "Loading…"
                                    } else {
                                        "Load more"
                                    },
                                    None,
                                )
                                .id("load-more")
                                .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                            ),
                        )
                    })
                    .when(self.next_page.is_none(), |d| {
                        d.child(div().flex().justify_center().pt(px(18.)).child(meta(
                            match (self.is_demo(), self.mailbox) {
                                (true, View::Inbox) => {
                                    "That’s everything important. 23 more in Other."
                                }
                                (false, View::Inbox) => "That’s everything in your inbox.",
                                _ => "That’s everything here.",
                            },
                            t.faint,
                        )))
                    }),
            )
    }

    /// One message drawn natively from its blocks. `base` keeps element ids unique when
    /// several messages are on screen.
    fn message_body(
        &self,
        t: &Theme,
        blocks: &[Block],
        base: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let sources = rich::image_sources(blocks);
        let pending = sources.iter().any(|src| !self.image_requests.contains(src));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_size(px(15.))
            .line_height(px(26.))
            .text_color(t.text2)
            .when(pending, |d| {
                d.child(
                    card(t)
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .pl(px(12.))
                        .pr(px(6.))
                        .py(px(6.))
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .child(icon("image", 16., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .text_color(t.text3)
                                .child("Remote images are hidden to protect your privacy."),
                        )
                        .child(
                            button(t, ButtonKind::Secondary, "Load images", None)
                                .id(("load-images", base))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.load_images(sources.clone(), cx)
                                })),
                        ),
                )
            })
            .children(
                blocks
                    .iter()
                    .enumerate()
                    .map(|(i, block)| self.block(t, block, base + i)),
            )
    }

    fn block(&self, t: &Theme, block: &Block, id: usize) -> AnyElement {
        let text = |color: Hsla| {
            let highlights: Vec<_> = block
                .spans
                .iter()
                .map(|span| {
                    let style = HighlightStyle {
                        font_weight: span.bold.then_some(FontWeight::SEMIBOLD),
                        font_style: span.italic.then_some(FontStyle::Italic),
                        color: span
                            .link
                            .is_some()
                            .then_some(t.violet)
                            .or(span.bold.then_some(color)),
                        background_color: span.code.then_some(t.hover),
                        underline: span.link.is_some().then(|| gpui::UnderlineStyle {
                            thickness: px(1.),
                            color: Some(t.violet.opacity(0.5)),
                            wavy: false,
                        }),
                        ..Default::default()
                    };
                    (span.range.clone(), style)
                })
                .collect();
            let styled = StyledText::new(block.text.clone()).with_highlights(highlights);
            let links: Vec<(std::ops::Range<usize>, String)> = block
                .spans
                .iter()
                .filter_map(|span| Some((span.range.clone(), span.link.clone()?)))
                .collect();
            if links.is_empty() {
                return styled.into_any_element();
            }
            // Links open in the system browser; only http, https and mailto get this far.
            let (ranges, urls): (Vec<_>, Vec<_>) = links.into_iter().unzip();
            gpui::InteractiveText::new(("link-text", id), styled)
                .on_click(ranges, move |index, _, cx| cx.open_url(&urls[index]))
                .into_any_element()
        };

        let element = match &block.kind {
            Kind::Paragraph => div().child(text(t.text)),
            Kind::Heading(level) => {
                let size = match level {
                    1 => 24.,
                    2 => 20.,
                    _ => 17.,
                };
                div()
                    .pt(px(6.))
                    .text_size(px(size))
                    .line_height(px(size * 1.35))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(text(t.text))
            }
            Kind::Item { marker, depth } => div()
                .flex()
                .gap(px(10.))
                .pl(px(4. + 20. * *depth as f32))
                .child(
                    div()
                        .flex_none()
                        .min_w(px(14.))
                        .text_color(t.text3)
                        .child(marker.clone()),
                )
                .child(div().flex_1().min_w(px(0.)).child(text(t.text))),
            Kind::Code => div()
                .px(px(14.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .font_family(crate::theme::MONO)
                .text_size(px(12.5))
                .line_height(px(20.))
                .child(text(t.text)),
            Kind::Rule => div().h(px(1.)).my(px(6.)).bg(t.border),
            Kind::Image { src, alt } => match self.images.get(src) {
                Some(image) => {
                    div().child(gpui::img(image.clone()).max_w(px(560.)).rounded(px(10.)))
                }
                None => div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(t.text3)
                    .child(icon("image", 15., t.text3))
                    .child(if alt.is_empty() {
                        "Image".to_string()
                    } else {
                        format!("Image: {alt}")
                    }),
            },
        };
        // Quoted text gets a bar on the left, one step per nesting level.
        if block.quote > 0 {
            div()
                .ml(px(14. * (block.quote.min(4) - 1) as f32))
                .pl(px(14.))
                .border_l_2()
                .border_color(t.border_strong)
                .text_color(t.text3)
                .child(element)
                .into_any_element()
        } else {
            element.into_any_element()
        }
    }

    fn brief(&self, t: &Theme, m: &Mail) -> impl IntoElement {
        let todo = |text: &str, due: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(16.))
                        .flex_none()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(t.border_strong),
                )
                .child(div().flex_1().child(text.to_string()))
                .child(mono(due.to_string(), t.amber))
        };
        // GPUI has no gradient strokes, so the border is a 1px gradient frame around the card.
        div()
            .p(px(1.))
            .rounded(px(14.))
            .bg(gradient(100., c(0x8b7cff, 0.75), c(0xffb86b, 0.3)))
            .shadow(vec![shadow(c(0x8b7cff, 0.16), 12., 40., -12.)])
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .px(px(20.))
                    .pt(px(16.))
                    .pb(px(18.))
                    .rounded(px(13.))
                    .bg(t.raised)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("sparkles", 16., t.violet))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Brief"))
                            .child(
                                meta(
                                    match &m.thread {
                                        Some(n) => format!("from {n} messages"),
                                        None => "from 1 message".to_string(),
                                    },
                                    t.text3,
                                )
                                .flex_1(),
                            )
                            .child(kbd(t, "⌘"))
                            .child(kbd(t, "J")),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(26.))
                            .text_color(t.text2)
                            .child(m.brief.clone()),
                    )
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(todo("Confirm pricing tiers on slide 14", "Fri"))
                                .child(todo("Sign off on the press embargo date", "Fri")),
                        )
                    }),
            )
    }

    fn reading(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        let visible = self.visible();
        let pos = visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0)
            + 1;
        // Only switch panes when this thread really has HTML to show.
        // Drafts always use the reader, so a click on the text can open the editor.
        let is_draft = self.mailbox == View::Drafts && !self.is_demo();
        let original = self.original && m.html.is_some() && !is_draft;
        // Until the message has been fetched we cannot know whether it has an original
        // (HTML) version, so show a loader, not the reader view that would flash by first.
        let loading = self.original && !is_draft && !m.loaded;
        let divider = || div().h(px(1.)).bg(t.border);

        let earlier = |tone: Tone, initials: &str, name: &str, snippet: &str, when: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(12.))
                .border_t_1()
                .border_color(t.border)
                .child(avatar(tone, initials, 24.))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text2)
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(t.text3)
                        .child(short(snippet, 110)),
                )
                .child(mono(when.to_string(), t.faint))
        };

        let attachment = |name: &str, info: &str, from: u32, to: u32| {
            card(t)
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(8.))
                .pr(px(14.))
                .py(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.))
                        .rounded(px(8.))
                        .bg(gradient(135., c(from, 1.), c(to, 1.)))
                        .child(icon("file", 16., gpui::white())),
                )
                .child(
                    div()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.to_string()),
                        )
                        .child(meta(info.to_string(), t.text3)),
                )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(t.surface)
            .relative()
            // Toolbar
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(if !self.list_open && !self.sidebar_open {
                        236.
                    } else {
                        20.
                    }))
                    .pr(px(20.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(t.border)
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        icon_button(t, "list")
                            .id("toggle-list")
                            .tooltip(tip(t.dark, "Show or hide message list", "⌘⇧B"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Show or hide message list", "⌘⇧B", cx)
                            }))
                            .when(!self.list_open, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_list(cx))),
                    )
                    .child(div().w(px(6.)))
                    .when(is_draft, |d| {
                        d.child(
                            button(t, ButtonKind::Primary, "Edit draft", None)
                                .id("edit-draft")
                                .tooltip(tip(t.dark, "Open this draft in the editor", ""))
                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                    this.set_hint(*hovered, "Open this draft in the editor", "", cx)
                                }))
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.open_draft(this.selected, cx)
                                    }),
                                ),
                        )
                        .child(div().w(px(8.)))
                    })
                    .child(div().w(px(6.)))
                    .child(
                        icon_button(t, "done")
                            .id("mark-done")
                            .tooltip(tip(t.dark, "Mark done (archive)", "E"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Mark done (archive)", "E", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                    )
                    .child(kbd(t, "E"))
                    .child(div().w(px(6.)))
                    .child(
                        icon_button(t, "clock")
                            .id("snooze")
                            .tooltip(tip(t.dark, "Snooze", "H"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Snooze", "H", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.palette = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        icon_button(t, "star")
                            .id("star")
                            .tooltip(tip(t.dark, "Star", "S"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Star", "S", cx)
                            }))
                            .when(m.starred, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(
                        icon_button(t, "mail")
                            .id("toggle-unread")
                            .tooltip(tip(t.dark, "Mark as unread or read", "U"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Mark as unread or read", "U", cx)
                            }))
                            // Lit while the open thread is unread.
                            .when(m.unread, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
                    )
                    .child(
                        icon_button(t, "tag")
                            .id("label")
                            .tooltip(tip(t.dark, "Add a label", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Add a label", "", cx)
                            }))
                            .when(self.menu == Some(Menu::Labels), |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::Labels, window, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "folder")
                            .id("move-to")
                            .tooltip(tip(t.dark, "Move to", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Move to", "", cx)
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::MoveTo, window, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "trash")
                            .id("trash")
                            .tooltip(tip(t.dark, "Move to trash", "⌘⌫"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Move to trash", "⌘⌫", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(t.border_strong))
                    .child(
                        icon_button(t, "reply")
                            .id("reply")
                            .tooltip(tip(t.dark, "Reply", "R"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Reply", "R", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::Reply, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "reply-all")
                            .id("reply-all")
                            .tooltip(tip(t.dark, "Reply all", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Reply all", "", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::ReplyAll, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "forward")
                            .id("forward")
                            .tooltip(tip(t.dark, "Forward", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Forward", "", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::Forward, cx)
                            })),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(t.border_strong))
                    .child(
                        button(
                            t,
                            ButtonKind::Ghost,
                            if original {
                                "Reader view"
                            } else {
                                "Show original"
                            },
                            Some("V"),
                        )
                        .id("original")
                        .tooltip(tip(t.dark, "Switch between original and reader view", "V"))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            this.set_hint(
                                *hovered,
                                "Switch between original and reader view",
                                "V",
                                cx,
                            )
                        }))
                        .when(original, |d| d.bg(t.selected))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_original(cx))),
                    )
                    .child(div().flex_1())
                    .child(mono(format!("{pos} / {}", visible.len()), t.text3).mr(px(6.)))
                    .child(
                        icon_button(t, "up")
                            .id("prev")
                            .tooltip(tip(t.dark, "Previous thread", "K"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Previous thread", "K", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                    )
                    .child(
                        icon_button(t, "down")
                            .id("next")
                            .tooltip(tip(t.dark, "Next thread", "J"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Next thread", "J", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                    )
                    .child(
                        icon_button(t, "more")
                            .id("more")
                            .tooltip(tip(t.dark, "More actions", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "More actions", "", cx)
                            }))
                            .when(self.menu == Some(Menu::More), |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::More, window, cx)
                            })),
                    ),
            )
            // Conversation (native reader)
            .when(loading, |d| d.child(self.loading_pane(t)))
            .when(!original && !loading, |d| {
                d.child(
                    div()
                        .id("conversation")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        // Clicking the text of a thread counts as opening it: it is marked read.
                        .when(!is_draft, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| this.open_selected(cx)))
                        })
                        // Clicking a draft's text opens it in the editor.
                        .when(is_draft, |d| {
                            d.cursor_pointer().on_click(
                                cx.listener(|this, _, _, cx| this.open_draft(this.selected, cx)),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(22.))
                                .px(px(64.))
                                .pt(px(40.))
                                .pb(px(24.))
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(6.))
                                        .when(m.rich, |d| d.child(pill(t, "Product", t.violet)))
                                        .children(
                                            m.tag
                                                .clone()
                                                .map(|(label, a)| pill(t, &label, t.accent(a))),
                                        )
                                        .when(m.rich, |d| d.child(pill(t, "Due Fri", t.amber)))
                                        .when(!m.rich && m.tag.is_none(), |d| {
                                            d.child(pill(
                                                t,
                                                &self.inbox_name(),
                                                t.accent(Accent::Violet),
                                            ))
                                        }),
                                )
                                .child(serif(&m.subject, 38., t.text))
                                .child(self.thread_banners(t, m, cx))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(10.))
                                        .child(
                                            div()
                                                .relative()
                                                .flex_none()
                                                .h(px(28.))
                                                .w(px(if m.rich { 68. } else { 48. }))
                                                .children(
                                                    [
                                                        Some((m.tone, m.initials.clone())),
                                                        m.rich.then(|| {
                                                            (Tone::Mint, SharedString::from("JR"))
                                                        }),
                                                        Some((
                                                            Tone::Graphite,
                                                            self.me_initials.clone(),
                                                        )),
                                                    ]
                                                    .into_iter()
                                                    .flatten()
                                                    .enumerate()
                                                    .map(|(n, (tone, initials))| {
                                                        avatar(tone, &initials, 28.)
                                                            .absolute()
                                                            .top_0()
                                                            .left(px(n as f32 * 20.))
                                                    }),
                                                ),
                                        )
                                        .child(meta(m.participants.clone(), t.text3).flex_1())
                                        .child(mono(m.time.clone(), t.text3)),
                                )
                                .when(!m.brief.is_empty(), |d| d.child(self.brief(t, m)))
                                .when(!m.earlier.is_empty(), |d| {
                                    d.child(div().children(m.earlier.iter().enumerate().map(
                                        |(k, e)| {
                                            // Click a collapsed message to read it in full.
                                            div()
                                                .child(
                                                    earlier(
                                                        e.tone,
                                                        &e.initials,
                                                        &e.sender,
                                                        if e.expanded { "" } else { &e.snippet },
                                                        &e.when,
                                                    )
                                                    .id(("earlier", k))
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        let selected = this.selected;
                                                        if let Some(e) = this
                                                            .mails
                                                            .get_mut(selected)
                                                            .and_then(|m| m.earlier.get_mut(k))
                                                        {
                                                            e.expanded = !e.expanded;
                                                            cx.notify();
                                                        }
                                                    })),
                                                )
                                                .when(e.expanded, |d| {
                                                    d.child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(px(12.))
                                                            .pb(px(14.))
                                                            .child(self.message_header(
                                                                t,
                                                                m,
                                                                k + 1,
                                                                true,
                                                                cx,
                                                            ))
                                                            .child(
                                                                self.message_body(
                                                                    t,
                                                                    &e.blocks,
                                                                    (k + 1) * 1000,
                                                                    cx,
                                                                )
                                                                .pl(px(40.)),
                                                            )
                                                            .when(
                                                                !e.header.attachments.is_empty(),
                                                                |d| {
                                                                    d.child(
                                                                        self.attachment_row(
                                                                            t,
                                                                            &e.header.attachments,
                                                                            (k + 1) * 100,
                                                                            cx,
                                                                        )
                                                                        .pl(px(40.)),
                                                                    )
                                                                },
                                                            ),
                                                    )
                                                })
                                        },
                                    )))
                                })
                                .child(divider())
                                .child(self.message_header(t, m, 0, true, cx))
                                .child(self.message_body(t, &m.body, 0, cx).pl(px(40.)).when(
                                    !m.loaded,
                                    |d| {
                                        d.child(div().text_color(t.text3).child("Loading message…"))
                                    },
                                ))
                                .when(!m.header.attachments.is_empty(), |d| {
                                    d.child(
                                        self.attachment_row(t, &m.header.attachments, 0, cx)
                                            .pl(px(40.)),
                                    )
                                })
                                .when(m.rich, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .gap(px(10.))
                                            .pl(px(40.))
                                            .child(attachment(
                                                "Q4-Launch-Plan.pdf",
                                                "PDF · 4.2 MB",
                                                0xff7ab6,
                                                0xff4f7b,
                                            ))
                                            .child(attachment(
                                                "Pricing-tiers.fig",
                                                "Figma · 1.1 MB",
                                                0x8b7cff,
                                                0x5b47e0,
                                            )),
                                    )
                                }),
                        ),
                )
            })
            .when(original, |d| d.child(self.original_pane(t, m, cx)))
            .children(self.menu_view(t, cx))
    }

    /// Shown in place of the mail panes while signed out, signing in, loading or failed.
    fn status_screen(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, message, action): (&str, SharedString, Option<&str>) = match &self.status {
            Status::SignedOut => (
                "Connect your inbox",
                "Nimbra opens Google in your browser to sign in. It can read and archive \
                 your mail, and send what you write. It cannot permanently delete anything."
                    .into(),
                Some("Connect Gmail"),
            ),
            Status::SigningIn => (
                "Waiting for Google",
                "Finish signing in in your browser, then come back here.".into(),
                None,
            ),
            Status::Failed(reason) => ("Something went wrong", reason.clone(), Some("Try again")),
            _ => (
                "Fetching your inbox",
                "This takes a few seconds.".into(),
                None,
            ),
        };
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(t.surface)
            .child(serif(title, 56., t.text))
            .child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .text_color(t.text2)
                    .child(message),
            )
            .children(action.map(|label| {
                div().pt(px(10.)).child(
                    button(t, ButtonKind::Primary, label, None)
                        .id("status-action")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.gmail.is_some() {
                                this.fetch_inbox(cx);
                            } else {
                                this.sign_in(cx);
                            }
                        })),
                )
            }))
    }

    /// Placeholder rows that pulse in a wave while a mailbox is being fetched, laid out
    /// like the real list and reading pane so nothing jumps when the mail arrives.
    fn skeleton(&self, t: &Theme) -> impl IntoElement {
        let bar = |width: f32, height: f32| {
            div()
                .w(px(width))
                .h(px(height))
                .flex_none()
                .rounded(px(height / 2.))
                .bg(t.border_strong)
        };
        // Each item breathes on the same clock, offset by `phase` so the pulse travels down.
        let pulse = |key: usize, phase: f32, element: Div| {
            element.with_animation(
                ("skeleton", key),
                Animation::new(Duration::from_millis(1400))
                    .repeat()
                    .with_easing(move |delta| {
                        (((delta - phase) * std::f32::consts::TAU).sin() + 1.) / 2.
                    }),
                |element, glow| element.opacity(0.35 + 0.65 * glow),
            )
        };
        let rows = (0..9).map(|i| {
            let wide = [0.9, 0.7, 0.82, 0.6, 0.76, 0.88, 0.66, 0.8, 0.72][i];
            pulse(
                i,
                i as f32 * 0.09,
                div()
                    .flex()
                    .gap(px(12.))
                    .pl(px(26.))
                    .pr(px(14.))
                    .py(px(13.))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded_full()
                            .bg(t.border_strong),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(9.))
                            .pt(px(4.))
                            .child(bar(120. + 60. * wide, 10.))
                            .child(bar(330. * wide, 9.)),
                    ),
            )
        });
        let lines = [
            1.0, 0.94, 0.97, 0.6, 0.0, 0.98, 0.9, 0.95, 0.72, 0.0, 0.96, 0.5,
        ];

        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(t.surface)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(px(self.list_width))
                    .h_full()
                    .border_r_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .pl(px(24.))
                            .pt(px(if self.sidebar_open { 26. } else { 60. }))
                            .pb(px(22.))
                            .child(overline(self.inbox_name().to_uppercase(), t.text3)),
                    )
                    .children(rows),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(14.))
                    .px(px(64.))
                    .pt(px(96.))
                    .child(pulse(100, 0., div().child(bar(420., 30.))))
                    .child(pulse(101, 0.08, div().pb(px(18.)).child(bar(220., 10.))))
                    .children(
                        lines
                            .into_iter()
                            .enumerate()
                            .map(|(i, width): (usize, f32)| {
                                pulse(
                                    110 + i,
                                    0.16 + i as f32 * 0.05,
                                    div().child(if width == 0. {
                                        div().h(px(8.))
                                    } else {
                                        bar(560. * width, 9.)
                                    }),
                                )
                            }),
                    ),
            )
    }

    /// Shown while an email's content is on its way.
    fn loading_pane(&self, t: &Theme) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .gap(px(12.))
            .child(icon("reload", 22., t.violet).with_animation(
                "message-loading",
                Animation::new(Duration::from_millis(900)).repeat(),
                |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
            ))
            .child(meta("Loading message…", t.text3))
    }

    /// One message's header: sender, a one-line recipient summary that expands into every
    /// field (From, To, Cc, Reply-To, Date), and, for earlier messages, reply actions.
    fn message_header(
        &self,
        t: &Theme,
        mail: &Mail,
        index: usize,
        actions: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let (header, sender, tone, initials, time) = match index {
            0 => (
                &mail.header,
                mail.sender.clone(),
                mail.tone,
                mail.initials.clone(),
                mail.time.clone(),
            ),
            k => {
                let e = &mail.earlier[k - 1];
                (
                    &e.header,
                    e.sender.clone(),
                    e.tone,
                    e.initials.clone(),
                    e.when.clone(),
                )
            }
        };
        let thread = mail.id.clone().unwrap_or_default();
        let open = self.details.as_ref() == Some(&(thread.clone(), index));
        let me = self.account.to_string();
        let label = |p: &gmail::Person| {
            if p.address.eq_ignore_ascii_case(&me) {
                "me".to_string()
            } else {
                p.label().to_string()
            }
        };
        let recipients: Vec<String> = header.to.iter().chain(&header.cc).map(label).collect();
        let summary = match recipients.len() {
            0 if header.from.address.is_empty() => "to you".to_string(),
            0 => "to undisclosed recipients".to_string(),
            1..=3 => format!("to {}", recipients.join(", ")),
            n => format!("to {}, {} and {} more", recipients[0], recipients[1], n - 2),
        };
        let row = |name: &str, value: String| {
            div()
                .flex()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_color(t.text3)
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_color(t.text2)
                        .child(value),
                )
        };
        let person = |p: &gmail::Person| {
            if p.name.is_empty() {
                p.address.clone()
            } else {
                format!("{} <{}>", p.name, p.address)
            }
        };
        let list =
            |people: &[gmail::Person]| people.iter().map(person).collect::<Vec<_>>().join(", ");
        let source_index = index;
        let reply_button =
            |name: &'static str, tip_text: &'static str, kind: ComposeKind, id: usize| {
                icon_button(t, name)
                    .id(("message-action", id))
                    .tooltip(tip(t.dark, tip_text, ""))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let source =
                            this.mails
                                .get(this.selected)
                                .and_then(|mail| match source_index {
                                    0 => Some(Source::latest(mail)),
                                    k => mail.earlier.get(k - 1).map(|e| Source::earlier(mail, e)),
                                });
                        this.open_compose_from(kind, source, cx);
                    }))
            };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(avatar(tone, &initials, 28.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(sender))
                                    .when(!header.from.address.is_empty(), |d| {
                                        d.child(meta(header.from.address.clone(), t.text3))
                                    }),
                            )
                            .child(
                                div()
                                    .id(("recipients", index))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let key = (thread.clone(), index);
                                        this.details = if this.details.as_ref() == Some(&key) {
                                            None
                                        } else {
                                            Some(key)
                                        };
                                        cx.notify();
                                    }))
                                    .child(meta(short(&summary, 90), t.text3))
                                    .child(icon(if open { "up" } else { "down" }, 12., t.text3)),
                            ),
                    )
                    .when(actions, |d| {
                        d.child(reply_button(
                            "reply",
                            "Reply to this message",
                            ComposeKind::Reply,
                            index * 3,
                        ))
                        .child(reply_button(
                            "reply-all",
                            "Reply all to this message",
                            ComposeKind::ReplyAll,
                            index * 3 + 1,
                        ))
                        .child(reply_button(
                            "forward",
                            "Forward this message",
                            ComposeKind::Forward,
                            index * 3 + 2,
                        ))
                    })
                    .child(mono(time, t.text3)),
            )
            .when(open && !header.from.address.is_empty(), |d| {
                d.child(
                    div()
                        .ml(px(40.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .p(px(12.))
                        .rounded(px(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .child(row("From", person(&header.from)))
                        .when(!header.to.is_empty(), |d| {
                            d.child(row("To", list(&header.to)))
                        })
                        .when(!header.cc.is_empty(), |d| {
                            d.child(row("Cc", list(&header.cc)))
                        })
                        .when_some(header.reply_to.as_ref(), |d, p| {
                            d.child(row("Reply-To", person(p)))
                        })
                        .when(header.date_ms > 0, |d| {
                            d.child(row("Date", long_date(header.date_ms)))
                        }),
                )
            })
    }

    /// A message's attachments, each with Quick Look and Save.
    fn attachment_row(
        &self,
        t: &Theme,
        attachments: &[gmail::Attachment],
        base: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap(px(10.))
            .children(attachments.iter().enumerate().map(|(i, att)| {
                let (preview, save) = (att.clone(), att.clone());
                let kind = att
                    .name
                    .rsplit('.')
                    .next()
                    .filter(|e| *e != att.name)
                    .unwrap_or("file")
                    .to_uppercase();
                card(t)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .py(px(6.))
                    .max_w(px(360.))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(30.))
                            .rounded(px(8.))
                            .bg(gradient(135., c(0x8b7cff, 1.), c(0x5b47e0, 1.)))
                            .child(icon("file", 16., gpui::white())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(short(&att.name, 34)),
                            )
                            .child(meta(
                                [kind, file_size(att.size)]
                                    .into_iter()
                                    .filter(|s| !s.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                                t.text3,
                            )),
                    )
                    .child(
                        icon_button(t, "search")
                            .id(("attachment-preview", base + i))
                            .tooltip(tip(t.dark, "Quick Look", ""))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fetch_attachment(preview.clone(), false, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "down")
                            .id(("attachment-save", base + i))
                            .tooltip(tip(t.dark, "Save to Downloads", ""))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fetch_attachment(save.clone(), true, cx)
                            })),
                    )
            }))
    }

    /// The Spam warning and the Unsubscribe link for the open thread, when they apply.
    fn thread_banners(&self, t: &Theme, mail: &Mail, cx: &mut Context<Self>) -> Div {
        let spam = mail.labels.iter().any(|l| l == "SPAM");
        let trash = mail.labels.iter().any(|l| l == "TRASH");
        let unsubscribe = mail.header.unsubscribe.clone();
        let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
        let labels = self.thread_labels(mail);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .when(!labels.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(labels.into_iter().map(|(index, label)| {
                            let hover = t.hover;
                            let id = label.id.clone();
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .pl(px(8.))
                                .pr(px(3.))
                                .py(px(2.))
                                .rounded_full()
                                .bg(t.hover)
                                .border_1()
                                .border_color(t.border)
                                .child(
                                    div()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(tints[index % tints.len()]),
                                )
                                .child(meta(short(&label.name, 30), t.text2))
                                .child(
                                    div()
                                        .id(("remove-label", index))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(16.))
                                        .rounded_full()
                                        .cursor_pointer()
                                        .text_color(t.text3)
                                        .hover(move |s| s.bg(hover))
                                        .tooltip(tip(t.dark, "Remove label", ""))
                                        .child("×")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.remove_label(id.clone(), cx)
                                        })),
                                )
                        })),
                )
            })
            .when(trash, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .child(icon("trash", 15., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .child("In Trash. Gmail deletes it for good after 30 days."),
                        )
                        .child(
                            button(t, ButtonKind::Secondary, "Restore", None)
                                .id("restore")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.remove_selected(gmail::Change::Untrash, cx)
                                })),
                        ),
                )
            })
            .when(spam, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(t.rose.opacity(0.12))
                        .border_1()
                        .border_color(t.rose.opacity(0.4))
                        .child(icon("bell", 15., t.rose))
                        .child(div().flex_1().child(
                            "This message is in Spam. Be careful with its links and attachments.",
                        ))
                        .child(
                            button(t, ButtonKind::Secondary, "Not spam", None)
                                .id("not-spam")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.remove_selected(gmail::Change::NotSpam, cx)
                                })),
                        ),
                )
            })
            .when_some(unsubscribe, |d, link| {
                d.child(
                    div().flex().child(
                        button(t, ButtonKind::Ghost, "Unsubscribe", None)
                            .id("unsubscribe")
                            .tooltip(tip(t.dark, "Leave this mailing list", ""))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.unsubscribe(link.clone(), cx)
                            })),
                    ),
                )
            })
    }

    /// The area the web view covers, with a strip above it explaining what is blocked.
    fn original_pane(&self, t: &Theme, m: &Mail, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.html_view.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            // Subject, sender, recipients and attachments stay native above the email.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(12.))
                    .px(px(28.))
                    .pt(px(16.))
                    .pb(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(serif(&m.subject, 26., t.text))
                    .child(self.thread_banners(t, m, cx))
                    .child(self.message_header(t, m, 0, false, cx))
                    .when(!m.header.attachments.is_empty(), |d| {
                        d.child(self.attachment_row(t, &m.header.attachments, 0, cx).pl(px(40.)))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(28.))
                    .pr(px(20.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.border)
                    .when_some(self.hint, |d, (label, key)| {
                        // Hovering a toolbar button explains it here.
                        d.child(div().font_weight(FontWeight::MEDIUM).child(label))
                            .when(!key.is_empty(), |d| d.child(kbd(t, key)))
                            .child(div().flex_1())
                    })
                    .when(self.hint.is_none(), |d| {
                        d.child(icon("image", 15., t.text3))
                    .child(
                        meta(
                            if self.original_images {
                                "Original formatting. Scripts are off. Remote images are shown."
                            } else {
                                "Original formatting. Scripts are off and remote images are blocked."
                            },
                            t.text3,
                        )
                        .flex_1(),
                    )
                    })
                    .child(
                        button(
                            t,
                            ButtonKind::Secondary,
                            if self.original_images { "Block images" } else { "Load images" },
                            None,
                        )
                        .id("original-images")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.original_images = !this.original_images;
                            cx.notify();
                        })),
                    ),
            )
            // The web view is a native layer; this element only reserves its space and
            // tells it where that space is each frame.
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        if let Some(view) = &view {
                            view.set_bounds(
                                f32::from(bounds.origin.x),
                                f32::from(bounds.origin.y),
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                            );
                        }
                    },
                    |_, _, _, _| {},
                )
                .flex_1()
                .w_full()
                .bg(gpui::white()),
            )
    }

    /// The Labels or More menu, dropped under its toolbar button.
    fn menu_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let menu = self.menu?;
        let hover = t.hover;
        let item = move |id: usize, icon_name: &str, label: SharedString, tint: Hsla| {
            div()
                .id(("menu-item", id))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(icon(icon_name, 16., tint))
                .child(div().flex_1().child(label))
        };
        let panel = div()
            .id("menu")
            .absolute()
            .top(px(50.))
            .flex()
            .flex_col()
            .min_w(px(220.))
            .p(px(6.))
            .rounded(px(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![shadow(t.shadow, 14., 36., -6.)])
            .occlude()
            // Any click outside the menu closes it.
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.menu_dismissed = Some((menu, std::time::Instant::now()));
                cx.notify();
            }));
        Some(match menu {
            // These menus only exist as pop-up windows.
            Menu::Row(_) | Menu::SendLater | Menu::MoveTo | Menu::Accounts => return None,
            Menu::Labels => {
                let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                panel
                    .left(px(150.))
                    .when(self.labels.is_empty(), |d| {
                        d.child(div().px(px(10.)).py(px(7.)).text_color(t.text3).child(
                            if self.is_demo() {
                                "Labels need a connected Gmail account."
                            } else {
                                "You have no labels in Gmail yet."
                            },
                        ))
                    })
                    .children(
                        self.labels
                            .iter()
                            .enumerate()
                            .take(12)
                            .map(|(index, label)| {
                                item(
                                    index,
                                    "tag",
                                    short(&label.name, 28),
                                    tints[index % tints.len()],
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.apply_label(index, cx)),
                                )
                            }),
                    )
            }
            Menu::More => panel
                .right(px(20.))
                .child(
                    item(0, "inbox", "Mark as unread".into(), t.text2)
                        .on_click(cx.listener(|this, _, _, cx| this.mark_unread(cx))),
                )
                .when(!self.is_demo(), |d| {
                    d.child(
                        item(1, "link", "Open in Gmail".into(), t.text2)
                            .on_click(cx.listener(|this, _, _, cx| this.open_in_gmail(cx))),
                    )
                }),
        })
    }

    fn compose_view(
        &self,
        t: &Theme,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(composer) = &self.compose else {
            return div().into_any_element();
        };
        let full = composer.full;
        let (width, height) = composer.size;
        let (left, top) = composer
            .position
            .unwrap_or((((viewport.0 - width) / 2.).max(0.), 96.));
        // Full page keeps the text in a readable column instead of stretching it edge to edge.
        let column = |d: Div| {
            if full {
                d.w_full().max_w(px(860.))
            } else {
                d.w_full()
            }
        };
        let field = |label: &str,
                     input: &Entity<TextInput>,
                     lead: Vec<AnyElement>,
                     trailing: Option<AnyElement>,
                     below: Option<AnyElement>| {
            div()
                .flex()
                .justify_center()
                .border_b_1()
                .border_color(t.border)
                .child(
                    column(div())
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(20.))
                        .py(px(10.))
                        .child(
                            div()
                                .w(px(56.))
                                .flex_none()
                                .text_color(t.text3)
                                .child(label.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(14.))
                                .children(lead)
                                .child(div().flex_1().min_w(px(120.)).child(input.clone())),
                        )
                        .children(trailing)
                        .children(below),
                )
        };
        // Each finished address is a pill with a button to take it off.
        let pills = |field: Field, cx: &mut Context<Self>| -> Vec<AnyElement> {
            let Some(slot) = Composer::slot(field) else {
                return Vec::new();
            };
            composer.chips[slot]
                .iter()
                .enumerate()
                .map(|(i, address)| {
                    let hover = t.hover;
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .pl(px(9.))
                        .pr(px(3.))
                        .py(px(1.))
                        .rounded_full()
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border_strong)
                        .text_size(px(13.))
                        .child(short(address, 34))
                        .child(
                            div()
                                .id(("remove-recipient", slot * 1000 + i))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(16.))
                                .rounded_full()
                                .cursor_pointer()
                                .text_color(t.text3)
                                .hover(move |s| s.bg(hover))
                                .child("×")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(composer) = &mut this.compose
                                        && i < composer.chips[slot].len()
                                    {
                                        composer.chips[slot].remove(i);
                                        composer.sync_placeholder(field, cx);
                                        this.composer_changed(cx);
                                        cx.notify();
                                    }
                                })),
                        )
                        .into_any_element()
                })
                .collect()
        };
        // "Cc Bcc" opens the two extra recipient fields.
        let cc_toggle = (!composer.show_cc).then(|| {
            let (dim, bright) = (t.text3, t.text);
            div()
                .id("show-cc")
                .flex_none()
                .text_color(dim)
                .cursor_pointer()
                .hover(move |s| s.text_color(bright))
                .child("Cc Bcc")
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(composer) = &mut this.compose {
                        composer.show_cc = true;
                        let cc = composer.cc.clone();
                        this.pending_focus = Some(cc.read(cx).focus_handle(cx));
                        cx.notify();
                    }
                }))
                .into_any_element()
        });
        // Address suggestions, in a strip under the field. (Not floating over the fields
        // below: the message editor is a native view that would cover it.)
        let mut dropdown: Option<(Field, AnyElement)> = composer.suggest.as_ref().map(|s| {
            let list = div()
                .w(px(380.))
                .flex()
                .flex_col()
                .p(px(4.))
                .rounded(px(10.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![shadow(t.shadow, 12., 32., -6.)])
                .children(s.people.iter().enumerate().map(|(i, p)| {
                    let hover = t.hover;
                    div()
                        .id(("suggest", i))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .when(i == s.selected, |d| d.bg(t.hover))
                        .child(avatar(
                            crate::data::tone_for(p.label()),
                            &crate::data::initials(p.label()),
                            24.,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(short(p.label(), 44))
                                .when(!p.name.is_empty(), |d| {
                                    d.child(meta(short(&p.address, 50), t.text3))
                                }),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.accept_suggestion(Some(i), cx)),
                        )
                }));
            {
                let strip = div()
                    .flex()
                    .justify_center()
                    .border_b_1()
                    .border_color(t.border)
                    .child(column(div()).pl(px(84.)).pr(px(20.)).pb(px(8.)).child(list));
                (s.field, strip.into_any_element())
            }
        });
        let mut drop_for = |f: Field| -> Option<AnyElement> {
            if dropdown.as_ref().is_some_and(|(field, _)| *field == f) {
                dropdown.take().map(|(_, el)| el)
            } else {
                None
            }
        };
        // Attached files, each with a button to take it off.
        let chip = |id: usize, name: &str, size: u64| {
            let hover = t.hover;
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(10.))
                .pr(px(4.))
                .py(px(4.))
                .rounded(px(8.))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .child(icon("clip", 13., t.text3))
                .child(short(name, 32))
                .child(meta(file_size(size), t.text3))
                .child(
                    div()
                        .id(("remove-file", id))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .text_color(t.text3)
                        .hover(move |s| s.bg(hover))
                        .child("×"),
                )
        };
        let chips: Vec<AnyElement> = composer
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                chip(i, &f.name, f.bytes.len() as u64)
                    .id(("file-chip", i))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(composer) = &mut this.compose
                            && i < composer.files.len()
                        {
                            composer.files.remove(i);
                            this.composer_changed(cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            })
            .chain(composer.kept.iter().enumerate().map(|(i, att)| {
                chip(1000 + i, &att.name, att.size)
                    .id(("kept-chip", i))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(composer) = &mut this.compose
                            && i < composer.kept.len()
                        {
                            composer.kept.remove(i);
                            this.composer_changed(cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            }))
            .collect();
        let accent = t.violet;
        let card = div()
            .id("compose-card")
            .flex()
            .flex_col()
            .bg(t.raised)
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.attach_files(paths.paths().to_vec(), cx)
            }))
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(accent))
            .when(full, |d| d.size_full().bg(t.surface))
            .when(!full, |d| {
                d.absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(18.))
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.18), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
            })
            .child(
                div()
                    .flex()
                    .justify_center()
                    // In full page the header also clears the traffic lights and is the
                    // strip you drag or double-click.
                    .when(full, |d| {
                        d.pt(px(36.)).on_mouse_down(MouseButton::Left, drag_or_zoom)
                    })
                    .when(!full, |d| {
                        d.cursor(gpui::CursorStyle::OpenHand).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event, _, cx| {
                                this.grab_composer(Grab::Move, event, cx)
                            }),
                        )
                    })
                    .child(
                        column(div())
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(20.))
                            .pt(px(16.))
                            .pb(px(8.))
                            .child(
                                serif(
                                    if composer.draft_id.is_some() {
                                        "Draft"
                                    } else if composer.reply.is_some() {
                                        "Reply"
                                    } else {
                                        "New message"
                                    },
                                    if full { 38. } else { 26. },
                                    t.text,
                                )
                                .flex_1(),
                            )
                            .child(
                                icon_button(t, if full { "shrink" } else { "expand" })
                                    .id("compose-size")
                                    .tooltip(tip(t.dark, "Full page or floating", "⌘⇧F"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_full_compose(cx)),
                                    ),
                            )
                            .child(kbd(t, "esc")),
                    ),
            )
            .child(field(
                "To",
                &composer.to,
                pills(Field::To, cx),
                cc_toggle,
                None,
            ))
            .children(drop_for(Field::To))
            .when(composer.show_cc, |d| {
                d.child(field("Cc", &composer.cc, pills(Field::Cc, cx), None, None))
                    .children(drop_for(Field::Cc))
                    .child(field(
                        "Bcc",
                        &composer.bcc,
                        pills(Field::Bcc, cx),
                        None,
                        None,
                    ))
                    .children(drop_for(Field::Bcc))
            })
            .child(field("Subject", &composer.subject, Vec::new(), None, None))
            .when(!chips.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .justify_center()
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            column(div())
                                .flex()
                                .flex_wrap()
                                .gap(px(8.))
                                .px(px(20.))
                                .py(px(10.))
                                .children(chips),
                        ),
                )
            })
            .when(composer.rich, |d| {
                let editor = self.editor.clone();
                let tool = |id: usize,
                            name: &'static str,
                            label: &'static str,
                            key: &'static str,
                            command: &'static str,
                            active: bool| {
                    let hover = t.hover;
                    div()
                        .id(("format", id))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(28.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .when(active, |d| d.bg(t.hover).border_1().border_color(t.border))
                        .child(icon(name, 15., if active { t.text } else { t.text2 }))
                        .tooltip(tip(t.dark, label, key))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &this.editor {
                                editor.exec(command);
                            }
                            this.pending_focus = Some(this.editor_focus.clone());
                            cx.notify();
                        }))
                };
                let f = composer.format;
                let gap = || div().w(px(1.)).h(px(16.)).mx(px(4.)).bg(t.border);
                d.child(
                    div()
                        .flex()
                        .justify_center()
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            column(div())
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .px(px(14.))
                                .py(px(5.))
                                .child(tool(0, "bold", "Bold", "⌘B", "bold", f[0]))
                                .child(tool(1, "italic", "Italic", "⌘I", "italic", f[1]))
                                .child(tool(2, "underline", "Underline", "⌘U", "underline", f[2]))
                                .child(gap())
                                .child(tool(3, "link", "Link", "⌘K", "link", false))
                                .child(gap())
                                .child(tool(
                                    4,
                                    "bullets",
                                    "Bulleted list",
                                    "⌘⇧8",
                                    "insertUnorderedList",
                                    f[3],
                                ))
                                .child(tool(
                                    5,
                                    "numbers",
                                    "Numbered list",
                                    "⌘⇧7",
                                    "insertOrderedList",
                                    f[4],
                                ))
                                .child(tool(6, "quote", "Quote", "", "blockquote", false))
                                .child(gap())
                                .child(tool(
                                    7,
                                    "clear-format",
                                    "Clear formatting",
                                    "",
                                    "removeFormat",
                                    false,
                                )),
                        ),
                )
                // The editor is a native web view: this element only reserves its space and
                // tells it where that space is each frame.
                .child(
                    div()
                        .id("compose-body")
                        .flex_1()
                        .min_h(px(0.))
                        .key_context("Editor")
                        .track_focus(&self.editor_focus)
                        .child(
                            gpui::canvas(
                                move |bounds, _, _| {
                                    if let Some(editor) = &editor {
                                        editor.set_bounds(
                                            f32::from(bounds.origin.x),
                                            f32::from(bounds.origin.y),
                                            f32::from(bounds.size.width),
                                            f32::from(bounds.size.height),
                                        );
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .size_full(),
                        ),
                )
            })
            .when(!composer.rich, |d| {
                d.child(
                    div()
                        .id("compose-body")
                        .flex()
                        .justify_center()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(
                            column(div())
                                .flex()
                                .flex_col()
                                .px(px(20.))
                                .py(px(if full { 24. } else { 14. }))
                                .text_size(px(15.))
                                .children(composer.lines.iter().cloned())
                                // The quoted earlier message stays folded behind "…".
                                .when_some(composer.quoted.clone(), |d, quoted| {
                                    d.child(
                                        div().flex().pt(px(10.)).child(
                                            div()
                                                .id("toggle-quoted")
                                                .px(px(8.))
                                                .rounded(px(6.))
                                                .bg(t.hover)
                                                .border_1()
                                                .border_color(t.border)
                                                .text_color(t.text3)
                                                .cursor_pointer()
                                                .child("•••")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if let Some(composer) = &mut this.compose {
                                                        composer.show_quoted =
                                                            !composer.show_quoted;
                                                        cx.notify();
                                                    }
                                                })),
                                        ),
                                    )
                                    .when(
                                        composer.show_quoted,
                                        |d| {
                                            d.child(
                                                div()
                                                    .mt(px(10.))
                                                    .pl(px(14.))
                                                    .border_l_2()
                                                    .border_color(t.border_strong)
                                                    .text_size(px(14.))
                                                    .line_height(px(22.))
                                                    .text_color(t.text3)
                                                    .child(short(&quoted, 6000)),
                                            )
                                        },
                                    )
                                }),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_center()
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        column(div())
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(12.))
                            .child(
                                icon_button(t, "clip")
                                    .id("attach")
                                    .tooltip(tip(t.dark, "Attach files", ""))
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .pl(px(4.))
                                    .text_color(if composer.error.is_some() {
                                        t.rose
                                    } else {
                                        t.text3
                                    })
                                    .child(match &composer.error {
                                        Some(error) => short(error, 90),
                                        None if composer.sending => "Sending…".into(),
                                        None if composer.saving => "Saving…".into(),
                                        None if composer.saved => "Saved to Drafts".into(),
                                        None if self.gmail.is_none() => {
                                            "Sample mode: nothing is sent.".into()
                                        }
                                        None => "Drop files here to attach them.".into(),
                                    }),
                            )
                            .child(
                                icon_button(t, "trash")
                                    .id("discard")
                                    .tooltip(tip(t.dark, "Discard draft", ""))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.discard_compose(cx)),
                                    ),
                            )
                            .child(
                                button(t, ButtonKind::Secondary, "Send later", None)
                                    .id("send-later")
                                    .tooltip(tip(
                                        t.dark,
                                        "Sent from this Mac while Nimbra is running",
                                        "",
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_menu(Menu::SendLater, window, cx)
                                    })),
                            )
                            .child(
                                button(t, ButtonKind::Primary, "Send", Some("⌘↵"))
                                    .id("send")
                                    .when(composer.sending, |d| d.opacity(0.5))
                                    .on_click(cx.listener(|this, _, _, cx| this.send_mail(cx))),
                            ),
                    ),
            )
            // Bottom-right corner grip for resizing the floating card.
            .when(!full, |d| {
                d.child(
                    div()
                        .absolute()
                        .right_0()
                        .bottom_0()
                        .size(px(18.))
                        .cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event, _, cx| {
                                this.grab_composer(Grab::Resize, event, cx)
                            }),
                        )
                        .child(
                            div()
                                .absolute()
                                .right(px(5.))
                                .bottom(px(5.))
                                .size(px(8.))
                                .border_r_2()
                                .border_b_2()
                                .border_color(t.border_strong),
                        ),
                )
            });
        div()
            .id("compose-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .when(!full, |d| d.bg(t.scrim))
            // A grabbed composer follows the mouse anywhere in the window.
            .on_mouse_move(
                cx.listener(move |this, event, _, cx| this.drag_composer(event, viewport, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if let Some(composer) = &mut this.compose {
                        composer.grab = None;
                        cx.notify();
                    }
                }),
            )
            .occlude()
            .child(card)
            .into_any_element()
    }

    /// An empty mailbox other than the inbox (which gets the Inbox Zero screen).
    fn empty_mailbox(&self, t: &Theme) -> impl IntoElement {
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(t.surface)
            .child(serif(
                &if self.mailbox == View::Search {
                    "No results".to_string()
                } else {
                    format!("Nothing in {}", self.inbox_name())
                },
                56.,
                t.text,
            ))
            .child(
                div()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .text_color(t.text2)
                    .child(if self.mailbox == View::Search {
                        format!("Nothing matches “{}”.", short(&self.query, 60))
                    } else {
                        "Threads will show up here.".to_string()
                    }),
            )
    }

    fn zero(&self, t: &Theme) -> impl IntoElement {
        let ink = c(0xf5f3ef, 1.);
        let soft = c(0xffffff, 0.7);
        let glass = |d: Div| {
            d.bg(c(0xffffff, 0.07))
                .border_1()
                .border_color(c(0xffffff, 0.14))
                .rounded(px(14.))
        };
        let stat = |name: &str, color: Hsla, label: &str, value: &str| {
            glass(div())
                .flex()
                .flex_col()
                .gap(px(6.))
                .pl(px(20.))
                .pr(px(28.))
                .py(px(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(name, 16., color))
                        .child(meta(label.to_string(), soft)),
                )
                .child(
                    div()
                        .font_family(SERIF)
                        .text_size(px(40.))
                        .line_height(px(44.))
                        .text_color(ink)
                        .child(value.to_string()),
                )
        };
        // Deterministic star field.
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = (seed.wrapping_mul(9301).wrapping_add(49297)) % 233280;
            seed as f32 / 233280.
        };
        let stars: Vec<Div> = (0..48)
            .map(|_| {
                let (x, y, a) = (rnd(), rnd() * 0.42, 0.2 + rnd() * 0.6);
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .size(px(if a > 0.7 { 2.5 } else { 1.5 }))
                    .rounded_full()
                    .bg(c(0xffffff, a))
            })
            .collect();
        let layer = |angle: f32, color: u32, alpha: f32, end: f32| {
            div()
                .absolute()
                .inset_0()
                .bg(fade(angle, color, alpha, end))
        };

        div().flex_1().min_w(px(0.)).h_full().bg(t.surface).child(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .size_full()
                .overflow_hidden()
                .bg(c(0x0a0912, 1.))
                // Aurora: GPUI has no blur filter, so the glow is layered two-stop gradients.
                .child(layer(45., 0x6b4dff, 0.9, 0.62))
                .child(layer(315., 0xffa94d, 0.6, 0.5))
                .child(layer(0., 0xff4fa0, 0.55, 0.42))
                .child(layer(300., 0x3fa9ff, 0.22, 0.4))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(relative(0.72))
                        .h(px(1.))
                        .bg(c(0xffffff, 0.22)),
                )
                .children(stars)
                .child(div().h(px(72.)).flex_none())
                .child(overline("THURSDAY, NOVEMBER 20", c(0xffffff, 0.55)))
                .child(div().h(px(18.)).flex_none())
                .child(serif("Inbox zero.", 120., ink))
                .child(div().h(px(14.)).flex_none())
                .child(
                    div()
                        .text_size(px(17.))
                        .line_height(px(26.))
                        .text_color(soft)
                        .child(if self.is_demo() {
                            "Every important thread is handled. Go make something wonderful."
                        } else {
                            "Your inbox is empty. Go make something wonderful."
                        }),
                )
                .child(div().h(px(40.)).flex_none())
                // Stats and calendar are sample content; real mail shows the headline only.
                .when(self.is_demo(), |d| {
                    d.child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(stat(
                                "done",
                                t.mint,
                                "threads handled",
                                &(32 + self.history.len()).to_string(),
                            ))
                            .child(stat("clock", t.sky, "time in inbox", "38m"))
                            .child(stat("zap", t.amber, "zero streak", "8 days")),
                    )
                })
                .child(div().flex_1())
                .when(self.is_demo(), |d| {
                    d.child(
                        glass(div())
                            .bg(c(0x14131c, 0.72))
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .pl(px(14.))
                            .pr(px(10.))
                            .py(px(10.))
                            .text_color(ink)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(34.))
                                    .rounded(px(9.))
                                    .bg(Tone::Sky.gradient())
                                    .child(icon("calendar", 16., gpui::white())),
                            )
                            .child(
                                div()
                                    .mr(px(40.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Up next · Design crit"),
                                    )
                                    .child(meta(
                                        "2:00 PM · Studio B · in 42 min",
                                        c(0xffffff, 0.55),
                                    )),
                            )
                            .child(button(
                                &Theme::dark(),
                                ButtonKind::Secondary,
                                "Join",
                                Some("J"),
                            )),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .mt(px(18.))
                        .mb(px(36.))
                        .child(meta("Press", c(0xffffff, 0.5)))
                        .child(kbd(&Theme::dark(), "Z"))
                        .child(meta("to bring the last thread back", c(0xffffff, 0.5))),
                ),
        )
    }

    fn palette(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        // The sample data shows the designed palette; a real mailbox gets real snooze times.
        let rows: Vec<(
            &'static str,
            SharedString,
            SharedString,
            &'static str,
            Option<&'static str>,
        )> = if self.is_demo() {
            COMMANDS
                .iter()
                .map(|(name, label, hint, key, header)| {
                    (*name, (*label).into(), (*hint).into(), *key, *header)
                })
                .collect()
        } else {
            self.snooze_choices
                .iter()
                .map(|c| {
                    (
                        c.icon,
                        c.label.clone().into(),
                        c.hint.clone().into(),
                        "",
                        c.header,
                    )
                })
                .collect()
        };
        let items =
            rows.into_iter()
                .enumerate()
                .flat_map(|(i, (name, label, hint, key, header))| {
                    let active = i == self.palette_sel;
                    let hover = t.hover;
                    let head = header.map(|h| {
                        div()
                            .pl(px(12.))
                            .pt(px(12.))
                            .pb(px(6.))
                            .child(overline(h, t.faint))
                            .into_any_element()
                    });
                    let row = div()
                        .id(("command", i))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .pl(px(10.))
                        .pr(px(12.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .when(active, |d| d.bg(t.selected))
                        .when(!active, |d| d.hover(move |s| s.bg(hover)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.palette_sel = i;
                            this.run_command(cx)
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(28.))
                                .rounded(px(8.))
                                .when(active, |d| {
                                    d.bg(gradient(135., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
                                })
                                .when(!active, |d| d.bg(t.hover).border_1().border_color(t.border))
                                .child(icon(
                                    name,
                                    16.,
                                    if active { gpui::white() } else { t.text2 },
                                )),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_color(t.text3)
                                .child(hint.clone()),
                        )
                        .when(!key.is_empty(), |d| d.child(kbd(t, key)))
                        .into_any_element();
                    head.into_iter().chain(std::iter::once(row))
                });

        let hint = |keys: &[&str], label: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(keys.iter().map(|k| kbd(t, k)))
                .child(meta(label.to_string(), t.text3))
        };

        div()
            .id("scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(150.))
            .bg(t.scrim)
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.palette = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("palette")
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .w(px(660.))
                    .h_auto()
                    .max_h(px(520.))
                    .overflow_hidden()
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.22), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .pl(px(20.))
                            .pr(px(16.))
                            .py(px(18.))
                            .border_b_1()
                            .border_color(t.border)
                            .child(icon("search", 18., t.text3))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(px(2.))
                                    .text_size(px(17.))
                                    .line_height(px(24.))
                                    .child("snooze until i’m free")
                                    .child(div().w(px(2.)).h(px(20.)).rounded(px(1.)).bg(t.violet)),
                            )
                            .child(pill(
                                t,
                                m.subject.split(',').next().unwrap_or(&m.subject),
                                t.violet,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(8.))
                            .pt(px(6.))
                            .pb(px(8.))
                            .children(items),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .pl(px(20.))
                            .pr(px(18.))
                            .py(px(12.))
                            .bg(t.hover)
                            .border_t_1()
                            .border_color(t.border)
                            .child(hint(&["↑", "↓"], "Navigate"))
                            .child(hint(&["↵"], "Snooze"))
                            .child(hint(&["esc"], "Close"))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .font_family(SERIF)
                                    .italic()
                                    .text_size(px(18.))
                                    .line_height(px(20.))
                                    .text_color(t.text3)
                                    .child("Nimbra"),
                            ),
                    ),
            )
    }
}

impl Render for Nimbra {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus);
        }
        // "System" follows the Mac's light or dark appearance.
        if self.settings.theme == crate::settings::ThemeChoice::System
            && std::env::var_os("NIMBRA_THEME").is_none()
        {
            self.dark = matches!(
                window.appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            );
        }
        let t = self.theme();
        let viewport = window.viewport_size();
        let viewport = (f32::from(viewport.width), f32::from(viewport.height));
        // The floating composer needs a concrete position before it can be dragged.
        if let Some(composer) = &mut self.compose
            && composer.position.is_none()
        {
            composer.position = Some((((viewport.0 - composer.size.0) / 2.).max(0.), 96.));
        }
        self.sync_html_view(window);
        self.sync_editor(window, cx);
        // The checking-for-mail bar fades out when the check is done, instead of vanishing.
        let bar = self.fetching && !self.pull_hold && self.pull == 0.;
        if self.bar_shown && !bar {
            let n = self.bar_fade.map_or(0, |(_, n)| n + 1);
            self.bar_fade = Some((std::time::Instant::now(), n));
        } else if bar {
            self.bar_fade = None;
        }
        self.bar_shown = bar;
        if self
            .bar_fade
            .is_some_and(|(at, _)| at.elapsed() > Duration::from_millis(600))
        {
            self.bar_fade = None;
        }
        // The inbox's unread count on the Dock icon.
        let unread = if self.gmail.is_some() {
            self.counts.get("INBOX").map_or(0, |(unread, _)| *unread)
        } else {
            0
        };
        if self.badge != Some(unread) {
            crate::system::set_badge(unread);
            self.badge = Some(unread);
        }
        self.frame = Some(window.bounds());
        if self.pull_hold && !self.fetching {
            self.pull_hold = false;
            // Close from wherever the strip is right now, so a fast refresh does not jump.
            let now = self
                .pull_anim
                .map_or(PULL_REST, |(from, to, start)| settle(from, to, start));
            self.pull_anim = Some((now, 0., std::time::Instant::now()));
            self.pull_gen += 1;
        }
        let empty = self.visible().is_empty();

        div()
            .id("nimbra")
            .key_context(if self.compose.is_some() {
                "Composer"
            } else {
                "Mail"
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Prev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &MarkDone, _, cx| {
                if !this.palette {
                    this.mark_done(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(
                cx.listener(|this, _: &Compose, _, cx| this.open_compose(ComposeKind::New, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Reply, _, cx| this.open_compose(ComposeKind::Reply, cx)),
            )
            .on_action(cx.listener(|this, _: &SendMail, _, cx| this.send_mail(cx)))
            .on_action(cx.listener(|this, _: &SaveDraft, _, cx| this.save_draft(cx)))
            .on_action(cx.listener(|this, _: &ToggleOriginal, _, cx| this.toggle_original(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, _, cx| this.focus_search(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleFullCompose, _, cx| this.toggle_full_compose(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleStar, _, cx| this.toggle_star(cx)))
            .on_action(cx.listener(|this, _: &MarkUnread, _, cx| this.toggle_unread(cx)))
            .on_action(cx.listener(|this, _: &Trash, _, cx| {
                if !this.palette {
                    this.trash(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &TogglePalette, _, cx| {
                this.palette = !this.palette && !this.visible().is_empty();
                this.palette_sel = 0;
                this.snooze_choices = Self::snooze_choices();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Print, _, cx| this.print_selected(cx)))
            .on_action(
                cx.listener(|this, _: &MenuReply, _, cx| this.open_compose(ComposeKind::Reply, cx)),
            )
            .on_action(cx.listener(|this, _: &MenuMarkDone, _, cx| this.mark_done(cx)))
            .on_action(cx.listener(|this, _: &MenuMarkUnread, _, cx| this.toggle_unread(cx)))
            .on_action(cx.listener(|this, _: &MenuStar, _, cx| this.toggle_star(cx)))
            .on_action(cx.listener(|this, _: &MenuTrash, _, cx| this.trash(cx)))
            .on_action(cx.listener(|this, _: &MenuUndo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &MenuToggleOriginal, _, cx| this.toggle_original(cx)))
            .on_action(cx.listener(|this, _: &MenuToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &MenuSearch, _, cx| this.focus_search(cx)))
            .on_action(cx.listener(|this, _: &MenuSnooze, _, cx| {
                if this.compose.is_none() && !this.visible().is_empty() {
                    this.palette = true;
                    this.palette_sel = 0;
                    this.snooze_choices = Self::snooze_choices();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAllThreads, _, cx| {
                let visible = this.visible();
                if visible.len() >= 2 {
                    this.picked = visible.into_iter().collect();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                if this.menu.take().is_some() {
                    cx.notify();
                } else if this.settings_open {
                    this.close_settings(cx);
                } else if this.new_label.is_some() {
                    this.close_new_label(cx);
                } else if !this.picked.is_empty() && !this.palette {
                    this.picked.clear();
                    cx.notify();
                } else if let Some(composer) = this.compose.as_mut()
                    && composer.suggest.take().is_some()
                {
                    cx.notify();
                } else if this.compose.is_some() {
                    this.close_compose(cx);
                } else if this.mailbox == View::Search || !this.search.read(cx).text().is_empty() {
                    this.clear_search(cx);
                } else {
                    this.palette = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| {
                if this.palette {
                    this.run_command(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.open_settings(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &Forward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
            .on_action(|_: &HideWindow, window, _| crate::blur::set_visible(window, false))
            .on_action(|_: &Quit, _, cx: &mut App| cx.quit())
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                let Some((start, width)) = this.list_grab else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.list_grab = None;
                    return;
                }
                this.list_width = (width + f32::from(event.position.x) - start).clamp(300., 720.);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.list_grab = None),
            )
            .on_action(cx.listener(|this, _: &ToggleList, _, cx| this.toggle_list(cx)))
            .relative()
            .flex()
            .size_full()
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                self.with_sidebar_progress(
                    "sidebar-width",
                    div()
                        .flex_none()
                        .h_full()
                        .overflow_hidden()
                        .child(self.sidebar(&t, cx)),
                    |d, open| d.w(px(236. * open)),
                ),
            )
            .map(|d| match self.status {
                Status::Demo | Status::Ready if empty && self.mailbox == View::Inbox => {
                    d.child(self.zero(&t))
                }
                Status::Demo | Status::Ready if empty => d.child(self.empty_mailbox(&t)),
                Status::Demo | Status::Ready => d.children(self.list_slot(&t, cx)).map(|d| {
                    if self.picked.len() >= 2 {
                        d.child(self.bulk_pane(&t, cx))
                    } else {
                        d.child(self.reading(&t, cx))
                    }
                }),
                Status::Loading => d.child(self.skeleton(&t)),
                _ => d.child(self.status_screen(&t, cx)),
            })
            .child(self.window_controls(&t, cx))
            .when(self.palette && !empty, |d| d.child(self.palette(&t, cx)))
            .when(self.compose.is_some(), |d| {
                d.child(self.compose_view(&t, viewport, cx))
            })
            .when(self.settings_open, |d| d.child(self.settings_view(&t, cx)))
            .when(self.new_label.is_some(), |d| {
                d.child(self.new_label_view(&t, cx))
            })
            .when_some(self.notice.clone(), |d, notice| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(24.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("notice")
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .max_w(px(560.))
                                .pl(px(16.))
                                .pr(px(8.))
                                .py(px(8.))
                                .rounded(px(12.))
                                .bg(t.raised)
                                .border_1()
                                .border_color(t.border_strong)
                                .shadow(vec![shadow(t.shadow, 12., 32., -6.)])
                                .occlude()
                                .child(div().flex_1().child(notice))
                                .child(
                                    button(&t, ButtonKind::Ghost, "Dismiss", None)
                                        .id("dismiss-notice")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.notice = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
    }
}
