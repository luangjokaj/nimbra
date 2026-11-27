//! Labels: putting them on threads and taking them off, moving a thread to one, and the
//! sheet that creates, renames and deletes them.

use super::*;

/// What the label sheet does with the name typed in it.
#[derive(Clone, PartialEq)]
pub(super) enum LabelSheet {
    /// Create a label; `apply` also puts it on the open thread.
    New { apply: bool },
    /// Rename the label with this id.
    Rename(String),
}

/// Where Move to sends the open thread.
#[derive(Clone, PartialEq)]
pub(super) enum MoveTarget {
    Inbox,
    Label(String),
    Spam,
}

impl Nimbra {
    /// Changes the label list, keeping the open label mailbox pointing at the same label.
    fn change_labels(
        &mut self,
        change: impl FnOnce(&mut Vec<mail::Label>),
        cx: &mut Context<Self>,
    ) {
        let checking = self.fetching();
        let open = match self.mailbox {
            View::Label(index) => self.labels.get(index).map(|l| l.id.clone()),
            _ => None,
        };
        change(&mut self.labels);
        nimbra_core::listing::sort_labels(&mut self.labels);
        // A list being fetched now may be from before this change (`refresh_labels`).
        self.labels_asked = None;
        // Label mailboxes in memory are kept by position, which may have moved.
        self.cache.retain(|view, _| !matches!(view, View::Label(_)));
        if let Some(id) = open {
            match self.labels.iter().position(|l| l.id == id) {
                Some(index) => self.mailbox = View::Label(index),
                None => self.mailbox = View::Inbox,
            }
        }
        self.next_pages
            .retain(|view, _| !matches!(view, View::Label(_)));
        self.paged.retain(|view| !matches!(view, View::Label(_)));
        // So is a check, by the label's old position: ask again.
        self.checks.forget();
        if checking {
            self.refresh(cx);
        }
    }

