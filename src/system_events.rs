//! The app's side of `integration`: links from other apps and from mail, answers to
//! notifications, a quit put off, waking and the network coming back, the Mac's appearance
//! and display options; and what the Dock and menu bar reach while no window is in front.

use super::*;
use crate::integration::{Event, NoteAction};
use crate::settings::ThemeChoice;

actions!(
    nimbra,
    [
        DockInbox,
        DockStarred,
        DockSnoozed,
        DockDrafts,
        DockSent,
        UseSystemAppearance,
        UseLightAppearance,
        UseDarkAppearance
    ]
);

/// How long the Mac waking or the network coming back settles before mail is checked.
const SETTLE: Duration = Duration::from_secs(2);

/// How long a notification's Archive or Mark as Read waits for its account to connect.
const NOTE_WAIT: Duration = Duration::from_secs(120);

/// What a flip of the shown appearance (⌘⇧L, the sidebar's button) stores: following the
/// Mac whenever that gives the same, so the same key always leads back to it.
pub(super) fn flipped(shown_dark: bool, mac_dark: bool) -> ThemeChoice {
    let dark = !shown_dark;
    if dark == mac_dark {
        ThemeChoice::System
    } else if dark {
        ThemeChoice::Dark
    } else {
        ThemeChoice::Light
    }
}

// The Dock badge's number: the engine's (both apps count the same).
pub(super) use nimbra_core::notifying::inbox_badge;

pub(super) fn theme_label(choice: ThemeChoice) -> &'static str {
    match choice {
        ThemeChoice::System => "System",
        ThemeChoice::Light => "Light",
        ThemeChoice::Dark => "Dark",
    }
}

