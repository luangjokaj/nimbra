//! The composer: writing a message (new, reply, forward or a draft), floating over the mail,
//! full page or in a window of its own. Its state, its fields and editor, saving it as a draft,
//! and how it is drawn. Sending is in `outgoing.rs`.

use super::*;

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
    /// Content still to be put into the editor.
    pub(super) load: Option<String>,
    /// Bold, italic, underline, bullets, numbers at the cursor.
    pub(super) format: [bool; 5],
    /// Put the keyboard in the message body once the editor shows it.
    pub(super) focus_body: bool,
    pub(super) subject: Entity<TextInput>,
    pub(super) lines: Vec<Entity<TextInput>>,
    pub(super) reply: Option<mail::ReplyTo>,
    /// The Gmail draft being edited, if this message was saved or opened from Drafts.
    pub(super) draft_id: Option<String>,
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
    pub(super) grab: Option<(Grab, (f32, f32), (f32, f32), (f32, f32))>,
    pub(super) sending: bool,
    pub(super) error: Option<SharedString>,
    /// A draft opened from Drafts, still on its way: shown at once, filled when it lands.
    pub(super) loading: bool,
    /// The draft save that follows a pause in typing; each edit replaces it.
    pub(super) autosave: Option<gpui::Task<()>>,
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

    /// The message as it would be saved or sent.
    pub(super) fn outgoing(&self, cx: &App) -> mail::Outgoing {
        let html = self
            .rich
            .then(|| crate::editor::message_html(&self.html, self.quoted.as_deref()));
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
                .into_iter()
                .chain(&self.lines)
                .any(|input| !input.read(cx).text().trim().is_empty())
    }

    pub(super) fn dirty(&self) -> bool {
        self.edits != self.saved_edits
    }

    /// The whole message text: the edited lines, then the quoted earlier message.
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
        if self.compose.is_some() {
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
            // A new message is from the default account; an answer from its message's.
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
            loading: false,
            autosave: None,
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
                // Its files go along: read from Gmail when it is saved or sent, and shown
                // as chips, each with a button to leave it out.
                composer.kept = header.attachments.clone();
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
                if self.settings.quote_replies {
                    composer.quoted = Some(quoted.join("\n").trim_end().to_string());
                }
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
        let Some(serial) = self
            .compose
            .as_ref()
            .filter(|c| c.rich && !c.loading)
            .map(|c| c.serial)
        else {
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
                "--raised": css(t.raised),
                "--text": css(t.text),
                "--dim": css(t.text3),
                "--faint": css(t.faint),
                "--accent": css(t.violet),
                "--line": css(t.border_strong),
                "--hairline": css(t.border),
                "--edge": css(t.edge),
                "--hover": css(t.hover),
                "--pressed": css(t.pressed),
                "--selection": css(t.selected),
                "--shadow": css(t.shadow),
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
            EditorEvent::Escape => self.request_close_compose(cx),
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
        let viewport = (f32::from(size.width), f32::from(size.height));
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
                .on_action(cx.listener(|this, _: &SendMail, _, cx| {
                    if this.compose_ready() {
                        this.send_mail(cx)
                    }
                }))
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
            let bounds = gpui::Bounds::centered(None, gpui::size(px(820.), px(700.)), cx);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("New Message".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(18.), px(18.))),
                    }),
                    window_min_size: Some(gpui::size(px(520.), px(420.))),
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

    /// The composer can take Send and Save: it is open and holds its message.
    fn compose_ready(&self) -> bool {
        self.compose.as_ref().is_some_and(|c| !c.loading)
    }

    /// Opens the draft behind a row of the Drafts mailbox: the composer shows at once and
    /// the draft fills it when it arrives from the row's own account.
    pub(super) fn open_draft(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.compose.is_some() {
            return;
        }
        let Some(mail) = self.mails.get(index) else {
            return;
        };
        let Some(thread_id) = mail.id.clone() else {
            return;
        };
        let account = mail
            .account
            .clone()
            .unwrap_or_else(|| self.account_id.clone());
        let Some(client) = self.client_for(Some(&account)) else {
            return;
        };
        self.notice = None;
        let Some(serial) = self.begin_draft(Some(account), cx) else {
            return;
        };
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.draft_for_thread(&thread_id) })
                .await;
            this.update(cx, |this, cx| {
                this.draft_arrived(serial, generation, result, cx)
            })
            .ok();
        })
        .detach();
    }

    fn draft_arrived(
        &mut self,
        serial: u64,
        generation: u64,
        result: anyhow::Result<mail::Draft>,
        cx: &mut Context<Self>,
    ) {
        let waiting = self
            .compose
            .as_ref()
            .is_some_and(|c| c.serial == serial && c.loading);
        if !waiting {
            return;
        }
        match result {
            Ok(draft) if self.account_gen == generation => self.fill_draft(draft, cx),
            outcome => {
                self.compose = None;
                self.pending_focus = Some(self.focus.clone());
                match outcome {
                    Err(error) if self.shown_signed_out(&error) => self.fail(error, cx),
                    Err(error) => {
                        eprintln!("nimbra: a draft did not open: {error}");
                        self.notice = Some(if error.is::<mail::Offline>() {
                            "Couldn’t open the draft while offline.".into()
                        } else {
                            "Couldn’t open the draft. Try again.".into()
                        });
                    }
                    // The account changed while it loaded.
                    Ok(_) => {}
                }
            }
        }
        cx.notify();
    }

    /// A draft in the composer at once (the sample's, or one already read).
    pub(super) fn edit_draft(&mut self, draft: mail::Draft, cx: &mut Context<Self>) {
        if self.begin_draft(None, cx).is_some() {
            self.fill_draft(draft, cx);
        }
    }

    /// The full page composer for a draft, waiting for its content. `account` is the one
    /// the draft is in (`None`: the one shown).
    fn begin_draft(&mut self, account: Option<String>, cx: &mut Context<Self>) -> Option<u64> {
        self.leave_sidebar(cx);
        self.open_compose_from(ComposeKind::New, None, cx);
        let composer = self.compose.as_mut()?;
        composer.loading = true;
        composer.full = true;
        composer.account = account;
        composer.load = None;
        cx.notify();
        Some(composer.serial)
    }

    fn fill_draft(&mut self, draft: mail::Draft, cx: &mut Context<Self>) {
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.loading = false;
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
        composer.autosave = Some(cx.spawn(async move |this, cx| {
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
        }));
    }

    /// Saves a composer's message as a Gmail draft (new, or over the one it has).
    /// `explicit` is ⌘S or a step that needs the draft (Send later); otherwise only unsaved
    /// edits with something in them are saved.
    pub(super) fn save_composer(&mut self, serial: u64, explicit: bool, cx: &mut Context<Self>) {
        let from = self.composer_mut(serial).and_then(|c| c.account.clone());
        let client = self.client_for(from.as_deref());
        let Some(client) = client else {
            if explicit && let Some(composer) = self.composer_mut(serial) {
                composer.error = Some("Drafts aren’t saved in sample mode.".into());
                cx.notify();
            }
            return;
        };
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        if composer.sending || composer.loading {
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
                composer.resave = true;
                self.forget_moved_draft(saved_with, id, cx);
                true
            }
            Ok(id) => {
                composer.draft_id = Some(id);
                composer.saved_edits = composer.saved_edits.max(edits);
                composer.saved = !composer.dirty();
                true
            }
            Err(error) => {
                eprintln!("nimbra: a draft was not saved: {error}");
                composer.error = Some(if error.is::<mail::Offline>() {
                    "Couldn’t save the draft while offline.".into()
                } else {
                    "Couldn’t save the draft. Try ⌘S again.".into()
                });
                composer.after_save = None;
                false
            }
        };
        let Some(composer) = self.composer_mut(serial) else {
            return;
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
                            out: false,
                        };
                        self.add_scheduled(item, cx);
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
    pub(super) fn unpark(&mut self, serial: u64, cx: &mut Context<Self>) {
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
                self.dispatch(composer, cx);
            }
            Parked::Discarded => {
                let (_, composer) = self.parked.remove(pos);
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
                            if let Err(error) = result
                                && !mail::is_gone(&error)
                            {
                                eprintln!("nimbra: a discarded draft was not deleted: {error}");
                                this.notice =
                                    Some("Couldn’t delete the draft. It’s still in Drafts.".into());
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
        // Inbox Zero offers Z only until now.
        cx.notify();
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
    fn match_contacts(&self, token: &str, present: &[String]) -> Vec<mail::Person> {
        let me = self.account.to_lowercase();
        let mut seen: HashMap<String, (mail::Person, u32)> = HashMap::new();
        let mut add = |p: &mail::Person| {
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
        let mut found: Vec<(mail::Person, u32)> = seen
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
                    const UNREADABLE: &str = "Couldn’t read that file.";
                    for path in paths.into_iter().take(20) {
                        let meta = std::fs::metadata(&path).map_err(|_| UNREADABLE)?;
                        if !meta.is_file() {
                            return Err("Folders can’t be attached. Zip them first.");
                        }
                        total += meta.len();
                        if total > mail::ATTACHMENT_LIMIT {
                            return Err("Attachments can be 25 MB in total.");
                        }
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let bytes = std::fs::read(&path).map_err(|_| UNREADABLE)?;
                        files.push(mail::OutgoingFile {
                            mime: mail::mime_for(&name).into(),
                            name: mail::safe_file_name(&name),
                            bytes: bytes.into(),
                        });
                    }
                    Ok(files)
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
                    Err(error) => composer.error = Some(error.into()),
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

    /// The composer's From can change: a new message or a forward (not a reply, which
    /// belongs to its thread's account), with more than one account.
    fn can_change_from(&self) -> bool {
        !self.is_demo()
            && self.settings.accounts.len() >= 2
            && self.compose.as_ref().is_some_and(|c| c.reply.is_none())
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
                                bytes: bytes.into(),
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
                eprintln!("nimbra: attachments were not read for a From change: {error}");
                composer.moving_to = None;
                self.notice =
                    Some("Couldn’t bring the attachments along, so From is unchanged.".into());
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
        composer.account = Some(id);
        // What quitting would keep belongs to the old account: start afresh.
        composer.saved_as = SavedAs::default();
        let serial = composer.serial;
        let moved = composer.draft_id.take();
        let saving = composer.saving;
        if let (Some(draft), Some(client)) = (moved.clone(), old_client) {
            self.forget_moved_draft(client, draft, cx);
        }
        if moved.is_some() || saving {
            // Unsaved in the new account: it counts as an edit, so it is saved there.
            if let Some(composer) = &mut self.compose {
                composer.edits += 1;
                composer.saved = false;
            }
            self.save_composer(serial, false, cx);
        }
        cx.notify();
    }

    /// Deletes a draft left in the account a composer's From moved away from. Quitting waits
    /// for it, as for other writes.
    fn forget_moved_draft(&self, client: Arc<mail::Client>, id: String, cx: &mut Context<Self>) {
        let working = self.writes.begin();
        cx.background_executor()
            .spawn(async move {
                let _working = working;
                if let Err(error) = client.delete_draft(&id)
                    && !mail::is_gone(&error)
                {
                    eprintln!("nimbra: a draft left in the account it was moved from");
                }
            })
            .detach();
    }

    /// Where a new message is from: the default account, with several and it connected;
    /// otherwise (`None`) the account shown.
    fn new_message_account(&self) -> Option<String> {
        if self.is_demo() || self.settings.accounts.len() < 2 {
            return None;
        }
        let id = self.settings.default_account()?.to_string();
        self.client_for(Some(&id)).map(|_| id)
    }

    pub(super) fn compose_view(
        &self,
        t: &Theme,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::theme::{radius, space};
        use crate::ui::{Elevation, elevation};

        let Some(composer) = &self.compose else {
            return div().into_any_element();
        };
        let full = composer.full;
        let loading = composer.loading;
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
        // A hairline between the sections of the card, inset to the text's edges.
        let divider = || {
            div().flex().justify_center().child(
                column(div())
                    .px(px(20.))
                    .child(div().h(px(1.)).bg(t.border)),
            )
        };
        // A line of text that is still on its way.
        let bar = |share: f32| {
            div()
                .h(px(8.))
                .w(relative(share))
                .rounded_full()
                .bg(t.border)
        };
        let field = |label: &str,
                     input: &Entity<TextInput>,
                     lead: Vec<AnyElement>,
                     trailing: Option<AnyElement>| {
            // While a draft loads its fields keep their place (and the keyboard), unseen.
            let value = div()
                .flex_1()
                .min_w(px(120.))
                .relative()
                .child(div().when(loading, |d| d.opacity(0.)).child(input.clone()))
                .when(loading, |d| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .child(bar(0.4)),
                    )
                });
            div().flex().justify_center().child(
                column(div())
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(space::M))
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
                            .child(value),
                    )
                    .children(trailing),
            )
        };
        // The small round button that takes something off.
        let remove = |id: (&'static str, usize)| {
            let hover = t.hover;
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(18.))
                .rounded_full()
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .press(t)
                .child(icon("x", 10., t.text3))
        };
        // Each finished address is a pill with a button to take it off.
        let pills =
            |field: Field, cx: &mut Context<Self>| -> Vec<AnyElement> {
                let Some(slot) = Composer::slot(field) else {
                    return Vec::new();
                };
                composer.chips[slot]
                    .iter()
                    .enumerate()
                    .map(|(i, address)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .pl(px(10.))
                            .pr(px(2.))
                            .py(px(1.))
                            .rounded_full()
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border_strong)
                            .text_size(px(13.))
                            .child(short(address, 34))
                            .child(remove(("remove-recipient", slot * 1000 + i)).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(composer) = &mut this.compose
                                        && i < composer.chips[slot].len()
                                    {
                                        composer.chips[slot].remove(i);
                                        composer.sync_placeholder(field, cx);
                                        this.composer_changed(cx);
                                        cx.notify();
                                    }
                                }),
                            ))
                            .into_any_element()
                    })
                    .collect()
            };
        // "Cc Bcc" opens the two extra recipient fields.
        let cc_toggle = (!composer.show_cc && !loading).then(|| {
            let (hover, bright) = (t.hover, t.text);
            div()
                .id("show-cc")
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .mr(px(-6.))
                .rounded(px(radius::CHIP))
                .text_color(t.text3)
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(bright))
                .press(t)
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
                .p(px(space::XS))
                .rounded(px(radius::CARD))
                .bg(t.raised)
                .border_1()
                .border_color(t.edge)
                .shadow(elevation(t, Elevation::Pop))
                .children(s.people.iter().enumerate().map(|(i, p)| {
                    let (hover, on) = (t.hover, i == s.selected);
                    div()
                        .id(("suggest", i))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(space::S))
                        .py(px(6.))
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .when(on, |d| d.bg(t.selected))
                        .when(!on, |d| d.hover(move |s| s.bg(hover)))
                        .map(|d| if on { d.press_selected(t) } else { d.press(t) })
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
            let strip = div().flex().justify_center().child(
                column(div())
                    .pl(px(84.))
                    .pr(px(20.))
                    .pb(px(space::S))
                    .child(list),
            );
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
        let chip = |name: &str, size: u64, remover: Stateful<Div>| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(10.))
                .pr(px(space::XS))
                .py(px(space::XS))
                .rounded(px(radius::CONTROL))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .child(icon("clip", 13., t.text3))
                .child(short(name, 32))
                .child(caption(file_size(size), t.text3))
                .child(remover)
                .into_any_element()
        };
        let chips: Vec<AnyElement> = composer
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                chip(
                    &f.name,
                    f.bytes.len() as u64,
                    remove(("remove-file", i)).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(composer) = &mut this.compose
                            && i < composer.files.len()
                        {
                            composer.files.remove(i);
                            this.composer_changed(cx);
                            cx.notify();
                        }
                    })),
                )
            })
            .chain(composer.kept.iter().enumerate().map(|(i, att)| {
                chip(
                    &att.name,
                    att.size,
                    remove(("remove-kept", i)).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(composer) = &mut this.compose
                            && i < composer.kept.len()
                        {
                            composer.kept.remove(i);
                            this.composer_changed(cx);
                            cx.notify();
                        }
                    })),
                )
            }))
            .collect();
        let accent = t.violet;
        let title = if composer.draft_id.is_some() || loading {
            Some("Draft")
        } else if composer.reply.is_some() {
            Some("Reply")
        } else {
            None
        };
        let header_button =
            |name: &'static str, id: &'static str, label: SharedString, key: &'static str| {
                icon_button(t, name)
                    .id(id)
                    .press(t)
                    .tooltip(tip(t.dark, label, key))
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
            .when(full, |d| d.size_full().bg(t.surface))
            .when(!full, |d| {
                d.absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(radius::SHEET))
                    .border_1()
                    .border_color(t.edge)
                    .shadow(elevation(t, Elevation::Sheet))
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
                            .gap(px(space::S))
                            .px(px(20.))
                            .pt(px(16.))
                            .pb(px(space::S))
                            // A new message has no title: its From is the first row.
                            .children(title.map(|title| {
                                if full {
                                    title_m(title, t.text)
                                } else {
                                    title_s(title, t.text)
                                }
                            }))
                            // A new message: who it is from, and a menu to change it.
                            .when(self.can_change_from() && !loading, |d| {
                                // The account it is changing to shows at once, with what
                                // it is waiting for.
                                let moving = composer.moving_to.is_some();
                                let email = composer
                                    .moving_to
                                    .clone()
                                    .or_else(|| self.compose_from())
                                    .and_then(|id| {
                                        self.settings.accounts.iter().find(|a| a.id == id)
                                    })
                                    .map(|a| a.email.clone())
                                    .unwrap_or_default();
                                let hover = t.hover;
                                let open = self.menu == Some(Menu::From);
                                // The label is the fields' label column (56 wide, 12 to the
                                // value) and only the address is the button: its padding
                                // hangs into that gap, so the address starts where values do.
                                d.child(if title.is_some() {
                                    meta("From".to_string(), t.text3)
                                } else {
                                    div()
                                        .w(px(56.))
                                        .flex_none()
                                        .text_color(t.text3)
                                        .child("From")
                                })
                                .child(
                                    div()
                                        .id("compose-from")
                                        .flex()
                                        .items_center()
                                        .gap(px(space::XS))
                                        .min_w(px(0.))
                                        .px(px(space::S))
                                        .py(px(3.))
                                        .ml(px(-4.))
                                        .rounded(px(radius::CONTROL))
                                        .cursor_pointer()
                                        .when(open, |d| d.bg(t.selected))
                                        .when(!open, |d| d.hover(move |s| s.bg(hover)))
                                        .tooltip(tip(t.dark, "Send From Another Account", ""))
                                        .child(
                                            div()
                                                .min_w(px(0.))
                                                .truncate()
                                                .when(title.is_none(), |d| d.text_size(px(14.)))
                                                .text_color(t.text2)
                                                .child(email),
                                        )
                                        .child(icon("down", 12., t.text3))
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
                                        .map(|a| meta(format!("From {}", a.email), t.text3)),
                                )
                            })
                            .child(div().flex_1())
                            .when(!self.compose_detached, |d| {
                                d.child(
                                    header_button(
                                        "detach",
                                        "compose-detach",
                                        "Open in Window".into(),
                                        "",
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.detach_compose(cx)
                                        }),
                                    ),
                                )
                                .child(
                                    header_button(
                                        if full { "shrink" } else { "expand" },
                                        "compose-size",
                                        if full { "Floating" } else { "Full Page" }.into(),
                                        "⌘⇧F",
                                    )
                                    // On press, as the close button (see there).
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_full_compose(cx)
                                        }),
                                    ),
                                )
                                // Close (asks about the draft if there is one). A window of
                                // its own has its close button in the title bar instead.
                                .child(
                                    header_button("x", "compose-close", "Close".into(), "esc")
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
                                    header_button(
                                        "attach",
                                        "compose-attach",
                                        "Back to Main Window".into(),
                                        "",
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.attach_compose(cx)
                                        }),
                                    ),
                                )
                            }),
                    ),
            )
            .child(field("To", &composer.to, pills(Field::To, cx), cc_toggle))
            .children(drop_for(Field::To))
            .child(divider())
            .when(composer.show_cc, |d| {
                d.child(field("Cc", &composer.cc, pills(Field::Cc, cx), None))
                    .children(drop_for(Field::Cc))
                    .child(divider())
                    .child(field("Bcc", &composer.bcc, pills(Field::Bcc, cx), None))
                    .children(drop_for(Field::Bcc))
                    .child(divider())
            })
            .child(field("Subject", &composer.subject, Vec::new(), None))
            .child(divider())
            .when(!chips.is_empty(), |d| {
                d.child(
                    div().flex().justify_center().child(
                        column(div())
                            .flex()
                            .flex_wrap()
                            .gap(px(space::S))
                            .px(px(20.))
                            .py(px(10.))
                            .children(chips),
                    ),
                )
                .child(divider())
            })
            .when(loading, |d| {
                d.child(
                    div().flex_1().min_h(px(0.)).flex().justify_center().child(
                        column(div())
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .px(px(20.))
                            .py(px(if full { 28. } else { 18. }))
                            .children([0.22, 0.94, 0.88, 0.57].map(bar)),
                    ),
                )
            })
            .when(composer.rich && !loading, |d| {
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
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .when(active, |d| d.bg(t.selected).press_selected(t))
                        .when(!active, |d| d.hover(move |s| s.bg(hover)).press(t))
                        .tooltip(tip(t.dark, label, key))
                        .child(icon(name, 15., if active { t.violet } else { t.text2 }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &this.editor {
                                editor.exec(command);
                            }
                            this.pending_focus = Some(this.editor_focus.clone());
                            cx.notify();
                        }))
                };
                let f = composer.format;
                let gap = || div().w(px(1.)).h(px(16.)).mx(px(space::XS)).bg(t.border);
                d.child(
                    div().flex().justify_center().child(
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
                                "Bulleted List",
                                "⌘⇧8",
                                "insertUnorderedList",
                                f[3],
                            ))
                            .child(tool(
                                5,
                                "numbers",
                                "Numbered List",
                                "⌘⇧7",
                                "insertOrderedList",
                                f[4],
                            ))
                            .child(tool(6, "quote", "Quote", "", "blockquote", false))
                            .child(gap())
                            .child(tool(
                                7,
                                "clear-format",
                                "Clear Formatting",
                                "",
                                "removeFormat",
                                false,
                            )),
                    ),
                )
                .child(divider())
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
                                    // Floating, the editor (a native view) leaves the card's
                                    // side edges free for resizing.
                                    let inset = if full { 0. } else { 6. };
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
            .when(!composer.rich && !loading, |d| {
                let (hover, bright) = (t.hover, t.text2);
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
                                // The quoted earlier message stays folded behind "•••".
                                .when_some(composer.quoted.clone(), |d, quoted| {
                                    d.child(
                                        div().flex().pt(px(10.)).child(
                                            div()
                                                .id("toggle-quoted")
                                                .px(px(space::S))
                                                .rounded(px(radius::CHIP))
                                                .bg(t.hover)
                                                .border_1()
                                                .border_color(t.border)
                                                .text_color(t.text3)
                                                .cursor_pointer()
                                                .hover(move |s| s.bg(hover).text_color(bright))
                                                .press(t)
                                                .tooltip(tip(
                                                    t.dark,
                                                    if composer.show_quoted {
                                                        "Hide Quoted Text"
                                                    } else {
                                                        "Show Quoted Text"
                                                    },
                                                    "",
                                                ))
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
                                            // A rule beside the quote, not a border on it.
                                            d.child(
                                                div()
                                                    .flex()
                                                    .gap(px(space::M))
                                                    .mt(px(10.))
                                                    .child(
                                                        div()
                                                            .flex_none()
                                                            .w(px(2.))
                                                            .rounded_full()
                                                            .bg(t.border_strong),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w(px(0.))
                                                            .text_size(px(14.))
                                                            .line_height(px(22.))
                                                            .text_color(t.text3)
                                                            .child(short(&quoted, 6000)),
                                                    ),
                                            )
                                        },
                                    )
                                }),
                        ),
                )
            })
            .child(divider())
            .child(
                div().flex().justify_center().child(
                    column(div())
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .px(px(14.))
                        .py(px(space::M))
                        .child(
                            icon_button(t, "clip")
                                .id("attach")
                                .press(t)
                                .tooltip(tip(t.dark, "Attach Files", ""))
                                .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
                        )
                        // Only real status: what is happening now, or what went wrong.
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .pl(px(space::XS))
                                .truncate()
                                .text_color(if composer.error.is_some() {
                                    t.rose
                                } else {
                                    t.text3
                                })
                                .children(match &composer.error {
                                    Some(error) => Some(error.clone()),
                                    None if composer.sending => Some("Sending…".into()),
                                    None if composer.saving => Some("Saving…".into()),
                                    None if composer.saved => Some("Saved".into()),
                                    None if self.connection.is_none() => Some("Sample mode".into()),
                                    None => None,
                                }),
                        )
                        .child(
                            icon_button(t, "trash")
                                .id("discard")
                                .press(t)
                                .tooltip(tip(t.dark, "Discard Draft", ""))
                                .on_click(cx.listener(|this, _, _, cx| this.discard_compose(cx))),
                        )
                        .child(
                            button(t, ButtonKind::Secondary, "Send Later")
                                .id("send-later")
                                .press(t)
                                .when(loading, |d| d.opacity(0.5))
                                .tooltip(tip(t.dark, "Sent while Nimbra is open", ""))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.compose_ready() {
                                        this.toggle_menu(Menu::SendLater, window, cx)
                                    }
                                })),
                        )
                        .child({
                            let ready = !composer.sending && !loading;
                            crate::ui::glow(
                                button(t, ButtonKind::Primary, "Send")
                                    .id("send")
                                    .press(t)
                                    .when(!ready, |d| d.opacity(0.5))
                                    .when(ready, |d| d.tooltip(tip(t.dark, "Send", "⌘↵")))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.compose_ready() {
                                            this.send_mail(cx)
                                        }
                                    })),
                                "send",
                            )
                        }),
                ),
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
                        -1 => h.left(px(-OUT)),
                        1 => h.right(px(-OUT)),
                        _ => h.left(px(CORNER)).right(px(CORNER)),
                    };
                    h = match hy {
                        -1 => h.top(px(-OUT)),
                        1 => h.bottom(px(-OUT)),
                        _ => h.top(px(CORNER)).bottom(px(CORNER)),
                    };
                    if corner {
                        h = h.size(px(CORNER + OUT));
                    } else if hx == 0 {
                        h = h.h(px(EDGE + OUT));
                    } else {
                        h = h.w(px(EDGE + OUT));
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
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!full, |d| d.rounded(px(radius::SHEET)))
                        .bg(t.scrim)
                        .occlude()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(20.))
                                .w(px(320.))
                                .p(px(space::XL))
                                .rounded(px(radius::SHEET))
                                .bg(t.raised)
                                .border_1()
                                .border_color(t.edge)
                                .shadow(elevation(t, Elevation::Sheet))
                                .child(title_s("Save this draft?", t.text))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(space::S))
                                        .child(
                                            button(t, ButtonKind::Ghost, "Discard")
                                                .id("ask-discard")
                                                .press(t)
                                                .text_color(t.rose)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.answer_close_compose(false, cx)
                                                })),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            button(t, ButtonKind::Secondary, "Cancel")
                                                .id("ask-cancel")
                                                .press(t)
                                                .tooltip(tip(t.dark, "Keep Writing", "esc"))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.request_close_compose(cx)
                                                })),
                                        )
                                        .child(crate::ui::glow(
                                            button(t, ButtonKind::Primary, "Save Draft")
                                                .id("ask-save")
                                                .press(t)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.answer_close_compose(true, cx)
                                                })),
                                            "ask-save",
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
                "New Message"
            };
            window.set_window_title(title);
        }
        div()
            .relative()
            .size_full()
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
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
            bytes: name.as_bytes().into(),
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
