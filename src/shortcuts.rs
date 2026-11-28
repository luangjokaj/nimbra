//! The mail actions' keyboard shortcuts, which can be changed in Settings › Shortcuts.
//!
//! One table says what each action is called, where Settings lists it, its keys out of the
//! box and the GPUI action it runs. Keys the user changed are kept per Mac in
//! `shortcuts.json`, only for the actions that differ from their defaults. The keys that
//! stay as they are (⌘Q, ⌘C, zoom, the composer's) are bound in `main.rs`.
//!
//! A key is written as GPUI writes it ("cmd-f", "shift-j", "g i" for G then I). A key
//! without ⌘ or ⌃ belongs to the mail view alone, so it never fires while typing; a key
//! with one of them takes the action's own context (`Shortcut::chord`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex, RwLock};

use gpui::{Action, KeyBinding, KeyBindingContextPredicate, Keystroke, SharedString};

use crate::app::{
    Back, Compose, Confirm, First, FocusSearch, FocusSidebar, Forward, ForwardMessage, GoAllMail,
    GoDrafts, GoInbox, GoSent, GoSnoozed, GoStarred, HideApp, HideWindow, Last, MarkDone,
    MarkUnread, Minimize, Next, OpenInWindow, OpenSettings, PageDown, PageUp, PickDown, PickThread,
    PickUp, Prev, Print, PrintMessage, Reload, Reply, ReplyAll, SaveDraft, SelectAllThreads,
    SendMail, ToggleFullCompose, ToggleList, ToggleOriginal, TogglePalette, ToggleShortcuts,
    ToggleSidebar, ToggleStar, ToggleTheme, Trash, Undo,
};

/// The mail view, while no text field has the keyboard.
pub const MAIL: &str = "Mail && !TextInput";
/// The composer, and the message editor in it (a web view, whose page runs the formatting
/// keys itself: `editor_keys`).
pub const COMPOSER: &str = "Composer";
pub const EDITOR: &str = "Editor";

/// Where Settings › Shortcuts lists an action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Reading,
    Organising,
    Writing,
    App,
}

impl Section {
    pub const ALL: [Section; 4] = [
        Section::Reading,
        Section::Organising,
        Section::Writing,
        Section::App,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Reading => "Reading",
            Section::Organising => "Organising",
            Section::Writing => "Writing",
            Section::App => "App",
        }
    }
}

/// A mail action whose keys can be changed.
pub struct Shortcut {
    /// Its name in `shortcuts.json`, which never changes.
    pub id: &'static str,
    /// What Settings and the conflict prompt call it.
    pub name: &'static str,
    pub section: Section,
    /// Its keys out of the box. The first is the one tooltips show; the ⌃ twins of ⌘ keys,
    /// for keyboards without ⌘, come last.
    pub defaults: &'static [&'static str],
    /// The context of its keys with ⌘ or ⌃ (`None`: anywhere in the app).
    pub chord: Option<&'static str>,
    pub action: fn() -> Box<dyn Action>,
}

fn boxed<A: Action + Default>() -> Box<dyn Action> {
    Box::new(A::default())
}

const fn s(
    id: &'static str,
    name: &'static str,
    section: Section,
    defaults: &'static [&'static str],
    chord: Option<&'static str>,
    action: fn() -> Box<dyn Action>,
) -> Shortcut {
    Shortcut {
        id,
        name,
        section,
        defaults,
        chord,
        action,
    }
}

const M: Option<&str> = Some(MAIL);
const C: Option<&str> = Some(COMPOSER);
const E: Option<&str> = Some(EDITOR);

