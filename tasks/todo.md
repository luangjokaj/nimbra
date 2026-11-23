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
- [x] **Recipient chips and autocomplete** from people you have emailed (local, from the
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
- [~] **Multiple accounts**: add, switch and sign out per account, each with its own keychain sign-in, encrypted store and key. Still to do: a unified inbox, and notifications and scheduled sends for accounts that are not shown.
- [x] **Offline**: queue archive, star, read, send while offline and retry; show an
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

Shipped on 2026-11-21, in this order (commits on `main`):

1. `6781947` Reading: full headers, per-message reply, attachments (Quick Look, save),
   inline images, unsubscribe, spam warning and Report spam, quoted replies.
2. `a326905` Writing: Cc and Bcc, address suggestions, outgoing attachments (media upload
   over 1 MB), autosave, undo send, discard with undo, send later.
3. `90259de` Formatted editor (locked-down web view), signature, Settings, Edit menu.
4. `fbcfe36` Important, All Mail, Spam, Trash, paging, labels (show, remove, create),
   Move to, rate-limit retries.
5. `327b500` Multi-select with bulk actions, local snooze and no-reply reminders.
6. `a0b04c8` Menu bar, Dock badge, Print, open at login, store limits.
7. `29ff973` Offline queue for changes and messages.
8. `626390d` Multiple accounts (switching, per-account keychain and store).
9. Recipient pills in To, Cc and Bcc.

Verified against the real account: headers and attachments parsed; a draft with Cc, Bcc
and a 1.5 MB attachment saved, re-saved with the attachment kept, and deleted; a formatted
reply draft round-tripped; Spam, Important and paging listed; an outage was simulated and
two queued changes replayed; the first account was recorded without migration.
Verified with real keystrokes: typing, ⌘B, ⌘V, ⌘Z and ⌘↵ in the editor, and ↑ back to
Subject.

Not verified here (needs a person at the keyboard): adding a second account through the
browser and switching; the Print panel; drag and drop of files; Quick Look; open at login
from Nimbra.app.

Delete permanently: added later, asking for the full-mail scope only when first used. Not done yet:
a unified inbox across accounts, and new-mail checks, snoozes and scheduled sends for
accounts that are not shown.

## Unified inbox (2026-11-22)

Goal: "All inboxes" across every signed-in account, plus each account on its own, with
every account checked for new mail.

- [x] One Gmail connection per signed-in account (`clients`), the shown one is still `gmail`
- [x] `Mail.account`: the account a row came from (set in All inboxes)
- [x] Per-message actions use the row's account: open, read/unread, star, done, trash,
      labels, attachments, delete forever, undo
- [x] `View::AllInboxes`: first page of every inbox, merged newest first; refresh refetches;
      not saved to any one account's store
- [x] Sidebar: "All inboxes" (combined unread) and an Accounts section (each with unread),
      shown with 2+ accounts; a click on an account shows only that account
- [x] Rows in All inboxes carry an account colour dot
- [x] Replies from All inboxes go out from the message's account (drafts saved there)
- [x] Accounts not shown: new-mail notifications; due snoozes and scheduled sends
- [x] Dock badge: unread across all accounts
- [x] Verify with sample mail (two sample accounts) and build

Review: built on 2026-11-22. One connection per signed-in account (`others`), rows tagged
with their account in All inboxes, and every per-message path (`sync`, open, prefetch,
attachments, snooze wake, scheduled send, composer save and send) asks for the row's
connection. All inboxes is never written to any account's store. Checked with sample mail
and two fake accounts (sidebar, badges, counts); 44 tests pass. Not checked against two real
Gmail accounts: needs a second sign-in. Limits: All inboxes shows each inbox's first 30
threads (no endless loading yet), labels apply per account, and another account's change
made offline is rolled back rather than queued (the first and last fixed since, see "The
open items").

## Dark emails in Original formatting (2026-11-22)

Goal: in dark theme HTML mail is dark like Spark's, with the sender's scripts still off.
Five commits: toolbar spacing (already done), navigation, link leak, print, dark emails.

- [x] 1. Reproduce navigation without a click (meta refresh and the like); only followed
      links may leave the app
- [x] 2. Print: a view of its own, always light, so the email does not replace the document
- [x] 3. Dark rendering: isolated-world script, theme in the cache key, dark canvas, docs
- [x] 3b. Setting in Settings > Reading to turn dark emails off; on by default (asked for
      after the plan was approved), applies to open windows at once
- [x] 4. Verify: harness (colours, hostile mail, no requests), the real app on sample mail
      (both windows, theme switch, link click, print), tests, fmt, clippy
- [x] 5. Found while verifying: `<link rel=preconnect>` got past the policy with images
      blocked; link tags are now inert

Review: done 2026-11-22. Both security holes were reproduced in the real app with the fixes
disabled and are gone with them. Colours and hostile mail: 52 harness checks. App on sample
mail: both windows, theme and setting switched live, setting kept over a restart, link
clicks, no white frame on load, Print panel light with its header. Not checked: macOS 13,
Windows (no darkening and no click check there), a real mailbox, DNS-only lookups.

## Auto-hide sidebar (2026-11-22)

Goal: ⌘B hides the sidebar into auto-hide, as in the recording.

- [x] Hidden: nothing of it stays (sidebar, window controls, traffic lights); the panes use
      the whole width, their titles on the top line
- [x] The pointer reaching the left edge springs it in as a floating panel (about 0.18s)
      over the content, which does not move; leaving slides it out (about 0.12s)
- [x] ⌘B or the toggle while it floats docks it from where it is; toggling shut hides it
      until the edge is reached again
- [x] Works over the email's native view (a hole for the panel, the pointer read from AppKit)
- [x] Stays while one of its menus is open or the mouse is down; stays away under overlays,
      other windows and while inactive; traffic lights kept in full screen and full compose
- [x] Verify: state tests, screenshots of every state, hit tests over the email, idle CPU

Review: built 2026-11-22, checked on sample mail with a scripted pointer: every state in
screenshots, traffic lights measured, hit tests over the email, an open menu, resize, 397
random steps without a crash, about 1% CPU idle while hidden. The AppKit pointer read was
checked against the screen's. Not checked: a hand on the mouse, full screen, two displays,
Windows.
Afterwards (asked for): the floating panel has the docked blur, by clipping the content
under it (except on Inbox Zero, where the gradient shows through the tint).

## Unsubscribe, list header, row padding (2026-11-22)

- [x] Unsubscribe at the subject's top right; mailto fixed (never into an open draft, right
      account and window, body kept, no signature)
- [x] List header: one row on the top line, no title, Offline left of Compose
- [x] Rows: the same space either side of the avatar
- [x] Verify on sample mail, every width

Review: the draft overwrite was reproduced on the build before and is gone. The https link
was checked as far as the browser call (clicking a real one would unsubscribe).
Also: picking a row no longer moves it (nor the switch knob, nor the sidebar as it docks):
no state adds or removes a border.

## Message windows act on their own thread (2026-11-22)

- [x] Restore, Not spam, remove label, Done, Trash: the window's thread, found by key and
      account; the main window's selection and picks stay, nothing else is marked read
- [x] Reply and attachments use the account the window was opened from
- [x] Reply never takes over a message being written elsewhere
- [x] Spam and Trash notes only for threads really there
- [x] Reproduced with real clicks and keys on the build before; main window paths unchanged

Found while auditing, all fixed in the section below: a row's right-click menu held a
position; Done (E) did nothing in All inboxes; Windows had no check that a link was clicked.

## About pane (2026-11-22)

- [x] The app's own icon (rendered from AppIcon.icon), everything centred in the pane

## Tooltips and the floating sidebar's corners (2026-11-22)

- [x] Tooltips are click-through windows, so the email does not cut them off
- [x] The content under the floating sidebar is cut in the panel's rounded shape

## Inbox Zero (2026-11-22)

- [x] The headline in the middle of the sky, Press Z in the middle of the ground
- [x] Today's date on real mail (it was the sample's hard-coded day)