    /// Asks Gmail for the account's labels again, unless it was asked a few minutes ago
    /// (`now` asks whatever). The list is kept over a relaunch, so without this a label
    /// made or renamed in Gmail itself, or on another device, never showed up here.
    pub(super) fn refresh_labels(&mut self, now: bool, cx: &mut Context<Self>) {
        const EVERY: Duration = Duration::from_secs(300);
        let Some(client) = self.connection.clone() else {
            return;
        };
        let generation = self.account_gen;
        if !now
            && self
                .labels_asked
                .is_some_and(|(at, asked_of)| asked_of == generation && at.elapsed() < EVERY)
        {
            return;
        }
        let asked = (std::time::Instant::now(), generation);
        self.labels_asked = Some(asked);
        cx.spawn(async move |this, cx| {
            let listed = cx
                .background_executor()
                .spawn(async move { client.labels() })
                .await;
            this.update(cx, |this, cx| {
                // Labels are a nicety: one that fails is asked again next time.
                let Ok(listed) = listed else {
                    this.labels_asked = None;
                    return;
                };
                // Another account by now, or a label made, renamed or deleted here since
                // (or being so now), which this list may be from before: ask again.
                if this.account_gen != generation
                    || this.labels_asked != Some(asked)
                    || this.writes.any()
                {
                    this.labels_asked = None;
                    return;
                }
                if same_labels(&this.labels, &listed) {
                    return;
                }
                this.change_labels(|labels| *labels = listed, cx);
                this.labels_asked = Some(asked);
                // Counts are asked for the labels listed: ask for the new ones'.
                this.counted = None;
                this.persist(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rename_label(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        let Some(client) = self.connection.clone() else {
            return;
        };
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let (label, new_name) = (id.clone(), name.clone());
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.rename_label(&label, &new_name)
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.change_labels(
                        |labels| {
                            if let Some(label) = labels.iter_mut().find(|l| l.id == id) {
                                label.name = name;
                            }
                        },
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn delete_label(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(client) = self.connection.clone() else {
            return;
        };
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let label = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.delete_label(&label)
                })
                .await;
            // Already gone from Gmail (deleted elsewhere, or here before this list was
            // brought up to date): that is what was asked for, so it goes from the list too.
            let result = match result {
                Err(error) if mail::is_gone(&error) => Ok(()),
                other => other,
            };
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    let was_open = matches!(this.mailbox, View::Label(i) if this.labels.get(i).is_some_and(|l| l.id == id));
                    this.change_labels(|labels| labels.retain(|l| l.id != id), cx);
                    for mail in &mut this.mails {
                        mail.labels.retain(|l| *l != id);
                    }
                    if was_open {
                        this.mailbox = View::Search; // forces the switch below to reload
                        this.select_mailbox(View::Inbox, cx);
                    }
                    this.notice = Some("Label deleted. The mail it was on is still there.".into());
                    this.persist(cx);
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The rows an action applies to: the picked ones (in list order), or the open one.
    pub(super) fn targets(&self) -> Vec<usize> {
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

    /// The user labels on a thread, with their position in the label list (for a tint).
    pub(super) fn thread_labels(&self, mail: &Mail) -> Vec<(usize, mail::Label)> {
        self.labels_on(mail)
            .map(|(index, label)| (index, label.clone()))
            .collect()
    }

    /// The account shown's labels on a thread of its own. Another account's thread (in
    /// every account's mailboxes) has none of them: label ids are each account's own, and
    /// the same id there would be another label.
    fn labels_on<'a>(&'a self, mail: &'a Mail) -> impl Iterator<Item = (usize, &'a mail::Label)> {
        let own = self.account_of(mail) == self.account_id;
        self.labels
            .iter()
            .enumerate()
            .filter(move |(_, label)| own && mail.labels.contains(&label.id))
    }

    /// The first of them, which a row shows as its tag.
    pub(super) fn first_label<'a>(&'a self, mail: &'a Mail) -> Option<(usize, &'a mail::Label)> {
        self.labels_on(mail).next()
    }

    /// Takes a label off the thread in that row.
    pub(super) fn remove_label(&mut self, index: usize, label_id: String, cx: &mut Context<Self>) {
        if self.is_foreign(index) {
            return;
        }
        self.unlabel_row(index, label_id, cx);
    }

    /// Takes the label with this id (of the thread's own account) off the thread in that
    /// row, in its account.
    fn unlabel_row(&mut self, index: usize, label_id: String, cx: &mut Context<Self>) {
        let client = self.client_at(index);
        let Some(mail) = self.mails.get_mut(index) else {
            return;
        };
        mail.labels.retain(|id| *id != label_id);
        let (Some(client), Some(id)) = (client, mail.id.clone()) else {
            cx.notify();
            return;
        };
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.remove_label(&id, &label_id)
                })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| this.fail(error, cx)).ok();
            }
        })
        .detach();
        cx.notify();
    }

    /// Opens the label sheet to rename a label.
    pub(super) fn open_rename_label(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(label) = self.labels.get(index).cloned() else {
            return;
        };
        self.open_label_sheet(LabelSheet::Rename(label.id), cx);
        if let Some(input) = &self.new_label {
            input.update(cx, |input, cx| input.set(label.name, usize::MAX, cx));
        }
    }

    /// Opens the New label sheet (from the Labels menu: also labels the open thread).
    pub(super) fn open_new_label(&mut self, cx: &mut Context<Self>) {
        self.label_for = Some(self.meant_for(&self.targets()));
        self.open_label_sheet(LabelSheet::New { apply: true }, cx);
    }