/// Every action whose keys can be changed, in the order Settings lists them.
pub static TABLE: &[Shortcut] = {
    use Section::*;
    &[
        s(
            "next",
            "Next conversation",
            Reading,
            &["j", "down"],
            M,
            boxed::<Next>,
        ),
        s(
            "prev",
            "Previous conversation",
            Reading,
            &["k", "up"],
            M,
            boxed::<Prev>,
        ),
        s(
            "full-view",
            "Full view",
            Reading,
            &["enter"],
            M,
            boxed::<Confirm>,
        ),
        s(
            "toggle-original",
            "Original formatting or reader",
            Reading,
            &["v"],
            M,
            boxed::<ToggleOriginal>,
        ),
        s("back", "Go back", Reading, &["cmd-["], None, boxed::<Back>),
        s(
            "forward",
            "Go forward",
            Reading,
            &["cmd-]"],
            None,
            boxed::<Forward>,
        ),
        s(
            "first",
            "First conversation",
            Reading,
            &["home", "cmd-up"],
            M,
            boxed::<First>,
        ),
        s(
            "last",
            "Last conversation",
            Reading,
            &["end", "cmd-down"],
            M,
            boxed::<Last>,
        ),
        s(
            "page-down",
            "Page down",
            Reading,
            &["space"],
            M,
            boxed::<PageDown>,
        ),
        s(
            "page-up",
            "Page up",
            Reading,
            &["shift-space"],
            M,
            boxed::<PageUp>,
        ),
        s(
            "open-in-window",
            "Open in its own window",
            Reading,
            &["o"],
            M,
            boxed::<OpenInWindow>,
        ),
        s(
            "focus-sidebar",
            "Move to the sidebar",
            Reading,
            &["left"],
            M,
            boxed::<FocusSidebar>,
        ),
        s(
            "go-inbox",
            "Go to Inbox",
            Reading,
            &["g i"],
            M,
            boxed::<GoInbox>,
        ),
        s(
            "go-starred",
            "Go to Starred",
            Reading,
            &["g s"],
            M,
            boxed::<GoStarred>,
        ),
        s(
            "go-snoozed",
            "Go to Snoozed",
            Reading,
            &["g b"],
            M,
            boxed::<GoSnoozed>,
        ),
        s(
            "go-sent",
            "Go to Sent",
            Reading,
            &["g t"],
            M,
            boxed::<GoSent>,
        ),
        s(
            "go-drafts",
            "Go to Drafts",
            Reading,
            &["g d"],
            M,
            boxed::<GoDrafts>,
        ),
        s(
            "go-all-mail",
            "Go to All Mail",
            Reading,
            &["g a"],
            M,
            boxed::<GoAllMail>,
        ),
        s(
            "mark-done",
            "Mark done",
            Organising,
            &["e"],
            M,
            boxed::<MarkDone>,
        ),
        s(
            "mark-unread",
            "Read or unread",
            Organising,
            &["u"],
            M,
            boxed::<MarkUnread>,
        ),
        s(
            "toggle-star",
            "Star",
            Organising,
            &["s"],
            M,
            boxed::<ToggleStar>,
        ),
        s(
            "snooze",
            "Snooze",
            Organising,
            &["h", "cmd-k", "ctrl-k"],
            None,
            boxed::<TogglePalette>,
        ),
        s(
            "trash",
            "Move to trash",
            Organising,
            &["backspace", "delete", "cmd-backspace", "ctrl-backspace"],
            M,
            boxed::<Trash>,
        ),
        s(
            "undo",
            "Undo",
            Organising,
            &["z", "cmd-z", "ctrl-z"],
            Some("Mail"),
            boxed::<Undo>,
        ),
        s(
            "pick-thread",
            "Pick the open conversation",
            Organising,
            &["x"],
            M,
            boxed::<PickThread>,
        ),
        s(
            "pick-down",
            "Pick the next one too",
            Organising,
            &["shift-down", "shift-j"],
            M,
            boxed::<PickDown>,
        ),
        s(
            "pick-up",
            "Pick the one above too",
            Organising,
            &["shift-up", "shift-k"],
            M,
            boxed::<PickUp>,
        ),
        s(
            "select-all",
            "Select all",
            Organising,
            &["cmd-a", "ctrl-a"],
            M,
            boxed::<SelectAllThreads>,
        ),
        s(
            "compose",
            "New message",
            Writing,
            &["c", "cmd-n"],
            None,
            boxed::<Compose>,
        ),
        s("reply", "Reply", Writing, &["r"], M, boxed::<Reply>),
        s(
            "reply-all",
            "Reply all",
            Writing,
            &["a"],
            M,
            boxed::<ReplyAll>,
        ),
        s(
            "forward-message",
            "Forward",
            Writing,
            &["f"],
            M,
            boxed::<ForwardMessage>,
        ),
        s(
            "send",
            "Send",
            Writing,
            &["cmd-enter", "ctrl-enter"],
            C,
            boxed::<SendMail>,
        ),
        s(
            "save-draft",
            "Save draft",
            Writing,
            &["cmd-s", "ctrl-s"],
            C,
            boxed::<SaveDraft>,
        ),
        s(
            "full-compose",
            "Full page composer",
            Writing,
            &["cmd-shift-f", "ctrl-shift-f"],
            C,
            boxed::<ToggleFullCompose>,
        ),
        s(
            "bold",
            "Bold",
            Writing,
            &["cmd-b", "ctrl-b"],
            E,
            boxed::<gpui::NoAction>,
        ),
        s(
            "italic",
            "Italic",
            Writing,
            &["cmd-i", "ctrl-i"],
            E,
            boxed::<gpui::NoAction>,
        ),
        s(
            "underline",
            "Underline",
            Writing,
            &["cmd-u", "ctrl-u"],
            E,
            boxed::<gpui::NoAction>,
        ),
        s(
            "link",
            "Link",
            Writing,
            &["cmd-k", "ctrl-k"],
            E,
            boxed::<gpui::NoAction>,
        ),
        s(
            "focus-search",
            "Search",
            App,
            &["/", "cmd-f", "ctrl-f"],
            Some("Mail"),
            boxed::<FocusSearch>,
        ),
        s(
            "reload",
            "Check for mail",
            App,
            &["cmd-r", "ctrl-r"],
            None,
            boxed::<Reload>,
        ),
        s(
            "toggle-sidebar",
            "Show or hide sidebar",
            App,
            &["cmd-b", "ctrl-b"],
            None,
            boxed::<ToggleSidebar>,
        ),
        s(
            "toggle-list",
            "Show or hide message list",
            App,
            &["cmd-shift-b", "ctrl-shift-b"],
            None,
            boxed::<ToggleList>,
        ),
        s(
            "toggle-theme",
            "Switch light and dark",
            App,
            &["cmd-shift-l", "ctrl-shift-l"],
            None,
            boxed::<ToggleTheme>,
        ),
        s(
            "shortcuts",
            "Keyboard shortcuts",
            App,
            &["?", "shift-/"],
            M,
            boxed::<ToggleShortcuts>,
        ),
        s(
            "settings",
            "Settings",
            App,
            &["cmd-,"],
            None,
            boxed::<OpenSettings>,
        ),
        s(
            "print",
            "Print conversation",
            App,
            &["cmd-p"],
            None,
            boxed::<Print>,
        ),
        s(
            "print-message",
            "Print this message",
            App,
            &["cmd-shift-p"],
            None,
            boxed::<PrintMessage>,
        ),
        s(
            "close-window",
            "Close window",
            App,
            &["cmd-w"],
            None,
            boxed::<HideWindow>,
        ),
        s(
            "minimise",
            "Minimise",
            App,
            &["cmd-m"],
            None,
            boxed::<Minimize>,
        ),
        s(
            "hide-app",
            "Hide Nimbra",
            App,
            &["cmd-h"],
            None,
            boxed::<HideApp>,
        ),
    ]
};

