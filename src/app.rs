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
        Back,
        Forward,
        Reload,
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
    to: Entity<TextInput>,
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
            RowAction::OpenInGmail => ("link", "Open in Gmail"),
        }
    }
}

/// A small menu dropped from the reading toolbar.
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Labels,
    More,
    /// The right-click menu of a row in the list (by index into the loaded threads).
    Row(usize),
}

/// What the mouse is doing to the floating composer.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Move,
    Resize,
}

const COMPOSER_MIN: (f32, f32) = (460., 320.);

impl Composer {
    /// The whole message text: the edited lines, then the quoted earlier message.
    fn body(&self, cx: &App) -> String {
        let mut body = self
            .lines
            .iter()
            .map(|line| line.read(cx).text())
            .collect::<Vec<_>>()
            .join("\n");
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

/// Which composer field an event came from.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    To,
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
            // NIMBRA_MAILBOX=sent (or starred, snoozed, drafts, done) opens that mailbox first.
            mailbox: match std::env::var("NIMBRA_MAILBOX").as_deref() {
                Ok("starred") => View::Starred,
                Ok("snoozed") => View::Snoozed,
                Ok("drafts") => View::Drafts,
                Ok("sent") => View::Sent,
                Ok("done") => View::Done,
                _ => View::Inbox,
            },
            cache: HashMap::new(),
            prefetching: false,
            open_when_listed: None,
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
        this.original = std::env::var_os("NIMBRA_READER").is_none();
        // NIMBRA_SCREEN=draft opens a sample draft in the composer (screenshots).
        if screen == "draft" {
            this.edit_draft(
                gmail::Draft {
                    id: "sample".into(),
                    to: "mira@example.com".into(),
                    subject: "Fwd: Billing problem".into(),
                    body: "Hi Mira,\n\nCan you take a look at this one?\n\nLuan Gjokaj\nUX Engineer & Designer".into(),
                    quoted: Some("---------- Forwarded message ---------\nFrom: Stripe\nSubject: Billing problem\n\nYour last payment could not be processed.".into()),
                    reply: None,
                },
                cx,
            );
            if let Some(composer) = &mut this.compose {
                composer.show_quoted = true;
            }
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
        if let Some(config) = this.config.clone() {
            this.status = Status::Loading;
            cx.spawn(async move |this, cx| {
                let token = cx
                    .background_executor()
                    .spawn(async { gmail::stored_refresh_token() })
                    .await;
                this.update(cx, |this, cx| match token {
                    Some(token) => {
                        this.gmail = Some(Arc::new(gmail::Client::new(config, token)));
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
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let store = crate::store::Store::open()?;
                    let saved: Option<Saved> = store.load("meta");
                    let mails: Option<Vec<Mail>> = store.load(&record(view));
                    anyhow::Ok((store, saved, mails))
                })
                .await;
            let Ok((store, saved, mails)) = opened else {
                eprintln!("nimbra: mail store unavailable; continuing without it");
                this.update(cx, |this, cx| this.fetch_inbox(cx)).ok();
                return;
            };
            this.update(cx, |this, cx| {
                this.store = Some(Arc::new(store));
                // Something is already on screen (a sign-in in this session): just refresh.
                if !this.mails.is_empty() || this.mailbox != view {
                    this.refresh(cx);
                    return;
                }
                if let Some(saved) = saved {
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
        let mails: Vec<Mail> = self
            .mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
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
                let Ok(client) = this.update(cx, |this, _| this.gmail.clone()) else {
                    break; // the window is gone
                };
                let Some(client) = client else {
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
                this.update(cx, |_, _| {
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
        match gmail::SignIn::start(&config) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.status = Status::SigningIn;
                self.notice = None;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config) })
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
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // Read the history point first, so nothing that changes during the
                    // fetch is missed by the next sync.
                    let (email, history_id) = client.profile()?;
                    let threads = client.threads(view, label_id.as_deref(), 30)?;
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
                    anyhow::Ok((email, history_id, threads, labels, counts))
                })
                .await;
            this.update(cx, |this, cx| {
                // Ignore the answer if the user has moved to another mailbox or search meanwhile.
                if this.mailbox != view || (view == View::Search && this.query != sent_query) {
                    return;
                }
                this.fetching = false;
                match result {
                    Ok((email, history_id, threads, labels, counts)) => {
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
        self.account = email.into();

        let fresh = threads.into_iter().map(Mail::from_summary).collect();
        self.install(fresh, cx);
    }

    /// Puts a new listing on screen, keeping downloaded thread contents, the open thread and
    /// threads cleared in the meantime, then saves it.
    fn install(&mut self, fresh: Vec<Mail>, cx: &mut Context<Self>) {
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
        cx.spawn(async move |this, cx| {
            let thread_id = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { client.thread(&thread_id) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(messages) => {
                    // The inbox may have been reloaded meanwhile, so look the thread up again.
                    if let Some(mail) = this.mails.iter_mut().find(|m| m.id.as_deref() == Some(&id))
                    {
                        mail.fill(messages);
                        this.persist(cx);
                        cx.notify();
                    }
                }
                Err(error) => this.fail(error, cx),
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
                this.field_event(field, event, cx);
            }
        });
        (input, subscription)
    }

    /// Opens the composer, empty or as a reply to the open thread.
    fn open_compose(&mut self, kind: ComposeKind, cx: &mut Context<Self>) {
        self.menu = None;
        if self.compose.is_some() || self.palette {
            return;
        }
        let (to, s1) = self.new_field(Field::To, "name@example.com", cx);
        let (subject, s2) = self.new_field(Field::Subject, "Subject", cx);
        let (line, s3) = self.new_field(Field::Line(0), "Write your message…", cx);
        let mut composer = Composer {
            to,
            subject,
            lines: vec![line],
            reply: None,
            full: false,
            position: None,
            size: (720., 440.),
            grab: None,
            draft_id: None,
            quoted: None,
            show_quoted: false,
            saving: false,
            saved: false,
            sending: false,
            error: None,
            _subscriptions: vec![s1, s2, s3],
        };
        let mut focus = composer.to.read(cx).focus_handle(cx);
        if kind != ComposeKind::New
            && let Some(mail) = self
                .mails
                .get(self.selected)
                .filter(|_| !self.visible().is_empty())
                .cloned()
        {
            let prefix = if kind == ComposeKind::Forward {
                "Fwd: "
            } else {
                "Re: "
            };
            let subject = if mail
                .subject
                .to_lowercase()
                .starts_with(&prefix.trim().to_lowercase())
            {
                mail.subject.to_string()
            } else {
                format!("{prefix}{}", mail.subject)
            };
            composer
                .subject
                .update(cx, |input, cx| input.set(subject, usize::MAX, cx));
            if kind == ComposeKind::Forward {
                // The forwarded message goes in as plain text under a divider.
                let mut quoted = vec![
                    String::new(),
                    "---------- Forwarded message ----------".to_string(),
                    format!("From: {}", mail.sender),
                    format!("Subject: {}", mail.subject),
                    String::new(),
                ];
                for block in &mail.body {
                    quoted.extend(block.text.lines().map(str::to_string));
                    quoted.push(String::new());
                }
                for text in quoted.into_iter().take(400) {
                    let index = composer.lines.len();
                    let (line, subscription) = self.new_field(Field::Line(index), "", cx);
                    line.update(cx, |input, cx| input.set(text, 0, cx));
                    composer.lines.push(line);
                    composer._subscriptions.push(subscription);
                }
            } else if let Some((address, reply_to)) = mail.reply.clone() {
                let mut to = vec![address];
                if kind == ComposeKind::ReplyAll {
                    for other in &mail.reply_all {
                        let me = other.eq_ignore_ascii_case(&self.account);
                        if !me && !to.iter().any(|known| known.eq_ignore_ascii_case(other)) {
                            to.push(other.clone());
                        }
                    }
                }
                composer
                    .to
                    .update(cx, |input, cx| input.set(to.join(", "), usize::MAX, cx));
                composer.reply = Some(reply_to);
                focus = composer.lines[0].read(cx).focus_handle(cx);
            }
        }
        self.pending_focus = Some(focus);
        self.compose = Some(composer);
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
        let covered = self.palette || self.compose.is_some() || self.notice.is_some();
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
            self.original_images = true;
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
            Menu::Labels if self.labels.is_empty() => vec![(
                "tag",
                if self.is_demo() {
                    "Labels need a connected Gmail account.".into()
                } else {
                    "You have no labels in Gmail yet.".into()
                },
                t.text3,
            )],
            Menu::Labels => self
                .labels
                .iter()
                .enumerate()
                .take(12)
                .map(|(index, label)| ("tag", short(&label.name, 28), tints[index % tints.len()]))
                .collect(),
            Menu::More if self.is_demo() => vec![("inbox", "Mark as unread".into(), t.text2)],
            Menu::More => vec![
                ("inbox", "Mark as unread".into(), t.text2),
                ("link", "Open in Gmail".into(), t.text2),
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
        let enabled = !(menu == Menu::Labels && self.labels.is_empty());

        // Where the menu goes, in window coordinates: under its button.
        let viewport = window.viewport_size();
        let pane_left = if self.sidebar_open { 236. } else { 0. }
            + if self.list_open { self.list_width } else { 0. };
        let (width, height) = (MENU_WIDTH, items.len() as f32 * 32. + 14.);
        let (left, top) = match menu {
            Menu::Labels => (pane_left + 150., 50.),
            Menu::More => (f32::from(viewport.width) - 20. - width, 50.),
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
            (Menu::Labels, index) => self.apply_label(index, cx),
            (Menu::More, 0) => self.mark_unread(cx),
            (Menu::More, _) => self.open_in_gmail(cx),
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

    /// Adds one of the user's labels to the open thread.
    fn apply_label(&mut self, label: usize, cx: &mut Context<Self>) {
        self.menu = None;
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
            RowAction::Done | RowAction::MoveToInbox | RowAction::Trash => {
                self.selected = row;
                if action == RowAction::Trash {
                    self.trash(cx);
                } else {
                    self.mark_done(cx);
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
        composer.draft_id = Some(draft.id);
        composer.quoted = draft.quoted;
        composer.reply = draft.reply;
        composer.full = true;
        self.pending_focus = Some(composer.lines[0].read(cx).focus_handle(cx));
        self.compose = Some(composer);
        cx.notify();
    }

    /// Saves the composer's contents to Gmail's Drafts without sending anything.
    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.saving || composer.sending {
            return;
        }
        let Some(client) = self.gmail.clone() else {
            composer.error = Some("Sample mode: drafts are not saved.".into());
            cx.notify();
            return;
        };
        let outgoing = gmail::Outgoing {
            to: composer.to.read(cx).text(),
            subject: composer.subject.read(cx).text(),
            body: composer.body(cx),
            reply: composer.reply.clone(),
        };
        let draft_id = composer.draft_id.clone();
        composer.saving = true;
        composer.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.save_draft(draft_id.as_deref(), &outgoing) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(id) => {
                        if let Some(composer) = &mut this.compose {
                            composer.saving = false;
                            composer.draft_id = Some(id);
                            composer.saved = true;
                        }
                        if this.mailbox == View::Drafts {
                            this.fetch_inbox(cx);
                        }
                    }
                    Err(error) if error.is::<gmail::SignedOut>() => {
                        this.compose = None;
                        this.fail(error, cx);
                    }
                    Err(error) => {
                        if let Some(composer) = &mut this.compose {
                            composer.saving = false;
                            composer.error = Some(error.to_string().into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn close_compose(&mut self, cx: &mut Context<Self>) {
        self.compose = None;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    fn focus_field(&mut self, field: Field, cursor: Option<usize>, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        let input = match field {
            Field::To => composer.to.clone(),
            Field::Subject => composer.subject.clone(),
            Field::Line(index) => match composer.lines.get(index) {
                Some(line) => line.clone(),
                None => return,
            },
        };
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
        match (field, event) {
            (Field::To, InputEvent::Enter | InputEvent::Tab | InputEvent::Down) => {
                self.focus_field(Field::Subject, None, cx)
            }
            (Field::Subject, InputEvent::Enter | InputEvent::Tab | InputEvent::Down) => {
                self.focus_field(Field::Line(0), None, cx)
            }
            (Field::Subject, InputEvent::Up) => self.focus_field(Field::To, None, cx),
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

    /// Sends what was written. Only ever runs from the Send button or its shortcut.
    fn send_mail(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.sending {
            return;
        }
        let outgoing = gmail::Outgoing {
            to: composer.to.read(cx).text(),
            subject: composer.subject.read(cx).text(),
            body: composer.body(cx),
            reply: composer.reply.clone(),
        };
        let draft_id = composer.draft_id.clone();
        if let Err(error) = outgoing.recipients() {
            composer.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        if outgoing.subject.trim().is_empty() && outgoing.body.trim().is_empty() {
            composer.error = Some("Write a subject or a message first.".into());
            cx.notify();
            return;
        }
        let Some(client) = self.gmail.clone() else {
            // Sample data: nothing leaves this computer.
            self.close_compose(cx);
            self.notice = Some("Sample mode: the message was not sent anywhere.".into());
            return;
        };
        composer.sending = true;
        composer.error = None;
        cx.notify();
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
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.close_compose(cx);
                    this.notice = Some("Message sent.".into());
                    if matches!(this.mailbox, View::Sent | View::Drafts) {
                        this.refresh(cx);
                    }
                }
                Err(error) if error.is::<gmail::SignedOut>() => {
                    this.compose = None;
                    this.fail(error, cx);
                }
                Err(error) => {
                    if let Some(composer) = &mut this.compose {
                        composer.sending = false;
                        composer.error = Some(error.to_string().into());
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
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

    /// Runs the highlighted palette command. Snoozing is only simulated on the sample data;
    /// on a real mailbox it must not archive anything, so it just says so.
    fn run_command(&mut self, cx: &mut Context<Self>) {
        self.palette = false;
        if self.is_demo() {
            self.mark_done(cx);
        } else {
            self.notice = Some("Snooze isn’t connected to Gmail yet. Nothing was changed.".into());
            cx.notify();
        }
    }

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        eprintln!("nimbra: {error}");
        if error.is::<gmail::SignedOut>() {
            self.gmail = None;
            // The stored mail belongs to that sign-in: remove it, and its key.
            self.store = None;
            std::thread::spawn(crate::store::wipe);
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
            let max = COMMANDS.len() as isize - 1;
            self.palette_sel = (self.palette_sel as isize + delta).clamp(0, max) as usize;
        } else {
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
        let visible = self.visible();
        let Some(pos) = visible.iter().position(|i| *i == self.selected) else {
            return;
        };
        let index = self.selected;
        self.done[index] = true;
        self.history.push(index);
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
                gmail::Change::Unarchive => "Moved to inbox",
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
        let Some(index) = self.history.pop() else {
            return;
        };
        self.done[index] = false;
        self.selected = index;
        let change = self.mails[index]
            .id
            .as_ref()
            .and_then(|id| self.removed_by.remove(id))
            .unwrap_or(gmail::Change::Archive);
        if let Some(inverse) = change.inverse() {
            self.sync(index, inverse, false, cx);
        }
        self.load_selected(cx);
        cx.notify();
    }

    fn toggle_star(&mut self, cx: &mut Context<Self>) {
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
                                if demo { "Work · 3 accounts" } else { "Gmail" },
                                t.text3,
                            )),
                    )
                    .child(
                        icon_button(t, if self.dark { "sun" } else { "moon" })
                            .id("theme")
                            .tooltip(tip(t.dark, "Light or dark theme", "⌘⇧L"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dark = !this.dark;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn row(&self, t: &Theme, i: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let m = &self.mails[i];
        let selected = i == self.selected;
        let hover = t.hover;
        // How many characters fit on a row at the list's current width (the text is cut
        // with an ellipsis by character count, so this follows the resize handle).
        let line = ((self.list_width - 116.) / 7.4).max(8.) as usize;
        // On a narrow list the tag gives way so the sender's name stays readable.
        let tag = m.tag.clone().filter(|_| line >= 36);
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
            .when(selected, |d| d.bg(t.selected))
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.open_row_menu(i, event.position, window, cx)
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open(i);
                this.open_selected(cx);
                cx.notify();
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
                    .when(self.fetching && !self.pull_hold && self.pull == 0., |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_none()
                                .justify_center()
                                .py(px(8.))
                                .child(meta("Checking for mail…", t.text3)),
                        )
                    })
                    .when(!today.is_empty(), |d| d.child(group("TODAY")))
                    .children(today.into_iter().map(|i| self.row(t, i, cx)))
                    .when(!earlier.is_empty(), |d| d.child(group("EARLIER")))
                    .children(earlier.into_iter().map(|i| self.row(t, i, cx)))
                    .child(div().flex().justify_center().pt(px(18.)).child(meta(
                        match (self.is_demo(), self.mailbox) {
                            (true, View::Inbox) => "That’s everything important. 23 more in Other.",
                            (false, View::Inbox) => "That’s everything in your inbox.",
                            _ => "That’s the latest here.",
                        },
                        t.faint,
                    ))),
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
                                                        self.message_body(
                                                            t,
                                                            &e.blocks,
                                                            (k + 1) * 1000,
                                                            cx,
                                                        )
                                                        .pl(px(36.))
                                                        .pb(px(14.)),
                                                    )
                                                })
                                        },
                                    )))
                                })
                                .child(divider())
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(12.))
                                        .child(avatar(m.tone, &m.initials, 28.))
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child(m.sender.clone()),
                                                )
                                                .child(meta(
                                                    if m.rich { "to you, Jonah" } else { "to you" },
                                                    t.text3,
                                                )),
                                        )
                                        .child(mono(m.time.clone(), t.text3)),
                                )
                                .child(self.message_body(t, &m.body, 0, cx).pl(px(40.)).when(
                                    !m.loaded,
                                    |d| {
                                        d.child(div().text_color(t.text3).child("Loading message…"))
                                    },
                                ))
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
            .when(original, |d| d.child(self.original_pane(t, cx)))
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

    /// The area the web view covers, with a strip above it explaining what is blocked.
    fn original_pane(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.html_view.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
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
            // Row menus only exist as pop-up windows.
            Menu::Row(_) => return None,
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
        let field = |label: &str, input: &Entity<TextInput>| {
            div()
                .flex()
                .justify_center()
                .border_b_1()
                .border_color(t.border)
                .child(
                    column(div())
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
                                .text_size(px(14.))
                                .child(input.clone()),
                        ),
                )
        };
        let card = div()
            .flex()
            .flex_col()
            .bg(t.raised)
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
            .child(field("To", &composer.to))
            .child(field("Subject", &composer.subject))
            .child(
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
                                                    composer.show_quoted = !composer.show_quoted;
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                                )
                                .when(composer.show_quoted, |d| {
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
                                })
                            }),
                    ),
            )
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
                                div()
                                    .flex_1()
                                    .pl(px(6.))
                                    .text_color(if composer.error.is_some() {
                                        t.rose
                                    } else {
                                        t.text3
                                    })
                                    .child(match &composer.error {
                                        Some(error) => error.clone(),
                                        None if composer.sending => "Sending…".into(),
                                        None if composer.saving => "Saving draft…".into(),
                                        None if composer.saved => "Draft saved to Gmail.".into(),
                                        None => {
                                            "Sent as plain text from your Gmail address.".into()
                                        }
                                    }),
                            )
                            .child(
                                button(
                                    t,
                                    ButtonKind::Ghost,
                                    if composer.draft_id.is_some() {
                                        "Close"
                                    } else {
                                        "Discard"
                                    },
                                    None,
                                )
                                .id("discard")
                                .on_click(cx.listener(|this, _, _, cx| this.close_compose(cx))),
                            )
                            .child(
                                button(t, ButtonKind::Secondary, "Save draft", Some("⌘S"))
                                    .id("save-draft")
                                    .when(composer.saving, |d| d.opacity(0.5))
                                    .on_click(cx.listener(|this, _, _, cx| this.save_draft(cx))),
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
        let items =
            COMMANDS
                .iter()
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
                        .on_click(cx.listener(|this, _, _, cx| this.run_command(cx)))
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
                                .child(*label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_color(t.text3)
                                .child(*hint),
                        )
                        .child(kbd(t, key))
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
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                if this.menu.take().is_some() {
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
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| {
                this.dark = !this.dark;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &Forward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
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
                Status::Demo | Status::Ready => d
                    .children(self.list_slot(&t, cx))
                    .child(self.reading(&t, cx)),
                Status::Loading => d.child(self.skeleton(&t)),
                _ => d.child(self.status_screen(&t, cx)),
            })
            .child(self.window_controls(&t, cx))
            .when(self.palette && !empty, |d| d.child(self.palette(&t, cx)))
            .when(self.compose.is_some(), |d| {
                d.child(self.compose_view(&t, viewport, cx))
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