thread_local! {
    /// What `NSApp.appearance` was last set to (None: following the Mac).
    static APPLIED: std::cell::Cell<Option<Option<bool>>> = const { std::cell::Cell::new(None) };
    /// Counts wakes and network returns, so a burst of them checks once.
    static SETTLING: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The app-wide actions: they work from the Dock and menu bar with no window in front.
pub fn install(cx: &mut App) {
    fn on_main(cx: &mut App, act: impl FnOnce(&mut Nimbra, &mut Context<Nimbra>) + 'static) {
        crate::show_main_window(cx);
        let Some(main) = crate::main_window(cx) else {
            return;
        };
        main.update(cx, |nimbra, _, cx| act(nimbra, cx)).ok();
    }
    cx.on_action(|_: &Compose, cx| {
        on_main(cx, |n, cx| n.open_compose_from(ComposeKind::New, None, cx))
    });
    cx.on_action(|_: &DockInbox, cx| on_main(cx, |n, cx| n.go_to(View::Inbox, cx)));
    cx.on_action(|_: &DockStarred, cx| on_main(cx, |n, cx| n.go_to(View::Starred, cx)));
    cx.on_action(|_: &DockSnoozed, cx| on_main(cx, |n, cx| n.go_to(View::Snoozed, cx)));
    cx.on_action(|_: &DockDrafts, cx| on_main(cx, |n, cx| n.go_to(View::Drafts, cx)));
    cx.on_action(|_: &DockSent, cx| on_main(cx, |n, cx| n.go_to(View::Sent, cx)));
    let with_main = |cx: &mut App, act: fn(&mut Nimbra, &mut Context<Nimbra>)| {
        if let Some(main) = crate::main_window(cx) {
            main.update(cx, |nimbra, _, cx| act(nimbra, cx)).ok();
        }
    };
    cx.on_action(move |_: &ToggleTheme, cx| with_main(cx, Nimbra::toggle_theme));
    cx.on_action(move |_: &UseSystemAppearance, cx| {
        with_main(cx, |n, cx| n.set_theme(ThemeChoice::System, cx))
    });
    cx.on_action(move |_: &UseLightAppearance, cx| {
        with_main(cx, |n, cx| n.set_theme(ThemeChoice::Light, cx))
    });
    cx.on_action(move |_: &UseDarkAppearance, cx| {
        with_main(cx, |n, cx| n.set_theme(ThemeChoice::Dark, cx))
    });
}

/// Which menu bar items would do something now. GPUI greys out an item whose action no
/// element listens for, so the main view listens only for these.
pub(super) struct MenuState {
    /// A thread is open: Print.
    pub open: bool,
    /// Reply, Reply All, Forward: they would not open a blank or second composer.
    pub answer: bool,
    /// Mark Done, in a mailbox where Done means something.
    pub done: bool,
    /// The open or picked threads: read or unread, star, trash, snooze.
    pub targets: bool,
    pub undo: bool,
    /// Original formatting or the reader, for a message that has both.
    pub original: bool,
}

impl Nimbra {
    pub(super) fn menu_state(&self) -> MenuState {
        let open = self.visible().contains(&self.selected);
        let targets = !self.palette && !self.targets().is_empty();
        MenuState {
            open,
            answer: open && !self.palette && self.compose.is_none(),
            done: targets && self.done_change().is_some(),
            targets,
            undo: self.can_undo(),
            original: open
                && (self.original
                    || self
                        .mails
                        .get(self.selected)
                        .is_some_and(|m| m.html.is_some())),
        }
    }

    /// Looks for a new version a minute after launch and then every few hours, while
    /// Settings allows it; one that is ready is offered in the sidebar (`update.rs`).
    pub(super) fn watch_for_updates(&mut self, cx: &mut Context<Self>) {
        if !crate::update::enabled() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let mut wait = crate::update::FIRST;
            loop {
                cx.background_executor().timer(wait).await;
                wait = crate::update::EVERY;
                let Ok(allowed) = this.update(cx, |this, _| {
                    this.settings.auto_update && this.update_ready.is_none()
                }) else {
                    break; // the window is gone
                };
                if !allowed {
                    continue;
                }
                let started = this
                    .update(cx, |this, cx| this.start_update_check(false, cx))
                    .unwrap_or(false);
                if !started {
                    continue;
                }
            }
        })
        .detach();
    }

    /// Nimbra › Check for Updates…, and the button in Settings › About: checks now, even
    /// with automatic updates off, and says what it found.
    pub(super) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if !crate::update::enabled() {
            self.update_status = Some("This build doesn’t update itself.".into());
            self.notice = Some(
                "This build doesn’t update itself. Download Nimbra from nimbra.me for updates."
                    .into(),
            );
            cx.notify();
            return;
        }
        if let Some(version) = self.update_ready.clone() {
            self.show_update_ready(&version, cx);
            return;
        }
        self.start_update_check(true, cx);
    }

    /// Checks for a new version in the background. `told`: the person asked, so the
    /// answer is shown whatever it is (the timer only speaks up when one is ready).
    /// Returns whether a check started.
    fn start_update_check(&mut self, told: bool, cx: &mut Context<Self>) -> bool {
        if self.update_checking {
            return false;
        }
        self.update_checking = true;
        self.update_status = Some("Checking for updates…".into());
        cx.notify();
        let reporter = crate::update::Reporter::default();
        // How far a download has got, read a few times a second while the check runs.
        let polled = reporter.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let now = polled.lock().ok().and_then(|p| p.clone());
                let checking = this.update(cx, |this, cx| {
                    if this.update_checking && this.update_progress != now {
                        this.update_progress = now;
                        cx.notify();
                    }
                    this.update_checking
                });
                if !matches!(checking, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { crate::update::check(&reporter) })
                .await;
            this.update(cx, |this, cx| {
                this.update_checking = false;
                this.update_progress = None;
                match found {
                    Ok(Some(version)) => {
                        // Count only; never more than the version.
                        eprintln!("nimbra: update {version} ready");
                        this.update_status = Some(format!("Nimbra {version} is ready.").into());
                        // Said where the download was shown, in the same card, with its
                        // Restart button.
                        this.show_update_ready(&version, cx);
                        this.update_ready = Some(version);
                    }
                    Ok(None) => {
                        this.update_status = Some("Nimbra is up to date.".into());
                        if told {
                            this.notice = Some(
                                format!("Nimbra is up to date ({}).", env!("CARGO_PKG_VERSION"))
                                    .into(),
                            );
                        }
                    }
                    // Offline, or a download that was not right: tried again later.
                    Err(error) => {
                        eprintln!("nimbra: no update: {error:#}");
                        this.update_status = Some("Couldn’t check for updates.".into());
                        if told {
                            this.notice =
                                Some(format!("Couldn’t check for updates: {error}.").into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        true
    }

    /// The first run of a new version opens its release notes in the browser, once. Only
    /// builds that update themselves keep track, so other builds never open them.
    pub(super) fn greet_new_version(&mut self, cx: &mut Context<Self>) {
        if !crate::update::enabled() {
            return;
        }
        let current = env!("CARGO_PKG_VERSION");
        if self.settings.last_version == current {
            return;
        }
        let used_before = !self.settings.accounts.is_empty();
        if crate::update::first_run_after_update(&self.settings.last_version, current, used_before)
        {
            cx.open_url(&crate::update::release_notes(current));
        }
        self.settings.last_version = current.to_string();
        self.save_settings(cx);
    }

    /// The sidebar's Restart: quits, and the new version opens in this one's place.
    pub(super) fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        if self.update_ready.is_none() {
            return;
        }
        self.restart_for_update = true;
        cx.quit();
    }

    /// Reads what macOS says, for as long as the app runs.
    pub(super) fn watch_system(&mut self, cx: &mut Context<Self>) {
        // Quitting is decided inside AppKit's own call, so it is kept current here; and the
        // badge follows every change, whichever window is shown or none.
        cx.observe_self(|this, cx| {
            crate::quitting::set_would_lose(this.would_lose(cx));
            this.update_badge();
            if !this.pending_notes.is_empty() {
                this.retry_notifications(cx);
            }
        })
        .detach();
        crate::notify::refresh_settings();
        self.update_badge();
        self.apply_theme(cx);
        let Some(mut events) = crate::integration::take_events() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let shows = match &event {
                    Event::OpenUrls(_) | Event::QuitPutOff => true,
                    Event::Mailto { window, .. } => this
                        .read_with(cx, |this, cx| this.message_window_of(*window, cx).is_none())
                        .unwrap_or(true),
                    Event::Notification { action, .. } => *action == NoteAction::Open,
                    _ => false,
                };
                // The window may be hidden (⌘W): brought back before it is acted in.
                if shows {
                    cx.update(crate::show_main_window).ok();
                }
                if this
                    .update(cx, |this, cx| this.on_system_event(event, cx))
                    .is_err()
                {
                    break; // the window is gone
                }
            }
        })
        .detach();
    }

    fn on_system_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::OpenUrls(urls) => {
                // One composer at a time: the first link is the one opened.
                if let Some(link) = urls.into_iter().find(|url| crate::mailto::is_mailto(url)) {
                    self.open_mailto(&link, None, None, cx);
                }
            }
            Event::Mailto {
                link,
                account,
                window,
            } => {
                // Written in the message window it was clicked in, if it was.
                let hosted = self.message_window_of(window, cx);
                // Unsaid, it is that window's account, or the open message's.
                let account = account
                    .or_else(|| hosted.as_ref().map(|(_, account)| account.clone()))
                    .or_else(|| {
                        let open = self.mails.get(self.selected)?;
                        Some(self.account_of(open).to_string())
                    });
                self.open_mailto(&link, account, hosted.map(|(host, _)| host), cx)
            }
            Event::Notification {
                account,
                thread,
                action,
            } => self.answer_notification(account, thread, action, cx),
            Event::QuitPutOff => {
                self.tell(QUIT_WOULD_LOSE);
                cx.notify();
            }
            Event::Woke | Event::Online => self.check_when_settled(cx),
            Event::DisplayOptions => cx.refresh_windows(),
            Event::Appearance => self.apply_theme(cx),
            // Settings says when macOS has turned these off.
            Event::BadgeSetting => {
                self.update_badge();
                cx.notify();
            }
        }
    }

    /// The open message window `window` is, and its account.
    fn message_window_of(
        &self,
        window: Option<gpui::AnyWindowHandle>,
        cx: &App,
    ) -> Option<(gpui::AnyWindowHandle, String)> {
        let window = window?;
        let handle = self
            .message_windows
            .iter()
            .find(|handle| gpui::AnyWindowHandle::from(**handle) == window)?;
        let account = handle.read(cx).ok()?.account().to_string();
        Some((window, account))
    }

    /// A `mailto:` link: a new message, filled in as it asks, never sent by itself. From
    /// mail, it is written from the account the mail is in, in the window it was clicked in
    /// (`host`; `None` is the main window); from elsewhere, the default.
    pub(super) fn open_mailto(
        &mut self,
        link: &str,
        account: Option<String>,
        host: Option<gpui::AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        let Some(mail) = crate::mailto::parse(link) else {
            self.notice = Some("This email link is not valid.".into());
            cx.notify();
            return;
        };
        // Never into a message that is being written: it would gain the link's recipients.
        if self.compose.is_some() || self.palette {
            self.notice = Some(
                "Finish or close the message you are writing, then open the link again.".into(),
            );
            cx.notify();
            return;
        }
        if host.is_none() {
            self.leave_sidebar(cx);
        }
        self.open_compose_from(ComposeKind::New, None, cx);
        if let Some(composer) = &mut self.compose
            && host.is_some()
        {
            // Floating over the message, as a reply there is.
            composer.full = false;
            composer.position = None;
            self.compose_host = host;
            self.compose_detached = false;
        }
        let account = account.filter(|a| self.client_for(Some(a)).is_some());
        let editor_focus = self.editor_focus.clone();
        // From the account the link was for: with that account's signature.
        let signature = account.as_ref().map(|a| self.default_signature_for(a));
        let Some(composer) = &mut self.compose else {
            return;
        };
        if account.is_some() {
            composer.account = account;
        }
        if let Some(signature) = signature {
            composer.signature = signature;
            composer.show_signature = true;
        }
        for (field, input, addresses) in [
            (Field::To, &composer.to, &mail.to),
            (Field::Cc, &composer.cc, &mail.cc),
            (Field::Bcc, &composer.bcc, &mail.bcc),
        ] {
            if addresses.is_empty() {
                continue;
            }
            let text = addresses.join(", ");
            input.update(cx, |input, cx| input.set(text, usize::MAX, cx));
            let _ = field;
        }
        for field in [Field::To, Field::Cc, Field::Bcc] {
            composer.chipify(field, true, cx);
        }
        // A Bcc is never hidden: the user sees everyone the message goes to.
        composer.show_cc = !mail.cc.is_empty() || !mail.bcc.is_empty();
        let subject = mail.subject.clone();
        composer
            .subject
            .update(cx, |input, cx| input.set(subject, usize::MAX, cx));
        if !mail.body.is_empty() {
            if composer.rich {
                // Above the empty line that `open_compose_from` put there.
                let rest = composer.load.take().unwrap_or_default();
                composer.load = Some(format!("{}{rest}", crate::editor::text_to_html(&mail.body)));
            } else if let Some(line) = composer.lines.first() {
                let text = mail.body.replace('\n', " ");
                line.update(cx, |input, cx| input.set(text, usize::MAX, cx));
            }
        }
        // Where the user goes on: the first thing the link left empty.
        if !mail.to.is_empty() {
            self.pending_focus = Some(if mail.subject.is_empty() {
                composer.subject.read(cx).focus_handle(cx)
            } else if composer.rich {
                composer.focus_body = true;
                editor_focus
            } else {
                composer.lines[0].read(cx).focus_handle(cx)
            });
        }
        cx.notify();
    }

    /// Archive or Mark as Read from a notification does what the same action does in the
    /// app, to the thread and account the notification is about; a click opens it.
    fn answer_notification(
        &mut self,
        account: Option<String>,
        thread: String,
        action: NoteAction,
        cx: &mut Context<Self>,
    ) {
        let account = account.unwrap_or_else(|| self.account_id.clone());
        let change = match action {
            NoteAction::Open => {
                self.open_thread(format!("acct:{account}:{thread}"), cx);
                return;
            }
            NoteAction::Archive => mail::Change::Archive,
            NoteAction::MarkRead => mail::Change::MarkRead,
        };
        if thread == "more" {
            return;
        }
        if let Some(row) = self
            .row_of(&thread, &account)
            .filter(|row| !self.done[*row] && self.visible().contains(row))
        {
            match change {
                mail::Change::Archive if self.done_change() == Some(mail::Change::Archive) => {
                    self.done_thread(row, cx);
                    return;
                }
                mail::Change::MarkRead => {
                    self.set_unread(row, false, cx);
                    crate::notify::withdraw(&account, &thread);
                    return;
                }
                _ => {}
            }
        }
        // Not in the open list: changed in Gmail all the same, through its account.
        let Some(client) = self.client_for(Some(&account)) else {
            // Not connected yet, as when the banner opened Nimbra: done once it is.
            self.pending_notes
                .push((std::time::Instant::now(), account, thread, action));
            // A last look once the wait is over, in case nothing else changes meanwhile.
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(NOTE_WAIT + Duration::from_secs(1))
                    .await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
            return;
        };
        self.sync_thread(client, account, thread, change, false, cx);
    }

    /// Carries out the notification answers whose account has connected since. One that
    /// waited too long is dropped, and the user told.
    fn retry_notifications(&mut self, cx: &mut Context<Self>) {
        for (since, account, thread, action) in std::mem::take(&mut self.pending_notes) {
            if self.client_for(Some(&account)).is_some() {
                self.answer_notification(Some(account), thread, action, cx);
            } else if since.elapsed() < NOTE_WAIT {
                self.pending_notes.push((since, account, thread, action));
            } else {
                self.notice = Some(
                    match action {
                        NoteAction::MarkRead => "Could not mark the message as read.",
                        _ => "Could not archive the message.",
                    }
                    .into(),
                );
                cx.notify();
            }
        }
    }

    /// The Mac woke or the network came back: once that has settled, mail is checked and
    /// anything waiting for the connection is sent, instead of at the next pause's end.
    fn check_when_settled(&mut self, cx: &mut Context<Self>) {
        let turn = SETTLING.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SETTLE).await;
            if SETTLING.with(|n| n.get()) == turn {
                crate::integration::end_pauses();
            }
        })
        .detach();
    }

    /// Unread threads in the shown account's inbox, for the Dock and the sidebar. While the
    /// whole inbox is on screen (open, and no further page), it is counted from the list,
    /// so an inbox that shows nothing unread never has a count: the server's own number can
    /// lag behind what was just read or archived, and IMAP's counts messages, not threads.
    pub(super) fn inbox_unread(&self) -> u32 {
        let listed_whole = self.mailbox == View::Inbox
            && self.query.is_empty()
            && self.status == Status::Ready
            && self.paged.contains(&View::Inbox)
            && !self.next_pages.contains_key(&View::Inbox);
        if listed_whole {
            let unread = self
                .visible()
                .into_iter()
                .filter(|&i| self.mails[i].unread)
                .count();
            return u32::try_from(unread).unwrap_or(u32::MAX);
        }
        self.counts.get("INBOX").map_or(0, |(unread, _)| *unread)
    }

    /// The Dock icon's number: unread conversations in every signed-in account's inbox.
    pub(super) fn update_badge(&mut self) {
        let count = if self.settings.dock_badge && crate::notify::badges_allowed() {
            let shown = self.connection.is_some().then(|| self.inbox_unread());
            inbox_badge(&self.account_id, shown, &self.account_unread, |id| {
                self.others.contains_key(id)
            })
        } else {
            0
        };
        if self.badge != Some(count) {
            crate::system::set_badge(count);
            self.badge = Some(count);
        }
    }

    /// Light, Dark, or following the Mac; remembered.
    pub(super) fn set_theme(&mut self, choice: ThemeChoice, cx: &mut Context<Self>) {
        self.settings.theme = choice;
        self.save_settings(cx);
        self.apply_theme(cx);
    }

    /// Makes every window, and AppKit's own panels and window buttons, follow the choice.
    pub(super) fn apply_theme(&mut self, cx: &mut Context<Self>) {
        let forced = match self.settings.theme {
            // NIMBRA_THEME (screenshots and test copies) wins over what is saved.
            _ if std::env::var_os("NIMBRA_THEME").is_some() => {
                Some(std::env::var("NIMBRA_THEME").as_deref() != Ok("light"))
            }
            ThemeChoice::Light => Some(false),
            ThemeChoice::Dark => Some(true),
            ThemeChoice::System => None,
        };
        if APPLIED.with(|applied| applied.replace(Some(forced))) != Some(forced) {
            crate::integration::set_app_appearance(forced);
        }
        // GPUI reads the Mac's appearance from NSApp; elsewhere it is not dependable, and
        // System keeps the theme the app started with.
        let mac_dark = if cfg!(target_os = "macos") {
            matches!(
                cx.window_appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            )
        } else {
            self.dark
        };
        self.dark = forced.unwrap_or(mac_dark);
        crate::integration::set_menu_check(
            &["View", "Appearance"],
            theme_label(self.settings.theme),
        );
        cx.notify();
    }
}