## Positions, All inboxes, Windows links (2026-11-22)

- [x] Another account's expired sign-in no longer signs out, and wipes, the shown one
- [x] Nothing acts by a remembered position: menus, picked rows, Undo, snooze panel, label
      sheet, and late answers from Gmail all follow the thread (id and account)
- [x] All inboxes: Done, Snooze and Move to work as in an inbox; counts and wake-ups go to
      the thread's own account
- [x] Windows: only a clicked link leaves the app (compiles for Windows; never run there)

Review: 25 cases reproduced on the build before and rerun after, with Gmail and the wipe
logged instead of done. Not checked: two real accounts, Windows at run time.

## Remembered sidebar (2026-11-22)

- [x] Auto-hide is kept over restarts; full view does not count as choosing it; saving is
      in order and survives quitting straight after

## Loading screen (2026-11-22)

- [x] The mail screen's own layout in ink: real bars dimmed, rows and an email that could be
      real, a wave of light; nothing moves when the mail arrives

## Cold launch into Inbox Zero (2026-11-22)

- [x] An empty Inbox kept on this Mac opens as Inbox Zero, from the first frame
- [x] Emptying the Inbox (and undoing it) is saved at once
- [x] Inbox Zero fades up on that launch; still with Reduce Motion
- [x] Checked end to end against main: launches, quit straight after, pixels

