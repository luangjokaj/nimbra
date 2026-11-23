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
        change: impl FnOnce(&mut Vec<gmail::Label>),
        cx: &mut Context<Self>,
    ) {
        let checking = self.fetching();
        let open = match self.mailbox {
            View::Label(index) => self.labels.get(index).map(|l| l.id.clone()),
            _ => None,
        };
        change(&mut self.labels);
        self.labels.sort_by_key(|l| l.name.to_lowercase());
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
        let Some(client) = self.gmail.clone() else {
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
        let Some(client) = self.gmail.clone() else {
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
        let Some(client) = self.gmail.clone() else {
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
                Err(error) if gmail::is_gone(&error) => Ok(()),
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
    pub(super) fn thread_labels(&self, mail: &Mail) -> Vec<(usize, gmail::Label)> {
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

    /// Takes a label off the thread in that row.
    pub(super) fn remove_label(&mut self, index: usize, label_id: String, cx: &mut Context<Self>) {
        if self.is_foreign(index) {
            return;
        }
        let Some(mail) = self.mails.get_mut(index) else {
            return;
        };
        mail.labels.retain(|id| *id != label_id);
        let (Some(client), Some(id)) = (self.gmail.clone(), mail.id.clone()) else {
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

    pub(super) fn close_new_label(&mut self, cx: &mut Context<Self>) {
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
        let apply = match self.label_sheet.clone() {
            LabelSheet::Rename(id) => {
                self.close_new_label(cx);
                self.rename_label(id, name, cx);
                return;
            }
            LabelSheet::New { apply } => apply,
        };
        if let Some(existing) = self
            .labels
            .iter()
            .position(|l| l.name.eq_ignore_ascii_case(&name))
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
                        match self.label_sheet {
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
                            .gap(px(8.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", None)
                                    .id("label-cancel").press_soft()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_new_label(cx)),
                                    ),
                            )
                            .child(
                                button(
                                    t,
                                    ButtonKind::Primary,
                                    if matches!(self.label_sheet, LabelSheet::Rename(_)) { "Rename" } else { "Create" },
                                    Some("↵"),
                                )
                                    .id("label-create").press_soft()
                                    .on_click(cx.listener(|this, _, _, cx| this.create_label(cx))),
                            ),
                    ),
            )
    }

    /// Move to: back to the inbox, into a label (out of the inbox), or to Spam.
    pub(super) fn move_selected(&mut self, target: MoveTarget, cx: &mut Context<Self>) {
        let leaves = inbox_like(self.mailbox);
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
                    self.remove_selected(gmail::Change::Archive, cx);
                }
            }
        }
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
        let (Some(client), Some(id)) = (self.gmail.clone(), mail.id.clone()) else {
            return;
        };
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

/// The same labels under the same names, in any order.
fn same_labels(a: &[gmail::Label], b: &[gmail::Label]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|x| b.iter().any(|y| y.id == x.id && y.name == x.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(id: &str, name: &str) -> gmail::Label {
        gmail::Label {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn a_label_made_or_renamed_elsewhere_is_a_change() {
        let here = [label("Label_1", "Blocked"), label("Label_2", "Later")];
        let reordered = [label("Label_2", "Later"), label("Label_1", "Blocked")];
        assert!(same_labels(&here, &reordered));
        let more = [
            label("Label_1", "Blocked"),
            label("Label_2", "Later"),
            label("Label_3", "Flights"),
        ];
        assert!(!same_labels(&here, &more));
        let renamed = [label("Label_1", "Blocked"), label("Label_2", "Soon")];
        assert!(!same_labels(&here, &renamed));
        assert!(!same_labels(&here, &here[..1]));
    }
}