pub fn find(id: &str) -> Option<&'static Shortcut> {
    TABLE.iter().find(|s| s.id == id)
}

// ------------------------------------------------------------------ keys

/// The keys changed from their defaults, by action id. An empty list: no key at all.
pub type Overrides = BTreeMap<String, Vec<String>>;

/// The overrides in effect, read by everything that shows a key.
static CURRENT: LazyLock<RwLock<Overrides>> = LazyLock::new(Default::default);

pub fn current() -> Overrides {
    CURRENT.read().map(|o| o.clone()).unwrap_or_default()
}

pub fn set_current(overrides: Overrides) {
    if let Ok(mut current) = CURRENT.write() {
        *current = overrides;
    }
}

/// The names GPUI gives to modifiers pressed on their own, which are no key.
const MODIFIER_NAMES: [&str; 5] = ["shift", "control", "alt", "platform", "function"];

/// A key in one spelling (GPUI's own, modifiers in its order), or `None` when it is no key
/// a binding can take. A sequence has two steps at most.
pub fn normalize(key: &str) -> Option<String> {
    let steps = key
        .split_whitespace()
        .map(|step| {
            let parsed = Keystroke::parse(step).ok()?;
            (!MODIFIER_NAMES.contains(&parsed.key.as_str())).then(|| parsed.unparse())
        })
        .collect::<Option<Vec<_>>>()?;
    (!steps.is_empty() && steps.len() <= 2).then(|| steps.join(" "))
}

/// Whether the key has ⌘ or ⌃ (in any step).
fn is_chord(key: &str) -> bool {
    key.split_whitespace().any(|step| {
        Keystroke::parse(step).is_ok_and(|k| k.modifiers.platform || k.modifiers.control)
    })
}

/// Where a key of the action applies. The editor's keys apply in the editor alone.
pub fn context_for(shortcut: &Shortcut, key: &str) -> Option<&'static str> {
    if shortcut.chord == Some(EDITOR) {
        Some(EDITOR)
    } else if is_chord(key) {
        shortcut.chord
    } else {
        Some(MAIL)
    }
}

/// Whether the action's key must have ⌘ or ⌃: the composer's and the editor's run while
/// typing, so a plain key would type nothing else.
pub fn needs_chord(shortcut: &Shortcut) -> bool {
    matches!(shortcut.chord, Some(COMPOSER) | Some(EDITOR))
}

/// Whether `key` can be given to the action: one step, with ⌘ or ⌃, where typing goes on.
pub fn fits(shortcut: &Shortcut, key: &str) -> bool {
    !needs_chord(shortcut) || (!key.contains(' ') && is_chord(key))
}

fn defaults_of(shortcut: &Shortcut) -> Vec<String> {
    shortcut
        .defaults
        .iter()
        .filter_map(|key| normalize(key))
        .collect()
}

/// The action's keys with `overrides`.
pub fn keys_in(overrides: &Overrides, shortcut: &Shortcut) -> Vec<String> {
    overrides
        .get(shortcut.id)
        .cloned()
        .unwrap_or_else(|| defaults_of(shortcut))
}

/// The action's keys now.
pub fn keys(id: &str) -> Vec<String> {
    find(id).map_or_else(Vec::new, |s| keys_in(&current(), s))
}

/// A binding of `key` to `action` in `context`, or `None` when GPUI cannot take the key.
fn binding(key: &str, action: Box<dyn Action>, context: Option<&str>) -> Option<KeyBinding> {
    let context = context
        .and_then(|c| KeyBindingContextPredicate::parse(c).ok())
        .map(Rc::new);
    KeyBinding::load(
        key,
        action,
        context,
        false,
        None,
        &gpui::DummyKeyboardMapper,
    )
    .ok()
}

