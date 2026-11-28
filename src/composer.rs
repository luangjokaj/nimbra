//! The composer: writing a message (new, reply, forward or a draft), floating over the mail,
//! full page or in a window of its own. Its state, its fields and editor, saving it as a draft,
//! and how it is drawn. Sending is in `outgoing.rs`.

use super::*;

/// A point or a size, in window points.
type Xy = (f32, f32);

/// The open composer: two single-line fields and a body made of one field per line.
pub(super) struct Composer {
    /// Tells composers apart while one is still finishing in the background.
    pub(super) serial: u64,
    pub(super) to: Entity<TextInput>,
    pub(super) cc: Entity<TextInput>,
    pub(super) bcc: Entity<TextInput>,
    /// The Cc and Bcc fields are shown (after "Cc Bcc" is clicked, or when they have text).
    pub(super) show_cc: bool,
    /// Finished addresses in To, Cc and Bcc, shown as pills; the fields hold only what is
    /// still being typed.
    pub(super) chips: [Vec<String>; 3],
    /// Address suggestions under a recipient field.
    pub(super) suggest: Option<Suggest>,
    /// Files picked or dropped in.
    pub(super) files: Vec<mail::OutgoingFile>,
    /// Files already in Gmail: attached to the draft being edited, or to the message being
    /// forwarded. Only the account they are in can read them.
    pub(super) kept: Vec<mail::Attachment>,
    /// The account From is changing to, while `kept` is read from the one it is leaving
    /// (see `Nimbra::set_compose_account`).
    pub(super) moving_to: Option<String>,
    /// Counts edits; the draft is saved a moment after the last one.
    pub(super) edits: u64,
    /// The edit count the saved draft has.
    pub(super) saved_edits: u64,
    /// A save was asked for while one was running.
    pub(super) resave: bool,
    /// What to do once the draft is saved.
    pub(super) after_save: Option<After>,
    /// Written in the formatted editor (a web view) rather than the plain line fields.
    pub(super) rich: bool,
    /// The editor's content, as HTML and as plain text.
    pub(super) html: String,
    pub(super) text: String,
    /// The signature under what is written, kept apart from it: its id (empty for one a
    /// draft was saved with that Settings no longer has) and its cleaned HTML. `None`: no
    /// signature. Only a formatted message has one.
    pub(super) signature: Option<(String, String)>,
    /// The signature changed and the editor still shows the old one.
    pub(super) show_signature: bool,
    /// Content still to be put into the editor.
    pub(super) load: Option<String>,
    /// Bold, italic, underline, bullets, numbers at the cursor.
    pub(super) format: [bool; 5],
    /// Put the keyboard in the message body once the editor shows it.
    pub(super) focus_body: bool,
    pub(super) subject: Entity<TextInput>,
    pub(super) lines: Vec<Entity<TextInput>>,
    pub(super) reply: Option<mail::ReplyTo>,
    /// A reply's own account and its thread there, once From changed: a reply written from
    /// another account has no thread in it (only its reply headers), and has it again when
    /// From changes back.
    pub(super) reply_home: Option<(String, String)>,
    /// The Gmail draft being edited, if this message was saved or opened from Drafts.
    pub(super) draft_id: Option<String>,
    /// The thread its row has in Drafts, to show its edits there at once when it is saved;
    /// `listed_new`: a row made up for it until Drafts is listed again (a new draft).
    pub(super) listed_as: Option<String>,
    pub(super) listed_new: bool,
    /// The account it is written from, when not the one shown (a reply in All inboxes).
    pub(super) account: Option<String>,
    /// The earlier message a draft quotes or forwards. Kept out of the editor behind a
    /// "…" toggle and added back when the message is saved or sent.
    pub(super) quoted: Option<String>,
    pub(super) show_quoted: bool,
    pub(super) saving: bool,
    /// What the last save left in Gmail, set by the thread that saved it (for quitting).
    pub(super) saved_as: SavedAs,
    /// The last save succeeded (shown in the footer).
    pub(super) saved: bool,
    /// Fills the window instead of floating over the mail.
    pub(super) full: bool,
    /// Top-left corner of the floating card; `None` until it is first laid out, centred.
    pub(super) position: Option<(f32, f32)>,
    pub(super) size: (f32, f32),
    /// An in-progress move or resize: what, where the mouse started, and the card's
    /// position and size at that moment.
    pub(super) grab: Option<(Grab, Xy, Xy, Xy)>,
    pub(super) sending: bool,
    pub(super) error: Option<SharedString>,
    pub(super) _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ComposeKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// What the mouse is doing to the floating composer.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Grab {
    Move,
    /// Resizing from an edge or corner: which way each side moves (-1 the left or top
    /// edge, 1 the right or bottom one, 0 that axis stays).
    Resize(i8, i8),
}

pub(super) const COMPOSER_MIN: (f32, f32) = (460., 320.);

/// Address suggestions for one recipient field.
pub(super) struct Suggest {
    pub(super) field: Field,
    pub(super) people: Vec<mail::Person>,
    pub(super) selected: usize,
}

/// Something to do once a composer's draft is saved.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum After {
    /// Send it at this time (milliseconds since the epoch).
    Schedule(i64),
}

/// How the ids of rows made up for new drafts start (`show_in_drafts`): never a
/// provider's thread id.
const MADE_UP: &str = "nimbra-draft-";

/// Whether a row's thread id is one made up for a new draft: never sent to the provider.
pub(super) fn made_up(id: &str) -> bool {
    id.starts_with(MADE_UP)
}

/// A composer that is off screen but still has work to finish.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Parked {
    /// Closed; kept until its last edits are saved.
    Closing,
    /// Sent, waiting out the undo delay.
    Sending,
    /// Being sent now; brought back if sending fails.
    InFlight,
    /// Discarded, waiting out the undo delay before its draft is deleted.
    Discarded,
}

impl Composer {
    pub(super) fn input(&self, field: Field) -> Option<Entity<TextInput>> {
        match field {
            Field::To => Some(self.to.clone()),
            Field::Cc => Some(self.cc.clone()),
            Field::Bcc => Some(self.bcc.clone()),
            Field::Subject => Some(self.subject.clone()),
            Field::Line(index) => self.lines.get(index).cloned(),
        }
    }

    /// Where a recipient field keeps its pills.
    pub(super) fn slot(field: Field) -> Option<usize> {
        match field {
            Field::To => Some(0),
            Field::Cc => Some(1),
            Field::Bcc => Some(2),
            _ => None,
        }
    }