    pub(super) fn open_label_sheet(&mut self, sheet: LabelSheet, cx: &mut Context<Self>) {
        self.label_sheet = sheet;
        if self.connection.is_none() {
            self.notice = Some("Labels need a connected account.".into());
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
        self.mail_views.hide();
        cx.notify();
    }

    pub(super) fn close_new_label(&mut self, cx: &mut Context<Self>) {
        self.new_label = None;
        self._new_label_subscription = None;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Creates the label typed in the sheet and puts it on the open thread.
    fn create_label(&mut self, cx: &mut Context<Self>) {
        let (Some(input), Some(client)) = (self.new_label.clone(), self.connection.clone()) else {
            return;
        };
        let name = input.read(cx).text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let apply = match self.label_sheet.clone() {
            LabelSheet::Rename(id) => {
                self.close_new_label(cx);
                self.rename_label(id, name, cx);
                return;
            }
            LabelSheet::New { apply } => apply,
        };
        if let Some(existing) = nimbra_core::listing::label_named(&self.labels, &name)
            .and_then(|found| self.labels.iter().position(|l| l.id == found.id))
        {
            self.close_new_label(cx);
            let meant = self.label_for.clone();
            if apply && self.still_meant(&meant, cx) {
                self.apply_label(existing, cx);
            }
            return;
        }
        self.close_new_label(cx);
        let (meant, generation) = (self.label_for.clone(), self.account_gen);
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.create_label(&name)
                })
                .await;
            this.update(cx, |this, cx| match result {
                // The label is in the account it was made in.
                Ok(_) if this.account_gen != generation => {}
                Ok(label) => {
                    let id = label.id.clone();
                    let name = label.name.clone();
                    this.change_labels(|labels| labels.push(label), cx);
                    if apply && let Some(index) = this.labels.iter().position(|l| l.id == id) {
                        if this.still_meant(&meant, cx) {
                            this.apply_label(index, cx);
                        }
                    } else {
                        this.notice = Some(format!("Created “{}”.", short(&name, 30)).into());
                    }
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The New label sheet.
    pub(super) fn new_label_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .w(dp(400.))
                    .flex()
                    .flex_col()
                    .gap(dp(14.))
                    .p(dp(22.))
                    .rounded(dp(18.))
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
                    .child(serif(
                        if matches!(self.label_sheet, LabelSheet::Rename(_)) {
                            "Rename label"
                        } else {
                            "New label"
                        },
                        28.,
                        t.text,
                    ))
                    .child(
                        div()
                            .px(dp(12.))
                            .py(dp(8.))
                            .rounded(dp(10.))
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border)
                            .text_size(dp(14.))
                            .children(input),
                    )
                    .child(meta(
                        match self.label_sheet {
                            _ if self.own_servers() => "Labels are folders on your mail server: it is made (or renamed) there too.",
                            LabelSheet::Rename(_) => "Renamed in Gmail too. Use “Parent/Child” to put it inside another label.",
                            LabelSheet::New { apply: true } => "It is added to this conversation and to Gmail’s label list. Use “Parent/Child” to nest it.",
                            LabelSheet::New { apply: false } => "It is added to Gmail’s label list. Use “Parent/Child” to put it inside another label.",
                        },
                        t.text3,
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(dp(8.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", None)
                                    .id("label-cancel").press_soft()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_new_label(cx)),
                                    ),
                            )
                            .child(crate::ui::glow(

                                button(
                                    t,
                                    ButtonKind::Primary,
                                    if matches!(self.label_sheet, LabelSheet::Rename(_)) { "Rename" } else { "Create" },
                                    Some("↵"),
                                )
                                    .id("label-create").press_soft()
                                    .on_click(cx.listener(|this, _, _, cx| this.create_label(cx))), "label-create", true)),
                    ),
            )
    }

    /// Move to: back to the inbox, into a label (out of the inbox), or to Spam.
    pub(super) fn move_selected(&mut self, target: MoveTarget, cx: &mut Context<Self>) {
        let leaves = inbox_like(self.mailbox);
        match target {
            MoveTarget::Spam => self.remove_selected(mail::Change::Spam, cx),
            MoveTarget::Inbox => {
                self.remove_selected(nimbra_core::listing::to_inbox(self.mailbox), cx)
            }
            MoveTarget::Label(label_id) => {
                let Some(index) = self.labels.iter().position(|l| l.id == label_id) else {
                    return;
                };
                let rows = self.targets();
                // Labels are each account's own: without this it would be archived unlabelled.
                if rows.iter().any(|&row| self.is_foreign(row)) {
                    self.notice = Some(
                        "Labels belong to each account: open this conversation's account to move it to one."
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                let threads: Vec<String> = rows
                    .iter()
                    .filter_map(|&row| self.mails[row].id.clone())
                    .collect();
                self.apply_label(index, cx);
                if leaves {
                    for thread in threads {
                        self.moved.insert(thread, label_id.clone());
                    }
                    self.remove_selected(mail::Change::Archive, cx);
                }
            }
        }
    }

    /// The Blocked label of the account the thread in row `index` is from, if it has one.
    fn blocked_label(&self, index: usize) -> Option<String> {
        let account = self.account_of(self.mails.get(index)?);
        self.labels_of(account)
            .into_iter()
            .find(|l| crate::blocking::is_label(&l.name))
            .map(|l| l.id)
    }

    /// Whether the thread in row `index` is in its account's Blocked label.
    pub(super) fn sender_blocked(&self, index: usize) -> bool {
        let (Some(label), Some(mail)) = (self.blocked_label(index), self.mails.get(index)) else {
            return false;
        };
        mail.labels.contains(&label)
    }

    /// Blocks the open thread's sender, or unblocks it when the thread is in Blocked.
    pub(super) fn toggle_block(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        let index = self.selected;
        let refuse = |this: &mut Self, why: &str, cx: &mut Context<Self>| {
            this.notice = Some(why.to_string().into());
            cx.notify();
        };
        if self.picked.len() >= 2 {
            return refuse(self, "Block one sender at a time.", cx);
        }
        let Some(mail) = self
            .mails
            .get(index)
            .filter(|_| self.visible().contains(&index))
        else {
            return;
        };
        let address = mail.header.from.address.trim().to_lowercase();
        if !mail::is_address(&address) {
            return refuse(self, "Open the conversation to block its sender.", cx);
        }
        let name = short(mail.header.from.label(), 40);
        // Each account keeps its own blocked senders and Blocked label: the thread's.
        let account = self.account_of(mail).to_string();
        let (Some(client), Some(store)) = (self.client_at(index), self.store_of(&account)) else {
            return;
        };
        let label = self.blocked_label(index);
        let unblock = self.sender_blocked(index);
        if unblock {
            if let Some(label) = label.clone() {
                self.unlabel_row(index, label, cx);
            }
            self.move_selected(MoveTarget::Inbox, cx);
            self.notice = Some(format!("Unblocked {name}").into());
        } else if let Some(label) = label.clone() {
            self.block_moved(label, &name, cx);
        }
        let meant = Some(self.meant_for(&[index]));
        let had_label = label.is_some();
        let generation = self.account_gen;
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    if unblock {
                        if !crate::blocking::set(&store, &address, false)? {
                            anyhow::bail!("That sender has no address to block.");
                        }
                        return Ok(None);
                    }
                    crate::blocking::block(&*client, &store, &address).map(Some)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                match result {
                    Err(error) => this.fail(error, cx),
                    Ok(None) => {}
                    Ok(Some((label, moved))) => {
                        if !had_label {
                            // Made just now: list it (the shown account's; another's is
                            // listed when it is next refreshed), then move the thread there.
                            if account == this.account_id
                                && !this.labels.iter().any(|l| l.id == label)
                            {
                                let made = mail::Label {
                                    id: label.clone(),
                                    name: crate::blocking::LABEL.into(),
                                };
                                this.change_labels(|labels| labels.push(made), cx);
                            }
                            if this.still_meant(&meant, cx) {
                                this.block_moved(label, &name, cx);
                            }
                        }
                        if moved > 0 && inbox_like(this.mailbox) && !this.fetching() {
                            this.refresh(cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Moves the open thread to Blocked, in its own account (`move_selected` knows only the
    /// shown account's labels). Its Undo also unblocks the sender (`Nimbra::undo_block`);
    /// out of the inbox nothing leaves the list, so nothing to undo.
    fn block_moved(&mut self, label: String, name: &str, cx: &mut Context<Self>) {
        let leaves = inbox_like(self.mailbox);
        let index = self.selected;
        let Some(thread) = self.mails.get(index).and_then(|m| m.id.clone()) else {
            return;
        };
        self.label_row(index, label.clone(), cx);
        if leaves {
            self.moved.insert(thread, label);
            self.remove_selected(mail::Change::Archive, cx);
            self.offer_undo(format!("Blocked {name}"), cx);
        } else {
            self.notice = Some(format!("Blocked {name}").into());
        }
    }

    /// Puts the label with this id (of the thread's own account) on the thread in that
    /// row, in its account. Shows in the shown account's label mailbox at once.
    fn label_row(&mut self, index: usize, label_id: String, cx: &mut Context<Self>) {
        let client = self.client_at(index);
        let Some(mail) = self.mails.get_mut(index) else {
            return;
        };
        if !mail.labels.contains(&label_id) {
            mail.labels.push(label_id.clone());
        }
        let (Some(client), Some(id)) = (client, mail.id.clone()) else {
            return;
        };
        let (account, now) = (
            self.account_of(&self.mails[index]).to_string(),
            self.mails[index].clone(),
        );
        if account == self.account_id
            && let Some(position) = self.labels.iter().position(|l| l.id == label_id)
        {
            self.pending.add(View::Label(position), &account, &now);
        }
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.add_label(&id, &label_id)
                })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| this.fail(error, cx)).ok();
            }
        })
        .detach();
    }

    /// Undoing a move to Blocked takes the sender off the list again.
    pub(super) fn undo_block(&mut self, index: usize, label: &str, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get(index) else {
            return;
        };
        let account = self.account_of(mail);
        let blocked = match self.labels_of(account).into_iter().find(|l| l.id == label) {
            Some(listed) => crate::blocking::is_label(&listed.name),
            // Not listed yet: the Blocked label that blocking just made in another account
            // (only the shown account's list learns it at once).
            None => true,
        };
        let Some(store) = self.store_of(account).filter(|_| blocked) else {
            return;
        };
        let address = mail.header.from.address.clone();
        let working = self.writes.begin();
        cx.background_executor()
            .spawn(async move {
                let _working = working;
                if crate::blocking::set(&store, &address, false).is_err() {
                    eprintln!("nimbra: could not save the blocked senders");
                }
            })
            .detach();
    }

    /// Moves what blocked senders left in an account's inbox (while Nimbra was closed),
    /// learning the senders already in its Blocked label the first time.
    pub(super) fn sweep_blocked(&mut self, account: String, cx: &mut Context<Self>) {
        let (Some(client), Some(store)) =
            (self.client_for(Some(&account)), self.store_of(&account))
        else {
            return;
        };
        let generation = self.account_gen;
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let moved = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    crate::blocking::sweep(&*client, &store)
                })
                .await;
            if moved > 0 {
                this.update(cx, |this, cx| {
                    if this.account_gen == generation
                        && inbox_like(this.mailbox)
                        && !this.fetching()
                    {
                        this.refresh(cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Adds one of the user's labels to the open thread.
    pub(super) fn apply_label(&mut self, label: usize, cx: &mut Context<Self>) {
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
        if self.is_foreign(self.selected) {
            if let Some(mail) = self.mails.get_mut(self.selected) {
                mail.labels.retain(|id| *id != label.id);
            }
            self.notice = Some(
                "Labels belong to each account: open this conversation's account to label it."
                    .into(),
            );
            cx.notify();
            return;
        }
        let (Some(client), Some(id)) = (self.connection.clone(), mail.id.clone()) else {
            return;
        };
        // In the label's mailbox at once, before it is listed there again (`pending`).
        let (account, now) = (self.account_of(mail).to_string(), mail.clone());
        if let Some(position) = self.labels.iter().position(|l| l.id == label.id) {
            self.pending.add(View::Label(position), &account, &now);
        }
        let name = label.name.clone();
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.add_label(&id, &label.id)
                })
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
}

use nimbra_core::listing::same_labels;