/// Every binding of the table with `overrides`. The message editor's (its formatting, and
/// the composer's keys, which its page reports itself) come last, so they win there over
/// the app's (⌘B shows the sidebar everywhere else).
pub fn bindings(overrides: &Overrides) -> Vec<KeyBinding> {
    let mut app = Vec::new();
    let mut editor = Vec::new();
    for shortcut in TABLE {
        for key in keys_in(overrides, shortcut) {
            let context = context_for(shortcut, &key);
            let made = binding(&key, (shortcut.action)(), context);
            if context == Some(EDITOR) {
                editor.extend(made);
                continue;
            }
            app.extend(made);
            if shortcut.chord == Some(COMPOSER) {
                editor.extend(binding(&key, Box::new(gpui::NoAction), Some(EDITOR)));
            }
        }
    }
    app.extend(editor);
    app
}

/// The keys the message editor's page runs itself, as GPUI writes them, by what the page
/// calls them: Send, Save draft and Full page composer (reported to the app) and the
/// formatting. One-step keys only: the page sees one key at a time.
pub fn editor_keys() -> serde_json::Value {
    let of =
        |id: &str| -> Vec<String> { keys(id).into_iter().filter(|k| !k.contains(' ')).collect() };
    serde_json::json!({
        "send": of("send"),
        "save": of("save-draft"),
        "full": of("full-compose"),
        "bold": of("bold"),
        "italic": of("italic"),
        "underline": of("underline"),
        "link": of("link"),
    })
}

// ------------------------------------------------------------------ glyphs

/// A key as the Mac writes it: "cmd-shift-b" is ⇧⌘B, "g i" is G I.
pub fn glyph(key: &str) -> String {
    key.split_whitespace()
        .map(step_glyph)
        .collect::<Vec<_>>()
        .join(" ")
}