Review: sandboxed copies of main and of the change, with the keychain and Gmail stubbed.
Launches (empty, mail, first run, unreadable store, no sign-in, offline, mail arriving) and
quitting 40 to 80ms after Done, Trash, a batch or Undo. The settled screen is byte for
byte main's in four layouts. Not checked: real Gmail, switching accounts, Reduce Motion
switched on in macOS (forced in the test copy), Windows.

Left open then, fixed since ("The open items", below): a Gmail error other than
offline replaces Inbox Zero with the error screen; emptying the inbox from All inboxes is
not saved at once.

## Inbox Zero below the horizon (2026-11-22)

- [x] "Press Z to undo" only while Z would bring something back
- [x] Otherwise, as on a fresh launch, a line for the day, fading up last
- [x] Checked in sandboxed copies: launch, clear, undo, send, discard, layouts

Review: 14 runs with the keychain and Gmail stubbed (launch, relaunch, offline, reduced
motion, Done, Trash, refused change, composer open, a send and a discard held then over).
Screens with the sidebar open and hidden, and the longest line at the minimum window.

Found, left open (older than this):
- [x] A message is lost from sending if Nimbra quits within the undo delay (reproduced:
      never sent, nothing said). Switching account clears it too (read in `reset_account`,
      not run).

## Loading that never ends (2026-11-22)

Cause: an answer for a mailbox already left returned before clearing `fetching`, and that
flag also refuses every later check.

- [x] Reproduced on main with Gmail stubbed
- [x] A check belongs to the mailbox it was started for; the bar cannot outlive it
- [x] 60 seconds at most for whatever shows as loading, then it says so
- [x] A message that fails to load offers Try again
- [x] Next pages belong to their mailbox; a failed one is not asked for in a loop
- [x] The same cases rerun on the change

Review: reproduced and rerun in sandboxed copies (switching, a hung request, a hung account,
a refused message and page, sign-out, offline, expired history). Random switching: stuck
8 of 25 on main, 0 of 25 now. The real 60 seconds run once. Not checked: real Gmail, Windows,
clicking Try again (its load was run).

Found, left open (read in the code, not run):
- [x] The access token's expiry is an `Instant`, which stands still while the Mac sleeps:
      after a long sleep an expired token is used, the 401 counts as signed out, and the
      mail kept on this Mac is wiped. A 401 should refresh the token and try once more.
- [x] `went_offline`'s loop can end with `retrying` still set (no connection during an
      account switch): offline is then never retried.
- [x] A keychain read that never answers leaves the skeleton up (no timeout on purpose:
      it can be waiting for the password prompt).

## Expired tokens, lost sends, stuck reconnect (2026-11-22)