    /// A recipient field's pills and typed text, as one comma-separated list.
    pub(super) fn recipients(&self, field: Field, cx: &App) -> String {
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
    pub(super) fn chipify(&mut self, field: Field, all: bool, cx: &mut App) {
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
            match mail::addresses(part) {
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
    pub(super) fn sync_placeholder(&self, field: Field, cx: &mut App) {
        if let (Field::To, Some(input)) = (field, self.input(field)) {
            let hint = if self.chips[0].is_empty() {
                "name@example.com"
            } else {
                ""
            };
            input.update(cx, |input, cx| input.set_placeholder(hint, cx));
        }
    }

    /// The signature's HTML, when the message has one.
    fn signature_html(&self) -> Option<&str> {
        self.signature
            .as_ref()
            .map(|(_, html)| html.as_str())
            .filter(|_| self.rich)
    }

    /// The message as it would be saved or sent.
    pub(super) fn outgoing(&self, cx: &App) -> mail::Outgoing {
        let html = self.rich.then(|| {
            crate::editor::message_html(&self.html, self.signature_html(), self.quoted.as_deref())
        });
        mail::Outgoing {
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
    pub(super) fn has_content(&self, cx: &App) -> bool {
        (self.rich && !self.text.trim().is_empty())
            || self.chips.iter().any(|c| !c.is_empty())
            || !self.files.is_empty()
            || !self.kept.is_empty()
            || [&self.to, &self.cc, &self.bcc, &self.subject]
                .iter()
                .chain(self.lines.iter().collect::<Vec<_>>().iter())
                .any(|input| !input.read(cx).text().trim().is_empty())
    }

    pub(super) fn dirty(&self) -> bool {
        self.edits != self.saved_edits
    }

    /// The whole message text: the edited lines, the signature after the usual "-- "
    /// line, then the quoted earlier message.
    pub(super) fn body(&self, cx: &App) -> String {
        let mut body = if self.rich {
            self.text.trim_end().to_string()
        } else {
            self.lines
                .iter()
                .map(|line| line.read(cx).text())
                .collect::<Vec<_>>()
                .join("\n")
        };
        if let Some(signature) = self.signature_html() {
            let text = crate::signature::to_text(signature);
            if !text.is_empty() {
                body.push_str("\n\n-- \n");
                body.push_str(&text);
            }
        }
        if let Some(quoted) = &self.quoted {
            body.push_str("\n\n");
            body.push_str(quoted);
        }
        body
    }
}

/// Which composer field an event came from.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Field {
    To,
    Cc,
    Bcc,
    Subject,
    Line(usize),
}

impl Nimbra {
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
    pub(super) fn open_compose(&mut self, kind: ComposeKind, cx: &mut Context<Self>) {
        self.leave_sidebar(cx);
        let source = self
            .mails
            .get(self.selected)
            .filter(|_| !self.visible().is_empty())
            .map(Source::latest);
        self.open_compose_from(kind, source, cx);
    }

    /// Opens the composer to reply to, reply to all on, or forward one message.
    pub(super) fn open_compose_from(
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
            moving_to: None,
            edits: 0,
            saved_edits: 0,
            resave: false,
            after_save: None,
            rich: !self.editor_failed && cfg!(any(target_os = "macos", target_os = "windows")),
            html: String::new(),
            text: String::new(),
            signature: None,
            show_signature: true,
            load: None,
            format: [false; 5],
            focus_body: false,
            subject,
            lines: vec![line],
            reply: None,
            reply_home: None,
            full: false,
            position: None,
            size: (720., 520.),
            grab: None,
            draft_id: None,
            listed_as: None,
            listed_new: false,
            // A new message is from the account shown; an answer from its message's.
            account: if kind == ComposeKind::New {
                self.new_message_account()
            } else {
                source.as_ref().and_then(|s| s.account.clone())
            },
            quoted: None,
            show_quoted: false,
            saving: false,
            saved_as: SavedAs::default(),
            saved: false,
            sending: false,
            error: None,
            _subscriptions: vec![s1, s2, s3, s4, s5],
        };
        let mut focus = composer.to.read(cx).focus_handle(cx);
        if kind != ComposeKind::New
            && let Some(source) = source
        {
            use nimbra_core::composing::{self, Answer};
            // Who "me" is: the address of the account the message is in (in All inboxes,
            // not necessarily the one shown).
            let me = source
                .account
                .as_ref()
                .and_then(|id| {
                    self.signature_accounts()
                        .into_iter()
                        .find(|(account, _)| account == id)
                })
                .map_or_else(|| self.account.to_string(), |(_, email)| email);
            let answered = composing::answer(
                match kind {
                    ComposeKind::Forward => Answer::Forward,
                    ComposeKind::ReplyAll => Answer::ReplyAll,
                    _ => Answer::Reply,
                },
                &composing::Source {
                    subject: &source.subject,
                    sender: &source.sender,
                    header: &source.header,
                    body: &source.body,
                    reply_to: source.reply_to.clone(),
                    fallback_to: source.fallback_to.clone(),
                    fallback_all: &source.fallback_all,
                },
                &me,
                &long_date(source.header.date_ms),
                self.settings.quote_replies,
            );
            composer
                .subject
                .update(cx, |input, cx| input.set(answered.subject, usize::MAX, cx));
            if kind == ComposeKind::Forward {
                // The forwarded message is folded behind "•••" and added when sent. Its
                // files go along: read from Gmail when it is saved or sent, and shown as
                // chips, each with a button to leave it out.
                composer.quoted = answered.quoted;
                composer.kept = answered.kept;
            } else {
                // Each address in the field it was in (reply-all keeps Cc on Cc, and on my
                // own message Bcc on Bcc), with the Cc and Bcc row open when either has one.
                composer.show_cc = !answered.cc.is_empty() || !answered.bcc.is_empty();
                for (field, list) in [
                    (Field::To, &answered.to),
                    (Field::Cc, &answered.cc),
                    (Field::Bcc, &answered.bcc),
                ] {
                    if list.is_empty() {
                        continue;
                    }
                    let text = list.join(", ");
                    if let Some(input) = composer.input(field) {
                        input.update(cx, |input, cx| input.set(text, usize::MAX, cx));
                    }
                    composer.chipify(field, true, cx);
                }
                composer.reply = answered.reply;
                // The message being answered is quoted below, folded behind "•••".
                composer.quoted = answered.quoted;
                focus = composer.lines[0].read(cx).focus_handle(cx);
            }
        }
        // The editor starts empty; the account's default signature is shown under it.
        composer.load = Some("<div><br></div>".to_string());
        let from = composer
            .account
            .clone()
            .unwrap_or_else(|| self.account_id.clone());
        composer.signature = self.default_signature_for(&from);
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
    pub(super) fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // While asking whether to save, the editor (a native view) steps aside so the
        // question shows over the composer.
        if self.compose_ask {
            if let Some(editor) = &self.editor {
                editor.set_visible(false);
                editor.blur();
            }
            return;
        }
        let Some(serial) = self.compose.as_ref().filter(|c| c.rich).map(|c| c.serial) else {
            if self.editor_shown.take().is_some()
                && let Some(editor) = &self.editor
            {
                editor.set_visible(false);
                editor.blur();
            }
            return;
        };
        // The composer moved to another window: the editor is made again there (its content
        // is kept in the composer and loaded once the new one is ready).
        if self.editor.is_some() && self.editor_window != Some(window.window_handle()) {
            if let Some(editor) = self.editor.take() {
                editor.set_visible(false);
            }
            self.editor_ready = false;
            self.editor_painted = false;
            self.editor_shown = None;
            self.editor_theme = None;
            self.editor_keys = false;
        }
        if self.editor.is_none() && !self.editor_failed {
            self.editor_window = Some(window.window_handle());
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
            composer.show_signature = true;
            self.editor_shown = Some(serial);
        } else if focus {
            editor.focus();
        }
        if std::mem::take(&mut composer.show_signature) {
            let shown = composer
                .signature
                .as_ref()
                .map(|(_, html)| crate::signature::for_display(html))
                .unwrap_or_default();
            editor.signature(&shown);
            // Pictures not downloaded yet: shown once they are.
            let sources = composer
                .signature
                .as_ref()
                .map(|(_, html)| crate::signature::image_sources(html))
                .unwrap_or_default();
            let this = cx.entity().downgrade();
            crate::signature_images::ensure(sources, cx, move |cx| {
                this.update(cx, |this, cx| {
                    if let Some(composer) = &mut this.compose {
                        composer.show_signature = true;
                    }
                    cx.notify();
                })
                .ok();
            });
        }
        editor.set_opaque(self.editor_painted);
        editor.set_visible(true);
    }

    fn editor_event(&mut self, event: crate::editor::EditorEvent, cx: &mut Context<Self>) {
        use crate::editor::EditorEvent;
        match event {
            EditorEvent::Ready => {
                self.editor_ready = true;
                self.editor_painted = false;
                self.editor_shown = None;
                self.editor_theme = None;
            }
            EditorEvent::Painted => {
                if !self.editor_painted {
                    self.editor_painted = true;
                    cx.notify();
                }
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
            EditorEvent::Escape => self.request_close_compose(cx),
            EditorEvent::Send => self.send_mail(cx),
            EditorEvent::Save => self.save_draft(cx),
            EditorEvent::ToggleFull => self.toggle_full_compose(cx),
            EditorEvent::Dropped(paths) => self.attach_files(paths, cx),
            EditorEvent::SignatureMenu { session, at } => {
                if self.compose.as_ref().is_some_and(|c| c.serial == session) {
                    self.open_signature_menu(at, cx);
                }
            }
            // Only the signature editor in Settings edits source.
            EditorEvent::Source { .. } => {}
        }
        cx.notify();
    }

    /// The signature a message from `account` starts with: the account's default.
    pub(super) fn default_signature_for(&self, account: &str) -> Option<(String, String)> {
        self.settings
            .default_signature(account)
            .filter(|s| !s.html.trim().is_empty())
            .map(|s| (s.id.clone(), s.html.clone()))
    }

    /// The menu of the From account's signatures, under the handle above the signature
    /// (`at`, in the editor). It is a window of its own, opened once this update is over.
    fn open_signature_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(handle) = self.editor_window else {
            return;
        };
        let origin = self.editor_origin.get();
        self.menu_at = (origin.0 + at.0, origin.1 + at.1 + 4.);
        cx.spawn(async move |this, cx| {
            cx.update_window(handle, |_, window, cx| {
                this.update(cx, |this, cx| this.toggle_menu(Menu::Signature, window, cx))
            })
            .ok();
        })
        .detach();
    }

    /// The composer's signature changes to `id` (or, with `None`, to none).
    pub(super) fn set_compose_signature(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let signature = id.and_then(|id| {
            self.settings
                .signature(&id)
                .map(|s| (s.id.clone(), s.html.clone()))
        });
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.signature == signature {
            return;
        }
        composer.signature = signature;
        composer.show_signature = true;
        self.composer_changed(cx);
        cx.notify();
    }

    /// Starts moving or resizing the floating composer.
    fn grab_composer(&mut self, grab: Grab, event: &gpui::MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.compose
            && let Some(position) = composer.position
            && !composer.full
        {
            let mouse = (from_screen(event.position.x), from_screen(event.position.y));
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
        let dx = from_screen(event.position.x) - start.0;
        let dy = from_screen(event.position.y) - start.1;
        match grab {
            Grab::Move => {
                composer.position = Some((
                    (position.0 + dx).clamp(80. - size.0, viewport.0 - 80.),
                    (position.1 + dy).clamp(0., viewport.1 - 60.),
                ));
            }
            Grab::Resize(hx, hy) => {
                // One axis: the far edge stays put, the grabbed one follows the mouse,
                // within the window and never below the smallest size.
                let axis =
                    |side: i8, start: f32, length: f32, delta: f32, room: f32, min: f32| match side
                    {
                        1 => (start, (length + delta).clamp(min, (room - start).max(min))),
                        -1 => {
                            let end = start + length;
                            let begin = (start + delta).clamp(0., end - min);
                            (begin, end - begin)
                        }
                        _ => (start, length),
                    };
                let (x, w) = axis(hx, position.0, size.0, dx, viewport.0, COMPOSER_MIN.0);
                let (y, h) = axis(hy, position.1, size.1, dy, viewport.1, COMPOSER_MIN.1);
                composer.position = Some((x, y));
                composer.size = (w, h);
            }
        }
        cx.notify();
    }

    /// The composer, shown in `window` when that is where it lives: full page, with the
    /// composer's keys, the editor and the keyboard focus. None anywhere else.
    pub(super) fn hosted_compose(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.compose.is_none() || self.compose_host != Some(window.window_handle()) {
            return None;
        }
        let size = window.viewport_size();
        let viewport = (from_screen(size.width), from_screen(size.height));
        let detached = self.compose_detached;
        if let Some(composer) = &mut self.compose {
            // A window of its own is all composer; elsewhere it floats as in the main window.
            if detached {
                composer.full = true;
            } else if composer.position.is_none() {
                // In the bottom-right corner, sized to fit the window with a margin.
                const MARGIN: f32 = 20.;
                composer.size = (
                    composer
                        .size
                        .0
                        .min(620.)
                        .min(viewport.0 - 2. * MARGIN)
                        .max(COMPOSER_MIN.0),
                    composer.size.1.min(viewport.1 - 92.).max(COMPOSER_MIN.1),
                );
                composer.position = Some((
                    (viewport.0 - composer.size.0 - MARGIN).max(0.),
                    (viewport.1 - composer.size.1 - MARGIN).max(0.),
                ));
            }
        }
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus);
        }
        self.sync_editor(window, cx);
        let t = self.theme();
        Some(
            div()
                .absolute()
                .inset_0()
                .key_context("Composer")
                .on_action(cx.listener(|this, _: &SendMail, _, cx| this.send_mail(cx)))
                .on_action(cx.listener(|this, _: &SaveDraft, _, cx| this.save_draft(cx)))
                .on_action(
                    cx.listener(|this, _: &ToggleFullCompose, _, cx| this.toggle_full_compose(cx)),
                )
                .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                    if let Some(composer) = this.compose.as_mut()
                        && composer.suggest.take().is_some()
                    {
                        cx.notify();
                    } else {
                        this.request_close_compose(cx);
                    }
                }))
                .child(self.compose_view(&t, viewport, cx))
                .into_any_element(),
        )
    }

    /// Moves the composer into a window of its own.
    fn detach_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose.is_none() {
            return;
        }
        let owner = cx.entity();
        self.compose_home = self.compose_host;
        cx.defer(move |cx| {
            let bounds =
                gpui::Bounds::centered(None, gpui::size(to_screen(820.), to_screen(700.)), cx);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("New message".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(18.), px(18.))),
                    }),
                    window_min_size: Some(gpui::size(to_screen(520.), to_screen(420.))),
                    ..Default::default()
                },
                |window, cx| {
                    let guard = owner.clone();
                    window.on_window_should_close(cx, move |window, cx| {
                        let handle = window.window_handle();
                        guard.update(cx, |n, cx| n.may_close_window(handle, cx))
                    });
                    cx.new(|cx| ComposeWindow {
                        _observe: cx.observe(&owner, |_, _, cx| cx.notify()),
                        owner,
                        shown: false,
                    })
                },
            );
            if let Ok(handle) = opened {
                me.update(cx, |n, cx| {
                    n.compose_host = Some(handle.into());
                    n.compose_detached = true;
                    if let Some(composer) = &n.compose {
                        n.pending_focus = Some(composer.to.read(cx).focus_handle(cx));
                    }
                    cx.notify();
                });
            }
        });
    }

    /// Puts a composer in a window of its own back where it came from (the main window if
    /// that message window has closed), floating in its corner. Its window then closes.
    fn attach_compose(&mut self, cx: &mut Context<Self>) {
        if !self.compose_detached || self.compose.is_none() {
            return;
        }
        let home = self
            .compose_home
            .take()
            .filter(|home| cx.windows().contains(home));
        self.compose_host = home;
        self.compose_detached = false;
        if let Some(composer) = &mut self.compose {
            composer.full = false;
            composer.position = None;
            composer.grab = None;
            self.pending_focus = Some(composer.to.read(cx).focus_handle(cx));
        }
        if let Some(home) = home {
            cx.defer(move |cx| {
                home.update(cx, |_, window, _| window.activate_window())
                    .ok();
            });
        } else {
            crate::show_main_window(cx);
        }
        cx.notify();
    }

    pub(super) fn toggle_full_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose_detached {
            return;
        }
        if let Some(composer) = &mut self.compose {
            composer.full = !composer.full;
            cx.notify();
        }
    }

    /// Opens the draft behind a row of the Drafts mailbox in the composer, from the row's
    /// own account (every account's Drafts mixes them); `in_window`: in a window of its own
    /// (a double-click on the row), as a detached composer is.
    pub(super) fn open_draft(&mut self, index: usize, in_window: bool, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get(index) else {
            return;
        };
        let account = self.account_of(mail).to_string();
        let Some(thread_id) = mail.id.clone() else {
            return;
        };
        if self.compose.is_some() {
            return;
        }
        // A row made up for a draft written here: its own composer comes back.
        if made_up(&thread_id) {
            self.reopen_made_up(&thread_id, in_window, cx);
            return;
        }
        if self.client_at(index).is_none() {
            return;
        }
        self.notice = None;
        // Fetched ahead as its row was opened (`prefetch_draft`): the composer opens now.
        let key = (message_window::key_of(mail), account.clone());
        if let Some(draft) = self.drafts_ready.remove(&key) {
            self.draft_wanted = None;
            self.open_fetched_draft(draft, account, thread_id, in_window, cx);
            return;
        }
        // On its way (or fetched now): opened as soon as it is here, with the loading line
        // under the toolbar meanwhile. Pressing again changes nothing.
        self.draft_wanted = Some((key.clone(), in_window));
        if !self.drafts_fetching.contains(&key) {
            self.prefetch_draft(index, cx);
        }
        cx.notify();
    }

    /// A made-up row trashed: its draft is deleted through the composer that wrote it,
    /// once a save running for it is over.
    pub(super) fn trash_made_up(&mut self, id: &str, cx: &mut Context<Self>) {
        let composer = match self.closed_drafts.iter().position(|(i, _)| i == id) {
            Some(pos) => Some(self.closed_drafts.remove(pos).1),
            None => self
                .parked
                .iter()
                .position(|(kind, c)| {
                    matches!(kind, Parked::Closing) && c.listed_as.as_deref() == Some(id)
                })
                .map(|pos| self.parked.remove(pos).1),
        };
        let Some(composer) = composer else {
            return;
        };
        let serial = composer.serial;
        self.parked.push((Parked::Discarded, composer));
        self.finish_parked_later(serial, Duration::ZERO, cx);
    }

    /// Edit draft on a made-up row: its composer, still saving or closed since.
    fn reopen_made_up(&mut self, id: &str, in_window: bool, cx: &mut Context<Self>) {
        let parked = self
            .parked
            .iter()
            .find(|(kind, c)| matches!(kind, Parked::Closing) && c.listed_as.as_deref() == Some(id))
            .map(|(_, c)| c.serial);
        let serial = match parked {
            Some(serial) => serial,
            None => {
                let Some(pos) = self.closed_drafts.iter().position(|(i, _)| i == id) else {
                    // Gone with an earlier listing: Drafts shows its real row.
                    self.refresh(cx);
                    return;
                };
                let (_, composer) = self.closed_drafts.remove(pos);
                let serial = composer.serial;
                self.parked.push((Parked::Closing, composer));
                serial
            }
        };
        self.unpark(serial, cx);
        if in_window && self.compose.is_some() {
            self.detach_compose(cx);
        }
        cx.notify();
    }

    /// The composer with a draft fetched for its row in Drafts (thread `thread`).
    fn open_fetched_draft(
        &mut self,
        draft: mail::Draft,
        account: String,
        thread: String,
        in_window: bool,
        cx: &mut Context<Self>,
    ) {
        self.edit_draft(draft, Some(account), cx);
        if let Some(composer) = &mut self.compose {
            composer.listed_as = Some(thread);
        }
        if in_window {
            self.detach_compose(cx);
        }
    }

    /// Fetches the draft behind the row in Drafts at `index` in the background, so Edit
    /// draft (or a double-click) opens it at once. Each time the row is opened, as the draft
    /// may have changed elsewhere.
    pub(super) fn prefetch_draft(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get(index) else {
            return;
        };
        let key = (
            message_window::key_of(mail),
            self.account_of(mail).to_string(),
        );
        let (Some(client), Some(thread_id)) = (self.client_at(index), mail.id.clone()) else {
            return;
        };
        if made_up(&thread_id) {
            return; // its composer is here, not in Gmail
        }
        if !self.drafts_fetching.insert(key.clone()) {
            return; // already on its way
        }
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.draft_for_thread(&thread_id) })
                .await;
            this.update(cx, |this, cx| {
                this.drafts_fetching.remove(&key);
                let wanted = this
                    .draft_wanted
                    .as_ref()
                    .filter(|(k, _)| *k == key)
                    .map(|(_, in_window)| *in_window);
                if wanted.is_some() {
                    this.draft_wanted = None;
                    cx.notify();
                }
                if this.account_gen != generation {
                    return;
                }
                match (result, wanted) {
                    (Ok(draft), Some(in_window)) if this.compose.is_none() => {
                        let (thread, account) = key;
                        this.open_fetched_draft(draft, account, thread, in_window, cx);
                    }
                    (Ok(draft), _) => {
                        this.drafts_ready.insert(key, draft);
                    }
                    (Err(error), Some(_)) => this.fail(error, cx),
                    (Err(_), None) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Hides (or shows again) a discarded draft's row in Drafts while its delete waits out
    /// the undo delay; the listing after the delete has it gone for good.
    fn show_draft_row(
        &mut self,
        account: Option<String>,
        id: &str,
        show: bool,
        cx: &mut Context<Self>,
    ) {
        let shown = self.account_id.clone();
        let account = account.unwrap_or_else(|| shown.clone());
        let mut changed = false;
        for index in 0..self.mails.len() {
            let mail = &self.mails[index];
            if mail.id.as_deref() == Some(id)
                && self.account_of(mail) == account
                && !self.history.contains(&index)
            {
                self.done[index] = !show;
                changed = true;
            }
        }
        if !changed {
            return;
        }
        self.picked.retain(|i| !self.done[*i]);
        if self.picked.len() < 2 {
            self.picked.clear();
        }
        let visible = self.visible();
        if !visible.contains(&self.selected) {
            self.selected = visible.first().copied().unwrap_or(0);
            self.load_selected(cx);
        }
        cx.notify();
    }

    /// Shows a composer's draft in Drafts as it is being saved, before Gmail lists it: its
    /// row brought up to date, or for a new draft a row made up from it, until Drafts is
    /// listed again (`composer_saved`).
    fn show_in_drafts(&mut self, serial: u64, cx: &mut Context<Self>) {
        let shown = self.account_id.clone();
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        let account = composer.account.clone().unwrap_or_else(|| shown.clone());
        // Saved already, with nothing more to save: no save will come to take its made-up row
        // out of `pending` (`composer_saved`), so it is shown without, until the listing
        // asked for below has the real one.
        let confirmed = composer.draft_id.is_some() && !composer.dirty() && !composer.saving;
        let out = composer.outgoing(cx);
        let id = match composer.listed_as.clone() {
            Some(id) => id,
            None => {
                composer.listed_new = true;
                let id = format!("{MADE_UP}{serial}");
                composer.listed_as = Some(id.clone());
                id
            }
        };
        let new = composer.listed_new;
        let first = out.to.split(',').next().unwrap_or_default().trim();
        let (name, address) = match first.split_once('<') {
            Some((name, rest)) => (
                name.trim().trim_matches('"').trim(),
                rest.trim_end_matches('>').trim(),
            ),
            None => ("", first),
        };
        let snippet: String = out.body.split_whitespace().collect::<Vec<_>>().join(" ");
        let summary = mail::Summary {
            id: id.clone(),
            sender: if name.is_empty() { address } else { name }.to_string(),
            sender_address: address.to_string(),
            brand_domain: None,
            subject: out.subject.trim().to_string(),
            snippet: snippet.chars().take(200).collect(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            message_count: 1,
            unread: false,
            starred: false,
            labels: vec!["DRAFT".into()],
            message_labels: vec![vec!["DRAFT".into()]],
            senders: Vec::new(),
            people: Vec::new(),
        };
        let mut row: Mail = nimbra_core::listing::Row::from_summary(summary).into();
        row.account = (account != shown).then(|| account.clone());
        // Its content as written, so a made-up row opens without asking Gmail.
        let me = self
            .signature_accounts()
            .into_iter()
            .find(|(a, _)| *a == account)
            .map(|(_, email)| email)
            .unwrap_or_else(|| self.account.to_string());
        let people = |list: &str| -> Vec<mail::Person> {
            list.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|one| match one.split_once('<') {
                    Some((name, rest)) => mail::Person {
                        name: name.trim().trim_matches('"').trim().to_string(),
                        address: rest.trim_end_matches('>').trim().to_string(),
                    },
                    None => mail::Person {
                        name: String::new(),
                        address: one.to_string(),
                    },
                })
                .collect()
        };
        row.header.from = mail::Person {
            name: String::new(),
            address: me,
        };
        row.header.to = people(&out.to);
        row.header.cc = people(&out.cc);
        row.header.date_ms = row.timestamp_ms;
        let paragraphs: Vec<String> = out
            .body
            .split("\n\n")
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        row.body = nimbra_core::rich::from_text(paragraphs).into();
        row.html = out.html.clone();
        row.loaded = new;
        // Its row where it is listed now (on screen, or kept for a return): brought up to
        // date in place.
        let mut found = false;
        for mail in self
            .mails
            .iter_mut()
            .chain(self.cache.values_mut().flatten())
            .filter(|m| m.id.as_deref() == Some(id.as_str()))
            .filter(|m| m.account.as_deref().unwrap_or(&shown) == account)
        {
            mail.sender = row.sender.clone();
            mail.subject = row.subject.clone();
            mail.snippet = row.snippet.clone();
            mail.timestamp_ms = row.timestamp_ms;
            mail.refresh_time();
            if new {
                mail.header = row.header.clone();
                mail.body = row.body.clone();
                mail.html = row.html.clone();
                mail.loaded = true;
            } else {
                // Gmail's row: its content is fetched again when it is opened.
                mail.loaded = false;
            }
            found = true;
        }
        if found && !new {
            cx.notify();
            return;
        }
        // Not listed yet: in Drafts now, as new mail is in the Inbox (`arrived`).
        self.pending.add(View::Drafts, &account, &row);
        let refresh_after = confirmed;
        for view in [View::Drafts, View::every(View::Drafts)] {
            if view == self.mailbox {
                continue;
            }
            if let Some(list) = self.cache.get_mut(&view) {
                self.pending.apply(view, &shown, list, false, true, |m| {
                    m.account.clone().unwrap_or_else(|| shown.clone())
                });
            }
        }
        let here = self.mailbox == View::every(View::Drafts)
            || (self.mailbox == View::Drafts && account == shown);
        if !here || self.status != Status::Ready || found {
            if refresh_after {
                self.pending.forget(View::Drafts, &account, &id);
            }
            cx.notify();
            return;
        }
        let mut fresh = self.listed();
        self.pending
            .apply(self.mailbox, &shown, &mut fresh, false, true, |m| {
                m.account.clone().unwrap_or_else(|| shown.clone())
            });
        let pending = std::mem::take(&mut self.pending);
        let notice = self.notice.clone();
        self.install(fresh, cx);
        self.pending = pending;
        self.notice = notice;
        if refresh_after {
            self.pending.forget(View::Drafts, &account, &id);
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Opens `draft` in the composer, from `account` (whose draft it is; `None`: as a new
    /// message is), so it is saved and sent there again.
    pub(super) fn edit_draft(
        &mut self,
        draft: mail::Draft,
        account: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.open_compose(ComposeKind::New, cx);
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        if account.is_some() {
            composer.account = account;
        }
        composer
            .to
            .update(cx, |input, cx| input.set(draft.to, usize::MAX, cx));
        composer
            .subject
            .update(cx, |input, cx| input.set(draft.subject, usize::MAX, cx));
        // One field per line of the part the user wrote.
        // (The plain fields have no signature apart: a draft's stays in its text there.)
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
        // Saved with a signature of its own: it stays apart, as it was. A draft from
        // before (its signature in the text) or without one starts with none, so nothing
        // is added twice.
        let (html, signature) = crate::editor::split_signature(&html);
        let from = composer
            .account
            .clone()
            .unwrap_or_else(|| self.account_id.clone());
        composer.signature = signature.map(|html| {
            let html = crate::signature::clean(&html);
            let id = self
                .settings
                .signatures_of(&from)
                .find(|s| s.html == html)
                .map(|s| s.id.clone())
                .unwrap_or_default();
            (id, html)
        });
        composer.show_signature = true;
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
    pub(super) fn save_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(serial) = self.compose.as_ref().map(|c| c.serial) {
            self.save_composer(serial, true, cx);
        }
    }

    /// The composer with this serial, on screen or parked.
    pub(super) fn composer_mut(&mut self, serial: u64) -> Option<&mut Composer> {
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
    pub(super) fn save_composer(&mut self, serial: u64, explicit: bool, cx: &mut Context<Self>) {
        let from = self.composer_mut(serial).and_then(|c| c.account.clone());
        let client = self.client_for(from.as_deref());
        let Some(client) = client else {
            if explicit && let Some(composer) = self.composer_mut(serial) {
                composer.error = Some("Sample mode: drafts are not saved.".into());
                cx.notify();
            }
            return;
        };
        // Saved on purpose (⌘S, or closed to keep): in Drafts at once.
        let closing = self
            .parked
            .iter()
            .any(|(kind, c)| c.serial == serial && matches!(kind, Parked::Closing));
        if (explicit || closing) && self.composer_mut(serial).is_some_and(|c| !c.sending) {
            self.show_in_drafts(serial, cx);
        }
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
        let saved_as = composer.saved_as.clone();
        // The account it saves to, should the composer's From change on the way.
        let saved_in = from.clone().unwrap_or_else(|| self.account_id.clone());
        let saving_client = client.clone();
        cx.notify();
        let working = self.saves.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    let saved = client.save_draft(draft_id.as_deref(), &outgoing);
                    if let Ok(id) = &saved {
                        *saved_as.lock().unwrap() = Some((id.clone(), edits));
                    }
                    saved
                })
                .await;
            this.update(cx, |this, cx| {
                this.composer_saved(
                    serial,
                    edits,
                    explicit,
                    result,
                    (saved_in, saving_client),
                    cx,
                )
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
        (saved_in, saved_with): (String, Arc<mail::Client>),
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Err(error) if self.shown_signed_out(&error) => {
                self.compose = None;
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
        let shown = self.account_id.clone();
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        let now_from = composer.account.clone().unwrap_or(shown);
        let ok = match result {
            // Its From changed while this was saving: that draft is in the wrong account's
            // Drafts. Delete it there and save again from the account chosen.
            Ok(id) if now_from != saved_in => {
                std::thread::spawn(move || {
                    if let Err(error) = saved_with.delete_draft(&id)
                        && !mail::is_gone(&error)
                    {
                        eprintln!("nimbra: a draft left in the account it was moved from");
                    }
                });
                composer.resave = true;
                true
            }
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
        // Saved: its made-up row in Drafts goes with the next listing, which has the real
        // one (it shows until then).
        let made_up = if ok && composer.listed_new {
            composer.listed_as.clone().map(|id| (id, now_from.clone()))
        } else {
            None
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
                        let subject = composer.subject.read(cx).text();
                        let item = Scheduled {
                            draft_id,
                            at_ms: at,
                            subject,
                            account: composer.account.clone(),
                        };
                        self.add_scheduled(item, cx);
                    }
                    refresh = true;
                }
            }
            (Some(Parked::Closing), _) if !ok => self.unpark(serial, cx),
            (Some(Parked::Closing), _) if again => self.save_composer(serial, false, cx),
            (Some(Parked::Closing), _) => {
                if let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) {
                    let (_, composer) = self.parked.remove(pos);
                    // Its made-up row stays until Drafts is listed again: Edit draft there
                    // brings this composer back.
                    if let Some((id, _)) = &made_up {
                        self.closed_drafts.retain(|(i, _)| i != id);
                        self.closed_drafts.push((id.clone(), composer));
                        let excess = self.closed_drafts.len().saturating_sub(8);
                        self.closed_drafts.drain(..excess);
                    }
                }
                refresh = true;
            }
            (None, _) if again => self.save_composer(serial, false, cx),
            _ => {}
        }
        if let Some((id, account)) = made_up {
            self.pending.forget(View::Drafts, &account, &id);
        }
        // Drafts fetched ahead may be older than what was just saved.
        if ok {
            self.drafts_ready.clear();
        }
        if refresh && self.mailbox.base() == View::Drafts {
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Brings a parked composer back on screen (when nothing else is open there).
    pub(super) fn unpark(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) else {
            return;
        };
        if self.parked[pos].0 == Parked::Discarded
            && let Some(id) = self.parked[pos].1.listed_as.clone()
        {
            let account = self.parked[pos].1.account.clone();
            self.show_draft_row(account, &id, true, cx);
        }
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

    /// Esc, ⌘W or a window's close button on the composer: asks whether to keep what was
    /// written as a draft or throw it away. An empty composer just closes; asked again
    /// (Esc while asking), it goes back to writing.
    pub(super) fn request_close_compose(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        if self.compose_ask {
            // Hiding the editor (a native view with the keyboard) for the question makes
            // macOS send a cancel (Esc) straight after the click that asked: not an answer.
            if self
                .compose_ask_at
                .is_some_and(|at| at.elapsed() < Duration::from_millis(400))
            {
                return;
            }
            self.compose_ask = false;
            self.pending_focus = Some(self.editor_focus.clone());
        } else if composer.has_content(cx) {
            self.compose_ask = true;
            if let Some(composer) = &mut self.compose {
                composer.suggest = None;
            }
            self.sync_suggestions(cx);
            self.compose_ask_at = Some(std::time::Instant::now());
        } else {
            self.close_compose(cx);
            return;
        }
        cx.notify();
    }

    /// A window's close button or ⌘W: a composer with something in it, shown in that
    /// window, asks first (false keeps the window open).
    pub(super) fn may_close_window(
        &mut self,
        window: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> bool {
        let here = self.compose.is_some() && self.compose_host == Some(window);
        if here && self.compose.as_ref().is_some_and(|c| c.has_content(cx)) {
            if !self.compose_ask {
                self.compose_ask = true;
                if let Some(composer) = &mut self.compose {
                    composer.suggest = None;
                }
                self.sync_suggestions(cx);
                self.compose_ask_at = Some(std::time::Instant::now());
                cx.notify();
            }
            return false;
        }
        true
    }

    /// The answer to "Save this draft?".
    fn answer_close_compose(&mut self, save: bool, cx: &mut Context<Self>) {
        self.compose_ask = false;
        if save {
            self.close_compose(cx);
        } else {
            self.discard_compose(cx);
        }
    }

    /// Closes the composer. Its last edits are still saved to Drafts.
    pub(super) fn close_compose(&mut self, cx: &mut Context<Self>) {
        self.compose_ask = false;
        self.pending_focus = Some(self.focus.clone());
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        let unsaved = composer.saving || (composer.dirty() && composer.has_content(cx));
        if self.connection.is_some() && unsaved {
            let serial = composer.serial;
            self.parked.push((Parked::Closing, composer));
            self.save_composer(serial, false, cx);
        } else if composer.draft_id.is_some() && composer.listed_as.is_none() {
            // Saved already (as typing paused): in Drafts now all the same, its composer
            // kept for Edit draft on that row until Drafts is listed again.
            let serial = composer.serial;
            self.parked.push((Parked::Closing, composer));
            self.show_in_drafts(serial, cx);
            if let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) {
                let (_, composer) = self.parked.remove(pos);
                if let Some(id) = composer.listed_as.clone() {
                    self.closed_drafts.retain(|(i, _)| *i != id);
                    self.closed_drafts.push((id, composer));
                    let excess = self.closed_drafts.len().saturating_sub(8);
                    self.closed_drafts.drain(..excess);
                }
            }
        }
        cx.notify();
    }

    /// The Discard button: closes the composer and deletes its draft, unless undone.
    fn discard_compose(&mut self, cx: &mut Context<Self>) {
        self.compose_ask = false;
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
        // Out of Drafts now; Undo puts it back.
        if let Some(id) = composer.listed_as.clone() {
            let account = composer.account.clone();
            if composer.listed_new {
                let account = account.clone().unwrap_or_else(|| self.account_id.clone());
                self.pending.forget(View::Drafts, &account, &id);
            }
            self.show_draft_row(account, &id, false, cx);
        }
        self.parked.push((Parked::Discarded, composer));
        self.offer_undo("Draft discarded", cx);
        self.finish_parked_later(serial, UNDO_DELAY, cx);
        cx.notify();
    }

    pub(super) fn finish_parked_later(
        &mut self,
        serial: u64,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| this.finish_parked(serial, cx))
                .ok();
        })
        .detach();
    }

    /// The undo delay of a sent or discarded message is over: send it, or delete its draft.
    pub(super) fn finish_parked(&mut self, serial: u64, cx: &mut Context<Self>) {
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
                // On its way: Undo can't stop it any more, and the pill waits for "Sent".
                self.update_sending_toast(|toast| toast.can_undo = false, cx);
                self.dispatch(composer, cx);
            }
            Parked::Discarded => {
                let (_, composer) = self.parked.remove(pos);
                let (listed, account) = (composer.listed_as.clone(), composer.account.clone());
                if let (Some(id), Some(client)) = (
                    composer.draft_id.clone(),
                    self.client_for(composer.account.as_deref()),
                ) {
                    let working = self.writes.begin();
                    cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                let _working = working;
                                client.delete_draft(&id)
                            })
                            .await;
                        this.update(cx, |this, cx| {
                            // Already gone from the server: deleted all the same.
                            let result = match result {
                                Err(error) if mail::is_gone(&error) => Ok(()),
                                other => other,
                            };
                            match result {
                                Err(error) => {
                                    this.notice =
                                        Some(format!("The draft was not deleted: {error}").into());
                                    // Back in Drafts.
                                    if let Some(id) = &listed {
                                        this.show_draft_row(account.clone(), id, true, cx);
                                    }
                                }
                                Ok(()) => {
                                    if let Some(id) = &listed {
                                        for list in this.cache.values_mut() {
                                            list.retain(|m| m.id.as_deref() != Some(id.as_str()));
                                        }
                                    }
                                }
                            }
                            if this.mailbox.base() == View::Drafts {
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
        // Inbox Zero offers Z only until now.
        cx.notify();
    }

    /// A recipient field or the message changed.
    pub(super) fn field_changed(&mut self, field: Field, cx: &mut Context<Self>) {
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
                    // Typed in the field, or already a pill in To, Cc or Bcc.
                    let present: Vec<String> = text
                        .split([',', ';'])
                        .map(|p| p.trim().to_lowercase())
                        .chain(
                            self.compose
                                .iter()
                                .flat_map(|c| c.chips.iter().flatten())
                                .map(|chip| mail::address_of(chip).to_lowercase()),
                        )
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

    /// The people whose name, address or domain starts with `token`, best first: from
    /// what each account's connection learnt of the people it writes with (`contacts.rs`),
    /// the account the message is written from counting double, and from the mail open on
    /// screen. Never the user's own addresses, nor anyone already in the field.
    fn match_contacts(&self, token: &str, present: &[String]) -> Vec<mail::Person> {
        let from = self
            .compose
            .as_ref()
            .and_then(|c| c.account.clone())
            .unwrap_or_else(|| self.account_id.clone());
        let mine: Vec<String> = self
            .settings
            .accounts
            .iter()
            .map(|a| a.email.to_lowercase())
            .chain(std::iter::once(self.account.to_lowercase()))
            .filter(|a| !a.is_empty())
            .collect();
        let connections = self
            .connection
            .iter()
            .map(|c| (self.account_id.as_str(), c))
            .chain(self.others.iter().map(|(id, c)| (id.as_str(), c)));
        let books = connections
            .filter_map(|(account, client)| client.contacts().map(|book| (account, book.as_ref())));
        // The mail on screen (and the sample mail, which no connection has).
        let seen = self
            .mails
            .iter()
            .chain(self.cache.values().flatten())
            .flat_map(|mail| {
                std::iter::once(&mail.header).chain(mail.earlier.iter().map(|e| &e.header))
            })
            .flat_map(|header| {
                std::iter::once(&header.from)
                    .chain(&header.to)
                    .chain(&header.cc)
            });
        crate::contacts::rank(token, &from, books, seen, &mine, present)
    }

    /// Puts the chosen suggestion in place of what was being typed.
    pub(super) fn accept_suggestion(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
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
                        if total > mail::ATTACHMENT_LIMIT {
                            anyhow::bail!("Attachments can be 25 MB in total.");
                        }
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let bytes = std::fs::read(&path)?;
                        files.push(mail::OutgoingFile {
                            mime: mail::mime_for(&name).into(),
                            name: mail::safe_file_name(&name),
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

    /// The account the open composer writes from.
    pub(super) fn compose_from(&self) -> Option<String> {
        let composer = self.compose.as_ref()?;
        Some(
            composer
                .account
                .clone()
                .unwrap_or_else(|| self.account_id.clone()),
        )
    }

    /// The composer's From can change: with more than one account. A reply too: it goes
    /// from its thread's account unless changed (see `Composer::reply_home`).
    fn can_change_from(&self) -> bool {
        !self.is_demo() && self.settings.accounts.len() >= 2 && self.compose.is_some()
    }

    /// The From row shows: where it can change, and in the sample (with its one address),
    /// so the sample looks like the real thing.
    fn shows_from(&self) -> bool {
        self.can_change_from() || (self.is_demo() && self.compose.is_some())
    }

    /// The composer's From changes. A draft it already saved is deleted from the old
    /// account's Drafts and saved again in the new one's.
    ///
    /// Files it has from Gmail (a forward's, a draft's) are in the old account, which the
    /// new one cannot read: they are read first, and From changes once the message holds
    /// them itself.
    pub(super) fn set_compose_account(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.can_change_from() {
            return;
        }
        let from = self.compose_from();
        let Some(composer) = &mut self.compose else {
            return;
        };
        if from.as_deref() == Some(id.as_str()) {
            // Back to the account it is in: a change under way is called off.
            if composer.moving_to.take().is_some() {
                cx.notify();
            }
            return;
        }
        if self.client_for(Some(&id)).is_none() {
            self.notice = Some("That account is not connected yet. Try again in a moment.".into());
            cx.notify();
            return;
        }
        let old_client = self.client_for(from.as_deref());
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.kept.is_empty() {
            self.switch_compose_account(id, cx);
            return;
        }
        let Some(client) = old_client else {
            return;
        };
        // Already reading them (another account was picked a moment ago): only the
        // account to change to is new.
        if composer.moving_to.replace(id).is_some() {
            cx.notify();
            return;
        }
        let (serial, kept) = (composer.serial, composer.kept.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    kept.into_iter()
                        .map(|att| {
                            let bytes = client.download(&att.message_id, &att.attachment_id)?;
                            let file = mail::OutgoingFile {
                                name: att.name.clone(),
                                mime: att.mime.clone(),
                                bytes,
                            };
                            Ok((att, file))
                        })
                        .collect::<anyhow::Result<Vec<_>>>()
                })
                .await;
            this.update(cx, |this, cx| this.attachments_read(serial, read, cx))
                .ok();
        })
        .detach();
    }

    /// The files a composer had in Gmail were read (or not) so that its From can change.
    fn attachments_read(
        &mut self,
        serial: u64,
        read: anyhow::Result<Vec<(mail::Attachment, mail::OutgoingFile)>>,
        cx: &mut Context<Self>,
    ) {
        // Closed or sent meanwhile: it stays in the account it was in.
        let Some(composer) = self.compose.as_mut().filter(|c| c.serial == serial) else {
            return;
        };
        match read {
            Ok(read) => {
                take_over(&mut composer.kept, &mut composer.files, read);
                // Called off meanwhile: the files are the message's own now, which is as good.
                if composer.kept.is_empty()
                    && let Some(id) = composer.moving_to.take()
                {
                    self.switch_compose_account(id, cx);
                }
            }
            Err(error) => {
                composer.moving_to = None;
                self.notice = Some(
                    format!("From was not changed: the attachments could not be read ({error}).")
                        .into(),
                );
            }
        }
        cx.notify();
    }

    /// From changes to `id`, for a composer with nothing left in the old account but its
    /// saved draft.
    fn switch_compose_account(&mut self, id: String, cx: &mut Context<Self>) {
        if self.client_for(Some(&id)).is_none() {
            self.notice = Some("That account is not connected yet. Try again in a moment.".into());
            cx.notify();
            return;
        }
        let old = self.compose_from();
        let old_client = self.client_for(old.as_deref());
        let Some(composer) = &mut self.compose else {
            return;
        };
        composer.account = Some(id.clone());
        // A reply's thread is its own account's: from another, it goes without it (its
        // reply headers still thread it for whoever gets it), and back home it has it again.
        if let Some(reply) = &mut composer.reply {
            if composer.reply_home.is_none()
                && let Some(home) = old.clone()
                && !reply.thread_id.is_empty()
            {
                composer.reply_home = Some((home, reply.thread_id.clone()));
            }
            reply.thread_id = match &composer.reply_home {
                Some((home, thread)) if *home == id => thread.clone(),
                Some(_) => String::new(),
                None => reply.thread_id.clone(),
            };
        }
        // A message from another account has that account's signature.
        let signature = self.default_signature_for(&id);
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.signature != signature {
            composer.signature = signature;
            composer.show_signature = true;
            composer.edits += 1;
            composer.saved = false;
        }
        // What quitting would keep belongs to the old account: start afresh.
        composer.saved_as = SavedAs::default();
        let serial = composer.serial;
        let moved = composer.draft_id.take();
        if let (Some(draft), Some(client)) = (moved.clone(), old_client) {
            std::thread::spawn(move || {
                if let Err(error) = client.delete_draft(&draft)
                    && !mail::is_gone(&error)
                {
                    eprintln!("nimbra: a draft left in the account it was moved from");
                }
            });
        }
        if moved.is_some() || composer.saving {
            // Unsaved in the new account: it counts as an edit, so it is saved there.
            composer.edits += 1;
            composer.saved = false;
            self.save_composer(serial, false, cx);
        }
        cx.notify();
    }

    /// Who a new message is from: the account on screen, or in a view of every account
    /// (All inboxes and the like) the default account.
    fn new_message_account(&self) -> Option<String> {
        if self.is_demo() || self.settings.accounts.len() < 2 {
            return None;
        }
        let id = if matches!(self.mailbox, View::Every(_)) {
            self.settings.default_account()?.to_string()
        } else {
            self.account_id.clone()
        };
        self.client_for(Some(&id)).map(|_| id)
    }

    pub(super) fn compose_view(
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
                d.w_full().max_w(dp(860.))
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
                        .gap(dp(12.))
                        .px(dp(20.))
                        .py(dp(10.))
                        .child(
                            div()
                                .w(dp(56.))
                                .flex_none()
                                .text_color(t.text3)
                                .child(label.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(dp(0.))
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(dp(6.))
                                .text_size(dp(14.))
                                .children(lead)
                                .child(div().flex_1().min_w(dp(120.)).child(input.clone())),
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
                        .gap(dp(4.))
                        .pl(dp(9.))
                        .pr(dp(3.))
                        .py(dp(1.))
                        .rounded_full()
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border_strong)
                        .text_size(dp(13.))
                        .child(short(address, 34))
                        .child(
                            div()
                                .id(("remove-recipient", slot * 1000 + i))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(dp(16.))
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
        // Address suggestions float over the fields below, in a window of their own (see
        // `suggest_popup`). Under the field, only an anchor that tells it where to be.
        let mut dropdown: Option<(Field, AnyElement)> = composer.suggest.as_ref().map(|s| {
            let owner = cx.weak_entity();
            let anchor = gpui::canvas(
                move |bounds, window, cx| {
                    let at = crate::blur::screen_point(
                        window,
                        from_screen(bounds.origin.x),
                        from_screen(bounds.origin.y),
                    );
                    let composer_window = window.window_handle();
                    if let Some(at) = at {
                        cx.defer(move |cx| {
                            owner
                                .update(cx, |this, cx| {
                                    this.place_suggestions(at, composer_window, cx)
                                })
                                .ok();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .w(dp(suggest_popup::SUGGEST_WIDTH))
            .h(dp(1.));
            // Takes no room: the fields below stay where they are.
            let strip = div()
                .h(dp(0.))
                .flex()
                .justify_center()
                .child(column(div()).pl(dp(84.)).pr(dp(20.)).child(anchor));
            (s.field, strip.into_any_element())
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
                .gap(dp(6.))
                .pl(dp(10.))
                .pr(dp(4.))
                .py(dp(4.))
                .rounded(dp(8.))
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
                        .size(dp(18.))
                        .rounded(dp(5.))
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
        let title = if composer.draft_id.is_some() {
            Some("Draft")
        } else if composer.reply.is_some() {
            Some("Reply")
        } else {
            None
        };
        // What it is, and the buttons for the card. Floating, they end its header row; in
        // full page they sit in the window's top right corner, level with the traffic lights.
        let controls = div()
            .flex()
            .items_center()
            .gap(dp(8.))
            // What it is (a draft being edited, or a reply), as a pill before
            // the buttons. A new message has none.
            .children(title.map(|title| pill(t, title, accent).mr(dp(4.))))
            .when(!self.compose_detached, |d| {
                d.child(
                    icon_button(t, "detach")
                        .id("compose-detach")
                        .press(t)
                        .tooltip(tip(t.dark, "Open in its own window", ""))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.detach_compose(cx)
                            }),
                        ),
                )
            })
            .when(!self.compose_detached, |d| {
                d.child(
                    icon_button(t, if full { "shrink" } else { "expand" })
                        .id("compose-size")
                        .press(t)
                        .tooltip(tip(t.dark, "Full page or floating", "⌘⇧F"))
                        // On press, as the close button (see there).
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_full_compose(cx)
                            }),
                        ),
                )
            })
            // Close (asks about the draft if there is one). A window of
            // its own has its close button in the title bar instead.
            .when(!self.compose_detached, |d| {
                d.child(
                    icon_button(t, "x")
                        .id("compose-close")
                        .press(t)
                        .tooltip(tip(t.dark, "Close", "esc"))
                        // On press, not release: a press that takes the
                        // keyboard from the editor (a native view) can lose
                        // its release, which made a click need doing twice.
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.request_close_compose(cx)
                            }),
                        ),
                )
            })
            .when(self.compose_detached, |d| {
                d.child(
                    icon_button(t, "attach")
                        .id("compose-attach")
                        .press(t)
                        .tooltip(tip(t.dark, "Back into the window", ""))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.attach_compose(cx)
                            }),
                        ),
                )
                .child(kbd(t, "esc"))
            });
        let (inline_controls, corner_controls) = if full {
            (None, Some(controls))
        } else {
            (Some(controls), None)
        };
        let card = div()
            .id("compose-card")
            .flex()
            .flex_col()
            .bg(t.raised)
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.attach_files(paths.paths().to_vec(), cx)
            }))
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(accent))
            .when(full, |d| d.relative().size_full().bg(t.surface))
            .when(!full, |d| {
                d.absolute()
                    .left(dp(left))
                    .top(dp(top))
                    .w(dp(width))
                    .h(dp(height))
                    .rounded(dp(18.))
                    .border_1()
                    // The app's own faint line: its shadow sets it apart from the mail.
                    .border_color(t.border)
                    // A plain shadow: no coloured glow spilling over the mail behind.
                    .shadow(vec![shadow(
                        c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                        30.,
                        80.,
                        -10.,
                    )])
            })
            .child(
                div()
                    .flex()
                    .justify_center()
                    // In full page the header also clears the traffic lights and is the
                    // strip you drag or double-click.
                    .when(full, |d| {
                        d.pt(dp(36.)).on_mouse_down(MouseButton::Left, drag_or_zoom)
                    })
                    .when(!full, |d| {
                        d.cursor(gpui::CursorStyle::OpenHand).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event, _, cx| {
                                this.grab_composer(Grab::Move, event, cx)
                            }),
                        )
                    })
                    // A field of its own when From shows, divided from To as the fields are.
                    .when(self.shows_from(), |d| d.border_b_1().border_color(t.border))
                    .child(
                        column(div())
                            .flex()
                            .items_center()
                            .gap(dp(8.))
                            .px(dp(20.))
                            // The same above and below, so the buttons sit in the middle,
                            // level with From.
                            .py(dp(8.))
                            // A new message: who it is from, and a menu to change it.
                            .when(self.shows_from(), |d| {
                                // The account it is changing to shows at once, with what
                                // it is waiting for.
                                let moving = composer.moving_to.is_some();
                                let email = composer
                                    .moving_to
                                    .clone()
                                    .or_else(|| self.compose_from())
                                    .and_then(|id| {
                                        self.signature_accounts()
                                            .into_iter()
                                            .find(|(account, _)| *account == id)
                                    })
                                    .map(|(_, email)| email)
                                    .unwrap_or_default();
                                let photo = composer
                                    .moving_to
                                    .clone()
                                    .or_else(|| self.compose_from())
                                    .and_then(|id| self.photos.get(&id).cloned());
                                let face = menus::account_face(&email, photo.as_ref(), 20.);
                                let anchor = self.from_anchor.clone();
                                let hover = t.hover;
                                // The label is the fields' label column (56 wide, 12 to the
                                // value) and only the photo and address are the button. This
                                // row's gap is 8, and the button's padding takes the other 4,
                                // so the photo starts where the fields' values do.
                                d.child(
                                    div()
                                        .w(dp(56.))
                                        .flex_none()
                                        .text_color(t.text3)
                                        .child("From"),
                                )
                                .child(
                                    div()
                                        .id("compose-from")
                                        .flex()
                                        .items_center()
                                        .relative()
                                        .gap(dp(8.))
                                        .min_w(dp(0.))
                                        .pl(dp(menus::FROM_PHOTO_INSET))
                                        .pr(dp(8.))
                                        .py(dp(3.))
                                        .rounded(dp(7.))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(hover))
                                        .when(self.menu == Some(Menu::From), |d| d.bg(t.selected))
                                        .tooltip(tip(t.dark, "Send from another account", ""))
                                        .child(face)
                                        .child(
                                            div()
                                                .min_w(dp(0.))
                                                .truncate()
                                                .text_size(dp(14.))
                                                .text_color(t.text2)
                                                .child(email),
                                        )
                                        .child(icon("down", 12., t.text3))
                                        // Where it is, for its menu to open under it.
                                        .child(
                                            gpui::canvas(
                                                move |bounds, _, _| {
                                                    anchor.set((
                                                        from_screen(bounds.origin.x),
                                                        from_screen(
                                                            bounds.origin.y + bounds.size.height,
                                                        ),
                                                    ));
                                                },
                                                |_, _, _, _| {},
                                            )
                                            .absolute()
                                            .inset_0(),
                                        )
                                        // On press, as the header's other buttons (it is also
                                        // where the card is dragged from).
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.toggle_menu(Menu::From, window, cx)
                                            }),
                                        ),
                                )
                                .when(moving, |d| {
                                    d.child(meta("Moving the attachments…".to_string(), t.text3))
                                })
                            })
                            // An answer written from another account than the one shown
                            // (All inboxes).
                            .when(!self.can_change_from(), |d| {
                                d.children(
                                    composer
                                        .account
                                        .as_deref()
                                        .filter(|a| *a != self.account_id)
                                        .and_then(|id| {
                                            self.settings.accounts.iter().find(|a| a.id == id)
                                        })
                                        .map(|a| meta(format!("from {}", a.email), t.text3)),
                                )
                            })
                            .child(div().flex_1())
                            // Their icons end where the fields' text does (Cc Bcc): an icon
                            // button's box reaches 7 past its icon (30 around 16).
                            .children(inline_controls.map(|c| c.mr(dp(-7.)))),
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
                                .gap(dp(8.))
                                .px(dp(20.))
                                .py(dp(10.))
                                .children(chips),
                        ),
                )
            })
            .when(composer.rich, |d| {
                let editor = self.editor.clone();
                let origin = self.editor_origin.clone();
                let tool = |id: usize,
                            name: &'static str,
                            label: &'static str,
                            key: &'static str,
                            command: &'static str,
                            active: bool| {
                    let hover = t.hover;
                    div()
                        .id(("format", id))
                        .press(t)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(dp(28.))
                        .rounded(dp(7.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .when(active, |d| d.bg(t.hover).border_1().border_color(t.border))
                        .child(icon(name, 15., if active { t.text } else { t.text2 }))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.format_hint = Some((label, key));
                            } else if this.format_hint.is_some_and(|(l, _)| l == label) {
                                this.format_hint = None;
                            }
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &this.editor {
                                editor.exec(command);
                            }
                            this.pending_focus = Some(this.editor_focus.clone());
                            cx.notify();
                        }))
                };
                let f = composer.format;
                let gap = || div().w(dp(1.)).h(dp(16.)).mx(dp(4.)).bg(t.border);
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
                                .gap(dp(2.))
                                .px(dp(14.))
                                .py(dp(5.))
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
                                ))
                                .child(div().flex_1())
                                .when_some(self.format_hint, |d, (label, key)| {
                                    d.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(dp(8.))
                                            .text_size(dp(12.))
                                            .text_color(t.text2)
                                            .child(label)
                                            .when(!key.is_empty(), |d| d.child(kbd(t, key))),
                                    )
                                }),
                        ),
                )
                // The editor is a native web view: this element only reserves its space and
                // tells it where that space is each frame.
                .child(
                    div()
                        .id("compose-body")
                        .flex_1()
                        .min_h(dp(0.))
                        .key_context("Editor")
                        .track_focus(&self.editor_focus)
                        .child(
                            gpui::canvas(
                                move |bounds, _, _| {
                                    // Floating, the editor (a native view) leaves the card's
                                    // side edges free for resizing.
                                    let inset = if full { 0. } else { 6. };
                                    origin.set((
                                        f32::from(bounds.origin.x) + inset,
                                        f32::from(bounds.origin.y),
                                    ));
                                    if let Some(editor) = &editor {
                                        editor.set_bounds(
                                            f32::from(bounds.origin.x) + inset,
                                            f32::from(bounds.origin.y),
                                            f32::from(bounds.size.width) - 2. * inset,
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
                        .min_h(dp(0.))
                        .overflow_y_scroll()
                        .child(
                            column(div())
                                .flex()
                                .flex_col()
                                .px(dp(20.))
                                .py(dp(if full { 24. } else { 14. }))
                                .text_size(dp(15.))
                                .children(composer.lines.iter().cloned())
                                // The quoted earlier message stays folded behind "…".
                                .when_some(composer.quoted.clone(), |d, quoted| {
                                    d.child(
                                        div().flex().pt(dp(10.)).child(
                                            div()
                                                .id("toggle-quoted")
                                                .px(dp(8.))
                                                .rounded(dp(6.))
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
                                                    .mt(dp(10.))
                                                    .pl(dp(14.))
                                                    .border_l_2()
                                                    .border_color(t.border_strong)
                                                    .text_size(dp(14.))
                                                    .line_height(dp(22.))
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
                            .gap(dp(8.))
                            .px(dp(14.))
                            .py(dp(12.))
                            .child(
                                icon_button(t, "clip")
                                    .id("attach")
                                    .press(t)
                                    .tooltip(tip(t.dark, "Attach files", ""))
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(dp(0.))
                                    .pl(dp(4.))
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
                                        None if self.connection.is_none() => {
                                            "Sample mode: nothing is sent.".into()
                                        }
                                        None => "Drop files here to attach them.".into(),
                                    }),
                            )
                            .child(
                                icon_button(t, "trash")
                                    .id("discard")
                                    .press(t)
                                    .tooltip(tip(t.dark, "Discard draft", ""))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.discard_compose(cx)),
                                    ),
                            )
                            .child(
                                button(t, ButtonKind::Secondary, "Send later", None)
                                    .id("send-later")
                                    .press_soft()
                                    .tooltip(tip(
                                        t.dark,
                                        "Sent from this Mac while Nimbra is running",
                                        "",
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_menu(Menu::SendLater, window, cx)
                                    })),
                            )
                            .child(crate::ui::glow(
                                button(t, ButtonKind::Primary, "Send", Some("⌘↵"))
                                    .id("send")
                                    .press_soft()
                                    .when(composer.sending, |d| d.opacity(0.5))
                                    .on_click(cx.listener(|this, _, _, cx| this.send_mail(cx))),
                                "send",
                                true,
                            )),
                    ),
            )
            // Last, so they are above the header strip that moves the window. The traffic
            // lights' middle is 25 pt down (18 to their top, 14 tall); the 30 pt buttons are
            // centred on it.
            .children(
                corner_controls.map(|controls| controls.absolute().top(dp(10.)).right(dp(16.))),
            )
            // Every edge and corner resizes the floating card (no visible grip).
            .when(!full, |d| {
                use gpui::CursorStyle as C;
                // Each handle reaches a little past the card (where the mouse also comes to
                // the app over a message window's email) and a little inside it.
                const EDGE: f32 = 6.;
                const CORNER: f32 = 16.;
                const OUT: f32 = 8.;
                let handles: [(i8, i8, C); 8] = [
                    (-1, -1, C::ResizeUpLeftDownRight),
                    (1, 1, C::ResizeUpLeftDownRight),
                    (1, -1, C::ResizeUpRightDownLeft),
                    (-1, 1, C::ResizeUpRightDownLeft),
                    (0, -1, C::ResizeUpDown),
                    (0, 1, C::ResizeUpDown),
                    (-1, 0, C::ResizeLeftRight),
                    (1, 0, C::ResizeLeftRight),
                ];
                // Edges first, so the corners sit on top of them.
                d.children(handles.into_iter().rev().map(|(hx, hy, cursor)| {
                    let corner = hx != 0 && hy != 0;
                    let mut h = div().absolute().cursor(cursor);
                    h = match hx {
                        -1 => h.left(dp(-OUT)),
                        1 => h.right(dp(-OUT)),
                        _ => h.left(dp(CORNER)).right(dp(CORNER)),
                    };
                    h = match hy {
                        -1 => h.top(dp(-OUT)),
                        1 => h.bottom(dp(-OUT)),
                        _ => h.top(dp(CORNER)).bottom(dp(CORNER)),
                    };
                    if corner {
                        h = h.size(dp(CORNER + OUT));
                    } else if hx == 0 {
                        h = h.h(dp(EDGE + OUT));
                    } else {
                        h = h.w(dp(EDGE + OUT));
                    }
                    h.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, _, cx| {
                            cx.stop_propagation();
                            this.grab_composer(Grab::Resize(hx, hy), event, cx)
                        }),
                    )
                }))
            })
            // "Save this draft?" over the composer.
            .when(self.compose_ask, |d| {
                d.child(
                    div()
                        .id("compose-ask")
                        .absolute()
                        // Over the composer's border too, which would stay bright around the
                        // dimmed composer otherwise.
                        .top(dp(-1.))
                        .left(dp(-1.))
                        .right(dp(-1.))
                        .bottom(dp(-1.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!full, |d| d.rounded(dp(18.)))
                        .bg(t.scrim)
                        .occlude()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(dp(14.))
                                .w(dp(340.))
                                .p(dp(22.))
                                .rounded(dp(16.))
                                .bg(t.raised)
                                .border_1()
                                .border_color(t.border)
                                .shadow(vec![shadow(c(0x000000, 0.35), 18., 50., -8.)])
                                .child(serif("Save this draft?", 24., t.text))
                                .child(meta(
                                    "Keep it in Drafts to finish later, or discard what you wrote.",
                                    t.text2,
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(dp(8.))
                                        .pt(dp(4.))
                                        .child(
                                            button(t, ButtonKind::Ghost, "Discard", None)
                                                .id("ask-discard")
                                                .press_soft()
                                                .text_color(t.rose)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.answer_close_compose(false, cx)
                                                })),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            button(t, ButtonKind::Secondary, "Cancel", Some("esc"))
                                                .id("ask-cancel")
                                                .press_soft()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.request_close_compose(cx)
                                                })),
                                        )
                                        .child(crate::ui::glow(
                                            button(t, ButtonKind::Primary, "Save draft", None)
                                                .id("ask-save")
                                                .press_soft()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.answer_close_compose(true, cx)
                                                })),
                                            "ask-save",
                                            true,
                                        )),
                                ),
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
            // Dimmed in the main window; over a message window the email stays readable.
            .when(!full && self.compose_host.is_none(), |d| d.bg(t.scrim))
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
            // Over a message window the email behind can still be scrolled.
            .when(self.compose_host.is_none(), |d| d.occlude())
            .child(card)
            .into_any_element()
    }
}