fn step_glyph(step: &str) -> String {
    let Ok(k) = Keystroke::parse(step) else {
        return step.to_string();
    };
    let m = k.modifiers;
    // ⇧/ is ? on the keyboards GPUI spells it for.
    if k.key == "/" && m.shift && !m.control && !m.alt && !m.platform && !m.function {
        return "?".into();
    }
    let mut out = String::new();
    if m.function {
        out.push_str("fn ");
    }
    if m.control {
        out.push('⌃');
    }
    if m.alt {
        out.push('⌥');
    }
    if m.shift {
        out.push('⇧');
    }
    if m.platform {
        out.push('⌘');
    }
    let name = match k.key.as_str() {
        "enter" => "↵".to_string(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "escape" => "esc".into(),
        "space" => "Space".into(),
        "tab" => "⇥".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        "insert" => "Ins".into(),
        key if key.chars().count() == 1 => key.to_uppercase(),
        key if key.starts_with('f') && key[1..].chars().all(|c| c.is_ascii_digit()) => {
            key.to_uppercase()
        }
        key => {
            let mut chars = key.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    };
    out.push_str(&name);
    out
}

/// The keys as caps, one per glyph (? and ⇧/ are one cap), each with the keys it stands for.
pub fn caps(keys: &[String]) -> Vec<(String, Vec<String>)> {
    let mut caps: Vec<(String, Vec<String>)> = Vec::new();
    for key in keys {
        let glyph = glyph(key);
        match caps.iter_mut().find(|(g, _)| *g == glyph) {
            Some((_, keys)) => keys.push(key.clone()),
            None => caps.push((glyph, vec![key.clone()])),
        }
    }
    caps
}

/// The keys to show where room is short (the ? sheet, tooltips, the menu bar): without
/// the ⌃ twin of a ⌘ key.
pub fn compact(keys: &[String]) -> Vec<String> {
    let twin_of = |key: &str| -> Option<String> {
        let k = Keystroke::parse(key).ok()?;
        if key.contains(' ') || !k.modifiers.control || k.modifiers.platform {
            return None;
        }
        let mut cmd = k.clone();
        cmd.modifiers.control = false;
        cmd.modifiers.platform = true;
        Some(cmd.unparse())
    };
    let shown: Vec<String> = keys
        .iter()
        .filter(|key| twin_of(key).is_none_or(|cmd| !keys.contains(&cmd)))
        .cloned()
        .collect();
    caps(&shown).into_iter().map(|(glyph, _)| glyph).collect()
}

/// The key a tooltip or hint shows for the action: its first, or nothing.
pub fn label(id: &str) -> SharedString {
    compact(&keys(id))
        .into_iter()
        .next()
        .map(SharedString::from)
        .unwrap_or_default()
}

// ------------------------------------------------------------------ changes

/// Whether two keys clash: the same, or one starts the other (G and G I).
fn clashes(a: &str, b: &str) -> bool {
    a == b || b.starts_with(&format!("{a} ")) || a.starts_with(&format!("{b} "))
}

/// Who has a key already.
#[derive(Debug, PartialEq)]
pub enum Taken {
    Free,
    /// The action itself.
    Own,
    /// A key that cannot be changed, by what it does.
    Fixed(&'static str),
    /// Other actions, by id.
    Used(Vec<&'static str>),
}

/// Who has `key` (normalized) besides `id`. `fixed` is the keys that stay, with their names.
pub fn taken(
    overrides: &Overrides,
    id: &str,
    key: &str,
    fixed: &[(String, &'static str)],
) -> Taken {
    if let Some((_, name)) = fixed.iter().find(|(k, _)| clashes(k, key)) {
        return Taken::Fixed(name);
    }
    let Some(own) = find(id) else {
        return Taken::Free;
    };
    let mut used = Vec::new();
    for shortcut in TABLE.iter().filter(|s| meet(own, s)) {
        if keys_in(overrides, shortcut).iter().any(|k| clashes(k, key)) {
            if shortcut.id == id {
                if keys_in(overrides, shortcut).iter().any(|k| k == key) {
                    return Taken::Own;
                }
            } else {
                used.push(shortcut.id);
            }
        }
    }
    if used.is_empty() {
        Taken::Free
    } else {
        Taken::Used(used)
    }
}

/// Whether two actions' keys can meet: the message editor's formatting is only ever pressed
/// in the editor, where it wins, so it shares keys with the app's (⌘B is Bold there and the
/// sidebar everywhere else). The composer's keys run in the editor too, so they meet both.
fn meet(a: &Shortcut, b: &Shortcut) -> bool {
    let editor = |s: &Shortcut| s.chord == Some(EDITOR);
    let app = |s: &Shortcut| !editor(s) && s.chord != Some(COMPOSER);
    !(editor(a) && app(b) || editor(b) && app(a))
}

/// Keeps only what differs from the defaults.
fn put(overrides: &mut Overrides, shortcut: &Shortcut, keys: Vec<String>) {
    if keys == defaults_of(shortcut) {
        overrides.remove(shortcut.id);
    } else {
        overrides.insert(shortcut.id.to_string(), keys);
    }
}

/// Gives `id` the one key `key`, in place of the keys it had, and takes it (and any key it
/// clashes with) from every other action, which may be left with none.
pub fn replace(overrides: &Overrides, id: &str, key: &str) -> Overrides {
    set_keys(overrides, id, &[key.to_string()])
}

/// Gives `id` exactly `keys`, taking each from the other actions.
fn set_keys(overrides: &Overrides, id: &str, keys: &[String]) -> Overrides {
    let mut next = overrides.clone();
    let Some(own) = find(id) else {
        return next;
    };
    for shortcut in TABLE {
        if shortcut.id == id {
            put(&mut next, shortcut, keys.to_vec());
            continue;
        }
        if !meet(own, shortcut) {
            continue;
        }
        let had = keys_in(overrides, shortcut);
        let kept: Vec<String> = had
            .iter()
            .filter(|k| !keys.iter().any(|key| clashes(k, key)))
            .cloned()
            .collect();
        if kept != had {
            put(&mut next, shortcut, kept);
        }
    }
    next
}

/// The other actions that would lose a key if `id` took `keys`.
pub fn losers(overrides: &Overrides, id: &str, keys: &[String]) -> Vec<&'static str> {
    let Some(own) = find(id) else {
        return Vec::new();
    };
    TABLE
        .iter()
        .filter(|shortcut| shortcut.id != id && meet(own, shortcut))
        .filter(|shortcut| {
            keys_in(overrides, shortcut)
                .iter()
                .any(|k| keys.iter().any(|key| clashes(k, key)))
        })
        .map(|shortcut| shortcut.id)
        .collect()
}

/// The action's own keys out of the box.
pub fn defaults(id: &str) -> Vec<String> {
    find(id).map(defaults_of).unwrap_or_default()
}

/// Whether the action's keys are not its defaults.
pub fn changed(overrides: &Overrides, id: &str) -> bool {
    overrides.contains_key(id)
}

/// The action's keys back to its defaults, taken from any action that has one of them now
/// (`losers` says which, to ask first).
pub fn reset_taking(overrides: &Overrides, id: &str) -> Overrides {
    let mut next = set_keys(overrides, id, &defaults(id));
    next.remove(id);
    next
}

/// Leaves `id` with no key at all.
pub fn clear(overrides: &Overrides, id: &str) -> Overrides {
    set_keys(overrides, id, &[])
}

// ------------------------------------------------------------------ the file

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nimbra/shortcuts.json"))
}

/// The overrides in a file's bytes: actions Nimbra knows, keys it can bind. Anything else
/// (a file that is not JSON, say) is left out, and leaves the defaults.
pub fn parse(bytes: &[u8]) -> Option<Overrides> {
    let raw: BTreeMap<String, Vec<String>> = serde_json::from_slice(bytes).ok()?;
    Some(
        raw.into_iter()
            .filter_map(|(id, keys)| {
                let shortcut = find(&id)?;
                let mut clean: Vec<String> = Vec::new();
                for key in keys.iter().filter_map(|k| normalize(k)) {
                    if !clean.contains(&key) {
                        clean.push(key);
                    }
                }
                (clean != defaults_of(shortcut)).then_some((id, clean))
            })
            .collect(),
    )
}

/// The overrides saved on this Mac. Missing or unreadable: none.
pub fn load() -> Overrides {
    if !crate::window_frame::kept() {
        return Overrides::new();
    }
    let Some(bytes) = path().and_then(|path| std::fs::read(path).ok()) else {
        return Overrides::new();
    };
    match parse(&bytes) {
        Some(overrides) => {
            if !overrides.is_empty() {
                eprintln!("nimbra: {} changed shortcuts", overrides.len());
            }
            overrides
        }
        None => {
            eprintln!("nimbra: the shortcuts file could not be read, using the defaults");
            Overrides::new()
        }
    }
}

/// One save at a time, so the last change is the one on disk.
static SAVING: Mutex<()> = Mutex::new(());

/// Saves the overrides in effect. Blocking: call it off the main thread.
pub fn save() {
    if !crate::window_frame::kept() {
        return;
    }
    let Some(path) = path() else {
        return;
    };
    let _one = SAVING.lock();
    let overrides = current();
    let partial = path.with_extension("json.partial");
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| serde_json::to_vec_pretty(&overrides).map_err(std::io::Error::other))
        .and_then(|json| std::fs::write(&partial, json))
        .and_then(|()| std::fs::rename(&partial, &path));
    if written.is_err() {
        eprintln!("nimbra: could not save the shortcuts");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bindings `main.rs` had before the table, for the actions now in it:
    /// (key, context, action).
    const BEFORE: &[(&str, Option<&str>, &str)] = &[
        // Once fixed in main.rs, now in the table with the same keys and contexts.
        ("enter", M, "Confirm"),
        ("cmd-enter", Some("Composer"), "SendMail"),
        ("ctrl-enter", Some("Composer"), "SendMail"),
        ("cmd-s", Some("Composer"), "SaveDraft"),
        ("ctrl-s", Some("Composer"), "SaveDraft"),
        ("cmd-shift-f", Some("Composer"), "ToggleFullCompose"),
        ("ctrl-shift-f", Some("Composer"), "ToggleFullCompose"),
        ("cmd-z", Some("Mail"), "Undo"),
        ("ctrl-z", Some("Mail"), "Undo"),
        ("cmd-,", None, "OpenSettings"),
        ("cmd-p", None, "Print"),
        ("cmd-shift-p", None, "PrintMessage"),
        ("cmd-w", None, "HideWindow"),
        ("cmd-h", None, "HideApp"),
        ("cmd-m", None, "Minimize"),
        // The editor's formatting: the page runs it, the app leaves it alone.
        ("cmd-b", Some("Editor"), "zed::NoAction"),
        ("ctrl-b", Some("Editor"), "zed::NoAction"),
        ("cmd-i", Some("Editor"), "zed::NoAction"),
        ("ctrl-i", Some("Editor"), "zed::NoAction"),
        ("cmd-u", Some("Editor"), "zed::NoAction"),
        ("ctrl-u", Some("Editor"), "zed::NoAction"),
        ("cmd-k", Some("Editor"), "zed::NoAction"),
        ("ctrl-k", Some("Editor"), "zed::NoAction"),
        ("/", M, "FocusSearch"),
        ("cmd-f", Some("Mail"), "FocusSearch"),
        ("ctrl-f", Some("Mail"), "FocusSearch"),
        ("c", M, "Compose"),
        ("r", M, "Reply"),
        ("a", M, "ReplyAll"),
        ("f", M, "ForwardMessage"),
        ("j", M, "Next"),
        ("down", M, "Next"),
        ("k", M, "Prev"),
        ("up", M, "Prev"),
        ("e", M, "MarkDone"),
        ("z", M, "Undo"),
        ("s", M, "ToggleStar"),
        ("u", M, "MarkUnread"),
        ("v", M, "ToggleOriginal"),
        ("cmd-backspace", M, "Trash"),
        ("backspace", M, "Trash"),
        ("delete", M, "Trash"),
        ("ctrl-backspace", M, "Trash"),
        ("cmd-k", None, "TogglePalette"),
        ("ctrl-k", None, "TogglePalette"),
        ("h", M, "TogglePalette"),
        ("cmd-shift-l", None, "ToggleTheme"),
        ("ctrl-shift-l", None, "ToggleTheme"),
        ("cmd-b", None, "ToggleSidebar"),
        ("cmd-shift-b", None, "ToggleList"),
        ("ctrl-shift-b", None, "ToggleList"),
        ("ctrl-b", None, "ToggleSidebar"),
        ("cmd-[", None, "Back"),
        ("cmd-]", None, "Forward"),
        ("cmd-r", None, "Reload"),
        ("ctrl-r", None, "Reload"),
        ("cmd-n", None, "Compose"),
        ("left", M, "FocusSidebar"),
        ("home", M, "First"),
        ("end", M, "Last"),
        ("cmd-up", M, "First"),
        ("cmd-down", M, "Last"),
        ("x", M, "PickThread"),
        ("shift-down", M, "PickDown"),
        ("shift-j", M, "PickDown"),
        ("shift-up", M, "PickUp"),
        ("shift-k", M, "PickUp"),
        ("o", M, "OpenInWindow"),
        ("space", M, "PageDown"),
        ("shift-space", M, "PageUp"),
        ("?", M, "ToggleShortcuts"),
        ("shift-/", M, "ToggleShortcuts"),
        ("g i", M, "GoInbox"),
        ("g s", M, "GoStarred"),
        ("g b", M, "GoSnoozed"),
        ("g t", M, "GoSent"),
        ("g d", M, "GoDrafts"),
        ("g a", M, "GoAllMail"),
        ("cmd-a", M, "SelectAllThreads"),
        ("ctrl-a", M, "SelectAllThreads"),
    ];

    fn spelled(key: &str) -> Vec<String> {
        KeyBinding::new(key, gpui::NoAction, None)
            .keystrokes()
            .iter()
            .map(|k| k.unparse())
            .collect()
    }

    #[test]
    fn the_defaults_bind_what_main_did() {
        let mut before: Vec<_> = BEFORE
            .iter()
            .map(|(key, context, action)| {
                (
                    spelled(key),
                    context.map(str::to_string),
                    if action.contains("::") {
                        action.to_string()
                    } else {
                        format!("nimbra::{action}")
                    },
                )
            })
            .collect();
        let mut now: Vec<_> = TABLE
            .iter()
            .flat_map(|s| {
                s.defaults.iter().map(move |key| {
                    (
                        spelled(key),
                        context_for(s, key).map(str::to_string),
                        (s.action)().name().to_string(),
                    )
                })
            })
            .collect();
        before.sort();
        now.sort();
        assert_eq!(before, now);
        // The bindings themselves, built as at launch.
        let bound: Vec<_> = bindings(&Overrides::new())
            .iter()
            .map(|b| {
                (
                    b.keystrokes()
                        .iter()
                        .map(|k| k.unparse())
                        .collect::<Vec<_>>(),
                    b.action().name().to_string(),
                )
            })
            .collect();
        let mut bound = bound;
        bound.sort();
        let mut expected: Vec<_> = before.into_iter().map(|(k, _, a)| (k, a)).collect();
        // The composer's keys also leave the editor alone, whose page reports them itself.
        for key in [
            "cmd-enter",
            "ctrl-enter",
            "cmd-s",
            "ctrl-s",
            "cmd-shift-f",
            "ctrl-shift-f",
        ] {
            expected.push((spelled(key), "zed::NoAction".to_string()));
        }
        expected.sort();
        assert_eq!(bound, expected);
    }

    #[test]
    fn ids_are_unique_and_every_default_is_a_key() {
        let mut ids: Vec<_> = TABLE.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TABLE.len());
        for s in TABLE {
            assert_eq!(defaults_of(s).len(), s.defaults.len(), "{}", s.id);
        }
    }

    #[test]
    fn keys_read_as_on_the_mac() {
        // The Mac's order: ⌃⌥⇧⌘.
        assert_eq!(glyph("cmd-shift-b"), "⇧⌘B");
        assert_eq!(glyph("cmd-f"), "⌘F");
        assert_eq!(glyph("ctrl-alt-shift-cmd-x"), "⌃⌥⇧⌘X");
        assert_eq!(glyph("cmd-enter"), "⌘↵");
        assert_eq!(glyph("backspace"), "⌫");
        assert_eq!(glyph("cmd-backspace"), "⌘⌫");
        assert_eq!(glyph("up"), "↑");
        assert_eq!(glyph("shift-down"), "⇧↓");
        assert_eq!(glyph("e"), "E");
        assert_eq!(glyph("shift-j"), "⇧J");
        assert_eq!(glyph("g i"), "G I");
        assert_eq!(glyph("?"), "?");
        assert_eq!(glyph("shift-/"), "?");
        assert_eq!(glyph("/"), "/");
        assert_eq!(glyph("cmd-["), "⌘[");
        assert_eq!(glyph("space"), "Space");
        assert_eq!(glyph("shift-space"), "⇧Space");
        assert_eq!(glyph("home"), "Home");
        assert_eq!(glyph("f5"), "F5");
        assert_eq!(glyph("escape"), "esc");
    }

    #[test]
    fn short_lists_leave_out_the_control_twins_and_doubles() {
        let keys = |list: &[&str]| list.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        assert_eq!(compact(&keys(&["h", "cmd-k", "ctrl-k"])), ["H", "⌘K"]);
        assert_eq!(compact(&keys(&["?", "shift-/"])), ["?"]);
        // A ⌃ key with no ⌘ twin stays.
        assert_eq!(compact(&keys(&["ctrl-y"])), ["⌃Y"]);
        assert_eq!(
            caps(&keys(&["?", "shift-/", "x"])),
            [
                ("?".to_string(), keys(&["?", "shift-/"])),
                ("X".to_string(), keys(&["x"]))
            ]
        );
    }

    #[test]
    fn keys_have_one_spelling() {
        assert_eq!(normalize("shift-cmd-b").as_deref(), Some("cmd-shift-b"));
        assert_eq!(
            normalize("alt-shift-cmd-i").as_deref(),
            Some("alt-cmd-shift-i")
        );
        assert_eq!(normalize("J").as_deref(), Some("shift-j"));
        assert_eq!(normalize("g  i").as_deref(), Some("g i"));
        assert_eq!(normalize("cmd--").as_deref(), Some("cmd--"));
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("shift"), None);
        assert_eq!(normalize("a b c"), None);
        assert_eq!(normalize("ctrl-nope-x"), None);
    }

    #[test]
    fn a_bad_file_leaves_the_defaults() {
        assert_eq!(parse(b"not json"), None);
        assert_eq!(parse(b"[1, 2]"), None);
        assert_eq!(parse(br#"{"mark-done": "e"}"#), None);
        // Unknown actions and keys that are no keys go; the rest stays.
        let parsed =
            parse(br#"{"mark-done": ["d", "shift", "D", "d"], "nope": ["x"], "reply": ["r"]}"#)
                .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed["mark-done"], ["d", "shift-d"]);
        // No keys at all is a choice, kept.
        let none = parse(br#"{"undo": []}"#).unwrap();
        assert_eq!(keys_in(&none, find("undo").unwrap()), Vec::<String>::new());
        assert_eq!(parse(b"{}"), Some(Overrides::new()));
    }

    #[test]
    fn replacing_moves_the_key() {
        let none = Overrides::new();
        let fixed = vec![("cmd-q".to_string(), "Quit")];
        assert_eq!(
            taken(&none, "reply", "e", &fixed),
            Taken::Used(vec!["mark-done"])
        );
        assert_eq!(taken(&none, "reply", "cmd-q", &fixed), Taken::Fixed("Quit"));
        assert_eq!(taken(&none, "reply", "r", &fixed), Taken::Own);
        assert_eq!(taken(&none, "reply", "y", &fixed), Taken::Free);
        // G starts every Go to.
        assert!(matches!(taken(&none, "reply", "g", &fixed), Taken::Used(ids) if ids.len() == 6));

        // Replacing: Reply takes E alone, and Mark done is left with no key.
        let moved = replace(&none, "reply", "e");
        let reply = find("reply").unwrap();
        let done = find("mark-done").unwrap();
        assert_eq!(keys_in(&moved, reply), ["e"]);
        assert_eq!(keys_in(&moved, done), Vec::<String>::new());
        assert_eq!(losers(&none, "reply", &["e".into()]), ["mark-done"]);
        // Reset takes Reply's R back, and gives E back to nobody: Mark done says so first.
        assert_eq!(
            losers(&moved, "mark-done", &defaults("mark-done")),
            ["reply"]
        );
        let back = reset_taking(&moved, "mark-done");
        assert_eq!(keys_in(&back, done), ["e"]);
        assert_eq!(keys_in(&back, reply), Vec::<String>::new());
        assert!(changed(&back, "reply") && !changed(&back, "mark-done"));
        // No key at all, and every one back.
        let none_left = clear(&none, "snooze");
        assert!(keys_in(&none_left, find("snooze").unwrap()).is_empty());
        assert!(reset_taking(&none_left, "snooze").is_empty());
        // A two-step key clashes with its first step: G takes every Go to's key.
        assert_eq!(losers(&none, "reply", &["g".into()]).len(), 6);
    }

    #[test]
    fn out_of_the_box_no_key_does_two_things() {
        let none = Overrides::new();
        for shortcut in TABLE {
            let keys = keys_in(&none, shortcut);
            assert!(
                losers(&none, shortcut.id, &keys).is_empty(),
                "{}",
                shortcut.id
            );
        }
        // ⌘K is Snooze, and Link in the editor; they never take it from each other.
        assert!(losers(&none, "link", &["cmd-k".into()]).is_empty());
        assert_eq!(losers(&none, "bold", &["cmd-s".into()]), ["save-draft"]);
    }

    #[test]
    fn writing_keys_need_a_modifier_and_stay_in_the_editor() {
        let bold = find("bold").unwrap();
        let send = find("send").unwrap();
        assert!(fits(bold, "cmd-shift-b") && fits(send, "ctrl-enter"));
        // A plain key would type nothing else, and the page sees one key at a time.
        assert!(!fits(bold, "b") && !fits(send, "g s") && !fits(send, "shift-s"));
        assert!(fits(find("mark-done").unwrap(), "g e"));
        assert_eq!(context_for(bold, "cmd-b"), Some(EDITOR));
        assert_eq!(context_for(send, "cmd-enter"), Some(COMPOSER));
        // The page gets the keys in effect.
        let keys = editor_keys();
        assert_eq!(keys["bold"], serde_json::json!(["cmd-b", "ctrl-b"]));
        assert_eq!(keys["send"], serde_json::json!(["cmd-enter", "ctrl-enter"]));
    }

    #[test]
    fn a_chord_takes_the_actions_context_and_a_letter_the_mail_views() {
        let compose = find("compose").unwrap();
        assert_eq!(context_for(compose, "cmd-shift-n"), None);
        assert_eq!(context_for(compose, "n"), Some(MAIL));
        assert_eq!(context_for(compose, "alt-n"), Some(MAIL));
        let search = find("focus-search").unwrap();
        assert_eq!(context_for(search, "ctrl-s"), Some("Mail"));
    }
}
