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
    ButtonKind, avatar, button, icon, icon_button, kbd, meta, mono, overline, pill, shadow,
};

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
        ToggleStar,
        Trash,
        Compose,
        Reply,
        SendMail,
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
    sending: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<gpui::Subscription>,
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
    /// Memory only: nothing is written to disk.
    cache: HashMap<View, Vec<Mail>>,
    /// The user's own Gmail labels, shown in the sidebar.
    labels: Vec<gmail::Label>,
    /// A mailbox request is in flight.
    fetching: bool,
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
        let mut this = Self {
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
            labels: Vec::new(),
            fetching: false,
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
            sidebar_open: std::env::var("NIMBRA_SIDEBAR").as_deref() != Ok("closed"),
            sidebar_gen: 0,
            reload_gen: 0,
            images: HashMap::new(),
            image_requests: HashSet::new(),
            back: Vec::new(),
            forward: Vec::new(),
        };
        // NIMBRA_SCREEN=compose opens the composer on the sample data (screenshots).
        if screen == "compose" {
            this.open_compose(true, cx);
        }
        // NIMBRA_SCREEN=loading holds the loading skeleton on screen (screenshots).
        if screen == "loading" {
            this.status = Status::Loading;
            this.fetching = true;
        }
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
                        this.fetch_inbox(cx);
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
            View::Label(index) => self
                .labels
                .get(index)
                .map(|label| SharedString::from(label.name.clone()))
                .unwrap_or_else(|| "Label".into()),
        }
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
        self.cache.insert(self.mailbox, kept);
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
            self.mails = self.cache.get(&view).cloned().unwrap_or_default();
            self.done = vec![false; self.mails.len()];
            self.history.clear();
            self.selected = 0;
            if !self.mails.is_empty() {
                self.status = Status::Ready;
                self.load_selected(cx);
            }
            self.fetch_inbox(cx);
        }
    }

    // ------------------------------------------------------------ gmail

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
                            this.fetch_inbox(cx);
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
            _ => None,
        };
        let need_labels = self.labels.is_empty();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let email = client.email_address()?;
                    let threads = client.threads(view, label_id.as_deref(), 30)?;
                    // Labels are a nicety; the mailbox still loads without them.
                    let labels = need_labels.then(|| client.labels().unwrap_or_default());
                    anyhow::Ok((email, threads, labels))
                })
                .await;
            this.update(cx, |this, cx| {
                // Ignore the answer if the user has moved to another mailbox meanwhile.
                if this.mailbox != view {
                    return;
                }
                this.fetching = false;
                match result {
                    Ok((email, threads, labels)) => {
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
        self.me_initials = email
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();
        self.account = email.into();

        let open_id = self.mails.get(self.selected).and_then(|m| m.id.clone());
        // Threads cleared while this request was in flight must stay cleared.
        let cleared: Vec<Option<String>> = self
            .history
            .iter()
            .filter_map(|i| self.mails.get(*i).map(|m| m.id.clone()))
            .collect();

        let fresh = threads.into_iter().map(Mail::from_summary).collect();
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
        // Counts only; never log addresses, subjects or message text.
        eprintln!(
            "nimbra: {:?} loaded ({} threads, {} already in memory)",
            self.mailbox,
            self.mails.len(),
            self.mails.iter().filter(|m| m.loaded).count()
        );
        self.load_selected(cx);
        cx.notify();
    }

    /// Fetches the messages of the open thread the first time it is shown.
    fn load_selected(&mut self, cx: &mut Context<Self>) {
        // Opening a thread marks it read, here and in Gmail.
        let index = self.selected;
        if self.visible().contains(&index) && self.mails[index].unread {
            self.mails[index].unread = false;
            self.sync(index, gmail::Change::MarkRead, false, cx);
        }
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
    fn open_compose(&mut self, reply: bool, cx: &mut Context<Self>) {
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
            sending: false,
            error: None,
            _subscriptions: vec![s1, s2, s3],
        };
        let mut focus = composer.to.read(cx).focus_handle(cx);
        if reply
            && let Some(mail) = self
                .mails
                .get(self.selected)
                .filter(|_| !self.visible().is_empty())
        {
            let subject = if mail.subject.to_lowercase().starts_with("re:") {
                mail.subject.to_string()
            } else {
                format!("Re: {}", mail.subject)
            };
            composer
                .subject
                .update(cx, |input, cx| input.set(subject, usize::MAX, cx));
            if let Some((address, reply_to)) = mail.reply.clone() {
                composer
                    .to
                    .update(cx, |input, cx| input.set(address, usize::MAX, cx));
                composer.reply = Some(reply_to);
                focus = composer.lines[0].read(cx).focus_handle(cx);
            }
        }
        self.pending_focus = Some(focus);
        self.compose = Some(composer);
        cx.notify();
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
            body: composer
                .lines
                .iter()
                .map(|line| line.read(cx).text())
                .collect::<Vec<_>>()
                .join("\n"),
            reply: composer.reply.clone(),
        };
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
                .spawn(async move { client.send_message(&outgoing) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.close_compose(cx);
                    this.notice = Some("Message sent.".into());
                    if this.mailbox == View::Sent {
                        this.fetch_inbox(cx);
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
                self.load_selected(cx);
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
        self.load_selected(cx);
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
        self.load_selected(cx);
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
            self.fetch_inbox(cx);
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
        self.load_selected(cx);
        cx.notify();
    }

    /// `Z`: brings back the last thread that was archived, moved or trashed.
    fn undo(&mut self, cx: &mut Context<Self>) {
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
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                control("back", self.can_go_back())
                    .id("go-back")
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
            )
            .child(
                control("next", self.can_go_forward())
                    .id("go-forward")
                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                div()
                    .id("reload")
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.palette = true;
                        cx.notify();
                    }))
                    .child(icon("search", 16., t.text3))
                    .child(
                        div()
                            .flex_1()
                            .text_color(t.text3)
                            .child("Search or jump to…"),
                    )
                    .child(kbd(t, "⌘"))
                    .child(kbd(t, "K")),
            )
            .child(section(t, "SPLIT INBOX"))
            .child(mailbox(
                View::Inbox,
                "inbox",
                (unread > 0 && self.mailbox == View::Inbox).then(|| unread.to_string()),
            ))
            .when(demo, |d| {
                d.child(nav(t, "users", "Team", fake("4"), false, None))
                    .child(nav(t, "calendar", "Calendar", fake("2"), false, None))
                    .child(nav(t, "news", "Newsletters", fake("31"), false, None))
                    .child(nav(t, "zap", "Other", fake("23"), false, None))
            })
            // With real mail the split inbox is Gmail's own categories.
            .when(!demo, |d| {
                d.child(mailbox(View::Primary, "users", None))
                    .child(mailbox(View::Updates, "bell", None))
                    .child(mailbox(View::Promotions, "tag", None))
                    .child(mailbox(View::Social, "news", None))
            })
            .child(section(t, "MAILBOX"))
            .child(mailbox(View::Starred, "star", None))
            .child(mailbox(View::Snoozed, "clock", fake("3")))
            .child(nav(t, "bell", "Reminders", fake("2"), false, None))
            .child(mailbox(View::Drafts, "draft", fake("1")))
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
                                None,
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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open(i);
                this.load_selected(cx);
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
                                    .child(short(&m.sender, 30)),
                            )
                            .children(m.thread.clone().map(|n| mono(n, t.text3)))
                            .when(m.starred, |d| d.child(icon("star", 13., t.amber)))
                            .children(m.tag.clone().map(|(label, a)| pill(t, &label, t.accent(a))))
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
                                    .child(short(
                                        m.subject.split(',').next().unwrap_or(&m.subject),
                                        48,
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(t.text3)
                                    .child(short(&m.snippet, 70)),
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
            .w(px(452.))
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
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
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_compose(false, cx)),
                                    ),
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

    fn composer(&self, t: &Theme, m: &Mail, cx: &mut Context<Self>) -> impl IntoElement {
        let (typed, ghost) = (&m.draft.0, &m.draft.1);
        let first = m.sender.split(' ').next().unwrap_or(&m.sender).to_string();
        let draft: SharedString = if typed.is_empty() {
            format!("Write a reply to {first}…").into()
        } else {
            format!("{typed}{ghost}").into()
        };
        let ghost_style = HighlightStyle {
            color: Some(t.faint),
            ..Default::default()
        };
        let ghost_from = if typed.is_empty() { 0 } else { typed.len() };
        let len = draft.len();

        div().px(px(32.)).pt(px(6.)).pb(px(24.)).child(
            div()
                .id("reply-box")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.open_compose(true, cx)))
                .flex()
                .flex_col()
                .gap(px(12.))
                .pl(px(18.))
                .pr(px(14.))
                .pt(px(14.))
                .pb(px(12.))
                .rounded(px(16.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![shadow(t.shadow, 18., 48., -8.)])
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("reply", 16., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text2)
                                .child(format!("Reply to {first}")),
                        )
                        .when(!typed.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded_full()
                                    .bg(t.selected)
                                    .child(icon("sparkles", 14., t.violet))
                                    .child(meta("Draft in your voice", t.violet)),
                            )
                        }),
                )
                .child(div().text_size(px(15.)).line_height(px(26.)).child(
                    StyledText::new(draft).with_highlights([(ghost_from..len, ghost_style)]),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(icon_button(t, "clip"))
                        .child(icon_button(t, "image"))
                        .child(icon_button(t, "link"))
                        .child(icon_button(t, "clock"))
                        .when(!ghost.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .pl(px(8.))
                                    .child(kbd(t, "Tab"))
                                    .child(meta("to accept", t.text3)),
                            )
                        })
                        .child(div().flex_1())
                        .child(button(t, ButtonKind::Ghost, "Send later", None))
                        .child(button(t, ButtonKind::Primary, "Send", Some("⌘↵"))),
                ),
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
            // Toolbar
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(28.))
                    .pr(px(20.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(t.border)
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        icon_button(t, "done")
                            .id("mark-done")
                            .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                    )
                    .child(kbd(t, "E"))
                    .child(div().w(px(6.)))
                    .child(icon_button(t, "clock").id("snooze").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.palette = true;
                            cx.notify();
                        },
                    )))
                    .child(
                        icon_button(t, "star")
                            .id("star")
                            .when(m.starred, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(icon_button(t, "tag"))
                    .child(
                        icon_button(t, "trash")
                            .id("trash")
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(t.border_strong))
                    .child(icon_button(t, "reply"))
                    .child(icon_button(t, "reply-all"))
                    .child(icon_button(t, "forward"))
                    .child(div().flex_1())
                    .child(mono(format!("{pos} / {}", visible.len()), t.text3).mr(px(6.)))
                    .child(
                        icon_button(t, "up")
                            .id("prev")
                            .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                    )
                    .child(
                        icon_button(t, "down")
                            .id("next")
                            .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                    )
                    .child(icon_button(t, "more")),
            )
            // Conversation
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
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
                            .child(
                                self.message_body(t, &m.body, 0, cx).pl(px(40.)).when(
                                    !m.loaded,
                                    |d| {
                                        d.child(div().text_color(t.text3).child("Loading message…"))
                                    },
                                ),
                            )
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
            .child(self.composer(t, m, cx))
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
                    .w(px(452.))
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

    fn compose_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(composer) = &self.compose else {
            return div().into_any_element();
        };
        let field = |label: &str, input: &Entity<TextInput>| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(20.))
                .py(px(10.))
                .border_b_1()
                .border_color(t.border)
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
                )
        };
        div()
            .id("compose-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(96.))
            .bg(t.scrim)
            .occlude()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(720.))
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.18), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px(px(20.))
                            .pt(px(16.))
                            .pb(px(8.))
                            .child(
                                serif(
                                    if composer.reply.is_some() {
                                        "Reply"
                                    } else {
                                        "New message"
                                    },
                                    26.,
                                    t.text,
                                )
                                .flex_1(),
                            )
                            .child(kbd(t, "esc")),
                    )
                    .child(field("To", &composer.to))
                    .child(field("Subject", &composer.subject))
                    .child(
                        div()
                            .id("compose-body")
                            .flex()
                            .flex_col()
                            .px(px(20.))
                            .py(px(14.))
                            .min_h(px(240.))
                            .max_h(px(420.))
                            .overflow_y_scroll()
                            .text_size(px(15.))
                            .children(composer.lines.iter().cloned()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(t.border)
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
                                        None => {
                                            "Sent as plain text from your Gmail address.".into()
                                        }
                                    }),
                            )
                            .child(
                                button(t, ButtonKind::Ghost, "Discard", None)
                                    .id("discard")
                                    .on_click(cx.listener(|this, _, _, cx| this.close_compose(cx))),
                            )
                            .child(
                                button(t, ButtonKind::Primary, "Send", Some("⌘↵"))
                                    .id("send")
                                    .when(composer.sending, |d| d.opacity(0.5))
                                    .on_click(cx.listener(|this, _, _, cx| this.send_mail(cx))),
                            ),
                    ),
            )
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
                &format!("Nothing in {}", self.inbox_name()),
                56.,
                t.text,
            ))
            .child(
                div()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .text_color(t.text2)
                    .child("Threads will show up here."),
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
            .on_action(cx.listener(|this, _: &Compose, _, cx| this.open_compose(false, cx)))
            .on_action(cx.listener(|this, _: &Reply, _, cx| this.open_compose(true, cx)))
            .on_action(cx.listener(|this, _: &SendMail, _, cx| this.send_mail(cx)))
            .on_action(cx.listener(|this, _: &ToggleStar, _, cx| this.toggle_star(cx)))
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
                if this.compose.is_some() {
                    this.close_compose(cx);
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
                Status::Demo | Status::Ready => {
                    d.child(self.list(&t, cx)).child(self.reading(&t, cx))
                }
                Status::Loading => d.child(self.skeleton(&t)),
                _ => d.child(self.status_screen(&t, cx)),
            })
            .child(self.window_controls(&t, cx))
            .when(self.palette && !empty, |d| d.child(self.palette(&t, cx)))
            .when(self.compose.is_some(), |d| {
                d.child(self.compose_view(&t, cx))
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
