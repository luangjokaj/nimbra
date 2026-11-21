# Nimbra: road to a complete mail client

Gap analysis against what a day-to-day mail client (Apple Mail, Gmail, Spark) is expected
to do, from reading the current code. Grouped into milestones; each one ships on its own.

## Milestone 1: Reading mail properly

- [x] **Message headers**: show From (name and address), To, Cc, Reply-To and the full date
      for every message. Today the reading pane hardcodes "to you" and only shows a time.
      Click a header line to expand all recipients; hover an address to copy it.
- [x] **Per-message actions in a thread**: reply / reply all / forward a specific message,
      not only the latest one.
- [x] **Attachments**: list real attachments (name, type, size) for every message; open
      (Quick Look) and save to Downloads. Today attachments only exist in the sample data.
- [x] **Inline images** (`cid:` parts) rendered in the original view, from the message itself.
- [x] **Unsubscribe**: show an Unsubscribe link when the message has a `List-Unsubscribe`
      header (mailto or https, with confirmation).
- [x] **Spam and phishing hints**: show Gmail's spam warning label; a "Report spam" action.

## Milestone 2: Writing mail properly

- [x] **Cc and Bcc** fields in the composer (shown on demand), with validation like To.
- [~] **Recipient chips and autocomplete** (autocomplete done; chips not yet) from people you have emailed (local, from the
      store; no contacts permission needed at first).
- [x] **Rich text editor**: bold, italic, links, lists, paste with formatting, line
      wrapping. Plan: the editor in a web view (same engine as the original view), sending
      multipart text and HTML.
- [x] **Quoted reply**: include the original message, folded behind "•••", like drafts.
- [x] **Attachments in outgoing mail**: attach files (picker and drag and drop), with a
      size check (Gmail limit 25 MB).
- [x] **Signature**: one editable signature, added to new messages and replies.
- [x] **Undo send**: hold a sent message for a few seconds with an Undo pill.
- [x] **Send later**: the button exists but does nothing; schedule with a date picker.
- [x] **Autosave drafts** while typing (today only on ⌘S).

## Milestone 3: Organising

- [x] **Labels**: remove a label, show labels on threads in the list and reading pane,
      create a label.
- [x] **Move to**: Inbox, a label, Spam; **Not spam** from the Spam folder.
- [~] **Spam and Trash mailboxes** (done, with Restore and Not spam; "Delete forever" left out on purpose, see Security) in the sidebar, with "Delete forever" in Trash
      (needs the full mail scope; see Security).
- [x] **Multi-select**: shift and ⌘ click, and select all, for bulk archive, trash, read,
      label.
- [x] **Snooze** (Gmail's API has no snooze: implement locally with a "Snoozed" label and
      a timer that moves the thread back to the inbox) and **Reminders**.
- [x] **All Mail / Important** mailboxes; more than 30 threads per mailbox (load more on
      scroll).

## Milestone 4: App essentials

- [x] **Settings window** (⌘,): account, signature, theme, reading, notifications, open at login.
- [x] **Sign out** that removes the token, store and key.
- [ ] **Multiple accounts** with a unified inbox.
- [ ] **Offline**: queue archive, star, read, send while offline and retry; show an
      offline indicator.
- [x] **Menu bar** (File, Edit, View, Message, Window) with the shortcuts, so actions are
      discoverable; Dock badge with the unread count.
- [x] **Print** a message (⌘P).
- [x] **Store limits**: cap the local store's size and prune old threads.

## Security notes

- Permanent delete and some spam actions need the broader `https://mail.google.com/` scope;
  that means signing in again, and Google treats it as restricted. Keep `gmail.modify`
  unless "Delete forever" is required.
- Attachments saved to disk are written only where the user chooses (or Downloads), never
  opened automatically; executable types get a warning.
- The rich-text editor's web view gets the same lockdown as the original view (no scripts
  from content, no remote loads, no navigation).

## Suggested order

1. Milestone 1 headers + attachments (biggest visible gap when reading).
2. Milestone 2 Cc/Bcc, quoted reply, attachments, signature, undo send.
3. Rich text editor (largest single item).
4. Milestone 3, then 4.

## Review

(To be filled in as milestones ship.)