- [x] All three reproduced against the real client and a fake Gmail on localhost
- [x] A refused access token is renewed and tried once more; only a revoked sign-in signs out
- [x] A message is on disk from Send until Gmail has it; quitting sends what waits
- [x] A send, and the offline queue, stay with their account across a switch
- [x] One reconnect loop, which always lets go of its flag
- [x] The same cases rerun on the change
- [x] Only a request that never left is sent again by itself; offline mail uses the same
      records (an older version's queue is moved over)
- [x] A failed first mail check waits 15 seconds (it was retried with no pause at all)

Review: 61 end-to-end cases against the real client and a fake Gmail on localhost, the
loading cases again, 25 random switching runs. Not checked: real Gmail, a real sleep, logging
out of macOS, Windows, two real accounts.

Found, left open (read in the code, not run):
- [x] A keychain read that fails (denied, locked) makes a new store key and deletes that
      account's store (`load_or_create_key`), unsent mail included. It should only make a
      key when there is none.
- [x] Quitting does not wait for a draft save or a Done/Trash that is on its way, and does
      not ask about a message still being written.
- [x] A scheduled send that finishes after a switch updates the other account's list.
- [x] A 5xx answer to a send reads as "Not sent", though Gmail may have sent it.

## Keychain, quitting, scheduled sends, 5xx (2026-11-22)

- [x] Reproduce all four on 89cb71b (real client, fake Gmail, fake keychain)
- [x] Keychain: a key is made only when there is none; a failed read deletes nothing and
      is never taken for signed out (signing in over it could wipe or cross accounts)
- [x] Quit: wait for writes on their way; a change that did not get through is queued for
      the next launch; what is being written is on disk first, then saved to Drafts
- [x] Scheduled: each account's list is changed in that account's store, one change at a
      time, by the thread that sent it; one sender per message
- [x] A 5xx (or unreadable answer) to a send is "may have been sent", never "Not sent"
- [x] The same cases, the 61 of the last fix and the loading cases on the change
- [x] Found on the way: notices about mail were cleared by the next list and cut at 90

Review: 8 reproductions (all failed on 89cb71b), 47 adversarial cases and the 61 of the
last fix pass against the real client, a fake Gmail and a fake keychain (the real error
codes checked against the keyring crate); the loading cases and 25 random runs again.
Not checked: the real Keychain's dialogs, real Gmail, logging out of macOS, Windows,
typing in the formatted editor just before quit (the test sets the fields).

Found, left open:
- [x] A change in another account that fails offline is undone, not queued (except at quit)
- [x] Discard then quit inside the undo delay leaves the draft in Drafts
- [x] With the store unavailable, mail that waits for a connection is lost at quit

## Settings at the bottom of the sidebar (2026-11-22)

- [x] Pinned below the scrolling part, with a top border, titled Settings
- [x] One account: its address is the line under it

Review: screenshots in both themes, floating, one and two accounts, at the minimum window.
Found then, fixed since: scrolled rows showed under the window controls.

## Sidebar top and toasts (2026-11-22)

- [x] What scrolls up fades out under the window controls, in the sidebar's own material
- [x] A toast grows with its text (six lines, then an ellipsis); Dismiss is a bordered button

Review: captured in both themes, docked and floating, at five scroll positions (the material
within 2 of 255), and toasts from two words to 2,000 characters, CJK and emoji included.

## Reloading on every tab click (asked 2026-11-22, approved)

Measured on 89cb71b (fake Gmail, Google's quota table; 6,000 units a minute per user):
a known category 2 units, a label or Done about 1,340 on every click, a first visit 3,860.

- [x] A click shows what is saved; it asks Gmail only after a change or a minute
- [x] A label's sync point is never saved (`holds(&[], None)` in `fetch_inbox`)
- [x] Search-based tabs re-read only threads whose history id moved
- [x] Fetch fewer emails ahead than 20 (2,400 units)
- [x] Found on the way: page 2 fetched with every first listing, the mail on show fetched
      twice, two saves of a record at once failing, no next page after a relaunch

Review: units counted on a fake Gmail with history, pages and versions (table in the
session): a seen tab 0, a first visit at most about 1,570, a cold launch 1,700. 33 cases
that try to make a tab show stale mail pass. Not checked: real Gmail's counts and limits.

Found, left open:
- [x] Done left open does not update by itself (it would drop the pages scrolled to)
- [x] The rows kept to save units are memory only: Done is read again after a relaunch

## The sidebar's open row (2026-11-22)

- [x] Room between the search field and All inboxes
- [x] The open row is glass: more see-through than the sidebar, a neutral contrasting tint

Review: captured with the real alpha and composited over dark, grey, coloured and white
desktops, both themes, floating. Not seen on a real blurred desktop.

## Settings opens Settings (2026-11-22)

- [x] The row opens Settings; its menu is gone (switch, add and sign out are in
      Settings › Accounts, and switching in the sidebar's Accounts list)

## Inbox Zero in All inboxes (2026-11-22)

- [x] The night sky there too, with "Your inboxes are empty."
- [x] Only when every account was reached and none has more than is listed

## All inboxes after switching accounts (2026-11-22)

- [x] Its list outlives a switch (it loaded again, behind the skeleton, after each one)
- [x] The first visit draws Inbox Zero when every account's Inbox was saved empty
- [x] Switching away from the first account (id "") kept dropping its connection

## The open items (2026-11-22)

- [x] A Keychain that is slow to answer says so, and is still waited for
- [x] Another account's change made offline is queued in its store and made when back
- [x] Discard, then quit: the draft is deleted (at the next launch when offline)
- [x] What cannot be kept (no key) puts quitting off once, and says what would be lost
- [x] Done and All Mail follow the change feed; list rows are kept, encrypted, over launches
- [x] A Gmail error leaves a mailbox known to be empty on screen
- [x] An inbox emptied from All inboxes is saved at once
- [x] All inboxes loads past 30 an account
- [x] No Inbox Zero over a filter or an unlisted page
- [x] Found on the way: the next page skipped mail after clearing (page tokens drift in the
      fake, which counts by position; asking from the top is right either way)
- [x] Found on the way: one hung Keychain question held up every account's checks

Review: 12 reproductions (all failed before) and 33 cases that try to break the fixes, on
the real client, a fake Gmail and a fake keychain; the new screens captured. Not checked:
real Gmail's page tokens, real Keychain dialogs, logging out (quitting is put off for that
too), Windows (nothing puts quitting off there), two real accounts.

## Profile photos (2026-11-22)

- [x] Every account's photo, also when opening into saved mail (it loaded only after a full
      listing, and only for the account shown)
- [x] The People API picks the user's own photo over the drawn letter; without it, said
- [x] Found on the way, reproduced: signing in again (Allow, delete permanently, Connect) as
      another Google account replaced this one's sign-in and wiped it

Review: 14 cases against a fake Google (photo, letter, API off, relaunch, sign-out, wrong
account). Not checked: real Google, a real Workspace photo.

## The keyboard (2026-11-23)

- [x] `←` puts the keys in the sidebar: arrows walk it, `→`/`↵` open, `esc` leaves; a hidden
      sidebar comes in for it and goes again
- [x] Home/End, `X` and Shift-arrows to pick, `O`, Space paging, `G` chords, `⌘1`–`⌘9`
- [x] `?` shortcuts sheet
- [x] Found on the way: J/K never scrolled the list, so the open thread went off screen

Review: 34 real keystrokes through the keymap on a fake Gmail (both accounts, hidden
sidebar, search box, Settings); walking the sidebar cost 0 Gmail units; the sheet and the
cursor captured. Not checked: Space in the original (web) view on real mail, light theme.

## A living night over Inbox Zero (2026-11-23)

- [x] Stars twinkle (a third hold still), each at its own pace, easing in after the fade-up
- [x] Gentler: each fades nearly out and back over 4.5 to 9 seconds, lingering lit and
      faint, with a soft glow as it comes back to full
- [x] A star falls every 8 to 22 seconds (the first after 3.5), one at a time
- [x] Still with Reduce Motion and while the window is in the background

Review: unit tests for the twinkle range, the fall schedule and each fall's path; captured
a fall on screen. Release build: 1.7% CPU still, about 4% twinkling at 10 frames a second.