/// A window holding only the composer (the composer's detach button).
pub(super) struct ComposeWindow {
    pub(super) owner: Entity<Nimbra>,
    /// Has shown the composer (until then it may not be registered as its window yet).
    pub(super) shown: bool,
    pub(super) _observe: gpui::Subscription,
}

impl Render for ComposeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let owner = self.owner.clone();
        let content = owner.update(cx, |n, cx| n.hosted_compose(window, cx));
        let t = owner.read(cx).theme();
        if content.is_some() {
            self.shown = true;
        }
        if content.is_none() && self.shown {
            // Sent, closed or discarded: the window goes with it.
            window.defer(cx, |window, _| window.remove_window());
        } else if let Some(composer) = &owner.read(cx).compose {
            let title = if composer.reply.is_some() {
                "Reply"
            } else {
                "New message"
            };
            window.set_window_title(title);
        }
        div()
            .relative()
            .size_full()
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            // ⌘W: asks about the draft (the window closes with the composer).
            .on_action(cx.listener(|this, _: &HideWindow, window, cx| {
                let handle = window.window_handle();
                this.owner.update(cx, |n, cx| {
                    if n.may_close_window(handle, cx) {
                        n.close_compose(cx);
                    }
                });
            }))
            .children(content)
    }
}

/// Files read from Gmail become the message's own: each one still in `kept` (not taken off
/// while it was read) moves to `files`.
fn take_over(
    kept: &mut Vec<mail::Attachment>,
    files: &mut Vec<mail::OutgoingFile>,
    read: Vec<(mail::Attachment, mail::OutgoingFile)>,
) {
    for (attachment, file) in read {
        if let Some(at) = kept.iter().position(|k| *k == attachment) {
            kept.remove(at);
            files.push(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str) -> mail::Attachment {
        mail::Attachment {
            message_id: "m1".into(),
            attachment_id: format!("a-{name}"),
            name: name.into(),
            mime: "text/plain".into(),
            size: 3,
        }
    }

    fn read(name: &str) -> (mail::Attachment, mail::OutgoingFile) {
        let file = mail::OutgoingFile {
            name: name.into(),
            mime: "text/plain".into(),
            bytes: name.as_bytes().to_vec(),
        };
        (attachment(name), file)
    }

    #[test]
    fn files_read_from_gmail_become_the_messages_own() {
        let mut kept = vec![attachment("a"), attachment("b")];
        let mut files = Vec::new();
        take_over(&mut kept, &mut files, vec![read("a"), read("b")]);
        assert!(kept.is_empty());
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn a_file_taken_off_while_it_was_read_stays_off() {
        let mut kept = vec![attachment("b")];
        let mut files = Vec::new();
        take_over(&mut kept, &mut files, vec![read("a"), read("b")]);
        assert!(kept.is_empty());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "b");
    }

    #[test]
    fn reading_them_twice_attaches_them_once() {
        let mut kept = vec![attachment("a")];
        let mut files = Vec::new();
        take_over(&mut kept, &mut files, vec![read("a")]);
        take_over(&mut kept, &mut files, vec![read("a")]);
        assert_eq!(files.len(), 1);
    }
}