impl Nimbra {
    /// NIMBRA_SCREEN=update-download: the progress of a pretend download, then its notice.
    pub(super) fn pretend_update_download(&mut self, cx: &mut Context<Self>) {
        use crate::update::{Progress, Stage};
        const TOTAL: u64 = 24 * 1024 * 1024;
        cx.spawn(async move |this, cx| {
            let set = |progress: Option<Progress>,
                       this: &gpui::WeakEntity<Nimbra>,
                       cx: &mut gpui::AsyncApp| {
                this.update(cx, |this, cx| {
                    this.update_progress = progress;
                    cx.notify();
                })
                .is_ok()
            };
            cx.background_executor().timer(Duration::from_secs(1)).await;
            for step in 0..=60u64 {
                let stage = Stage::Downloading {
                    done: TOTAL * step / 60,
                    total: Some(TOTAL),
                };
                let progress = Progress {
                    version: "0.1.4".into(),
                    stage,
                };
                if !set(Some(progress), &this, cx) {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            let verifying = Progress {
                version: "0.1.4".into(),
                stage: Stage::Verifying,
            };
            set(Some(verifying), &this, cx);
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |this, cx| {
                this.show_update_ready("0.1.4", cx);
                this.update_ready = Some("0.1.4".into());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The update's card says `version` is ready, with its Restart button, for as long as a
    /// notice would stay; the sidebar keeps offering the restart after that.
    pub(super) fn show_update_ready(&mut self, version: &str, cx: &mut Context<Self>) {
        let ready = crate::update::Progress {
            version: version.to_string(),
            stage: crate::update::Stage::Ready,
        };
        self.update_progress = Some(ready.clone());
        let seconds = (4. + update_notice(version).chars().count() as f32 / 25.).min(20.);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f32(seconds))
                .await;
            this.update(cx, |this, cx| {
                if this.update_progress.as_ref() == Some(&ready) {
                    this.update_progress = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

/// What the update's card says once the new version is ready (beside its Restart button).
pub(super) fn update_notice(version: &str) -> SharedString {
    format!("Nimbra {version} is ready. Restart to update.").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flip_to_what_the_mac_shows_follows_the_mac() {
        assert_eq!(flipped(true, false), ThemeChoice::System);
        assert_eq!(flipped(false, true), ThemeChoice::System);
    }

    #[test]
    fn a_flip_away_from_the_mac_is_kept() {
        assert_eq!(flipped(false, false), ThemeChoice::Dark);
        assert_eq!(flipped(true, true), ThemeChoice::Light);
    }
}
