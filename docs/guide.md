# Using Nimbra

How Nimbra reads, files, snoozes and sends your mail. Every action also has a key: see [Keyboard shortcuts](keys.md).

- [Checking for mail](#checking-for-mail)
- [Reading](#reading)
- [Organising](#organising)
- [Snooze and reminders](#snooze-and-reminders)
- [Blocking senders](#blocking-senders)
- [Writing and sending](#writing-and-sending)

## Checking for mail

While the app is open it reads Gmail's change feed every 30 seconds (or 10 seconds, 1 or 5 minutes, set in Settings › General): only what changed since the last look is fetched, the open mailbox is updated in place, and a macOS notification (sender and subject) is shown for each thread with a new unread inbox message. Clicking a notification opens that email. Notifications need the bundled `Nimbra.app` and your permission, and only arrive while Nimbra is running.

Clicking a mailbox shows what is kept at once and asks Gmail only if something changed since it was last checked or that was over a minute ago. The content of the first 3 threads of the open mailbox is fetched ahead and stored, so opening those is instant (more would run into Gmail's limit of 6,000 units a minute). A full fetch only happens when there is no starting point yet, the point is older than Gmail keeps (about a week), or the mailbox is a search, and then only threads that changed since they were last listed are read again. The mail store keeps the newest 300 threads per mailbox, with full text for the newest 100.

## Reading

HTML mail opens in its original formatting by default; `V` switches to the native reader view and back. In dark theme, light emails get a dark page and light text, with pictures and brand colours kept; Nimbra's own script does this in an isolated WebKit world, and **Settings › Reading › Darken emails in dark theme** turns it off. Printing is always light.

**Print** is in the ⋯ menu and the File menu. **Print conversation** (⌘P) prints every message of the thread, oldest first; **Print this message** (⇧⌘P) prints the one you are reading, with its full headers. Each email prints the way it is shown, in its own formatting or as reader text, on a page Nimbra lays out with a header for each message and its attachments listed. Remote images print only when the reading view shows them.

A conversation opens on its latest message. Every message folds to its line and opens again from its header or that line, and **Settings › Appearance › Newest message first** puts the latest at the top. The emails of the last few conversations you left stay ready, and the ones above and below the open one are prepared ahead, so going back or pressing `J` and `K` shows them at once.

A sender whose mail passed DMARC, from a domain with an enforced policy and a BIMI record, shows the brand's logo instead of initials; the sender's address stays beside it. Logos are looked up once per domain and kept for a week, in a store all accounts share.

Each message shows its sender and recipients; click the recipients line for From, To, Cc, Reply-To and the full date. Earlier messages in a thread have their own Reply, Reply all and Forward. Attachments can be previewed with Quick Look or saved to Downloads. Mailing lists get an Unsubscribe button beside the subject; spam is flagged with a Not spam button. Double-click a thread to open it in a window of its own; Return reads it in full view.

## Organising

Opening a thread marks it read. `E` archives it (in Done, `E` moves it back to the inbox), `⌫` moves it to the trash, and `Z` undoes either. Threads show their labels (click × to remove one), the tag button adds a label or creates one, and the folder button moves a thread to the inbox, a label or Spam. The + next to Labels creates a label (“Parent/Child” nests it), and right-clicking a label renames or deletes it. Under All inboxes, deleting a label that several accounts have asks which ones to delete it from. On a server of its own (not Gmail's) a label is a folder, and deleting it deletes the mail in it; the question says so. ⌘-click and Shift-click pick several threads (⌘A picks all), and one Undo brings a batch back.

`/` or ⌘F opens search: matches from the mail on this Mac appear as you type, and the server's own results join a moment later. Search uses Gmail's syntax (`from:ana has:attachment newer_than:7d`). From All inboxes it searches every account at once, newest first; with several accounts, the chips under the field (or Tab) pick All accounts or one of them. IMAP servers understand `from:`, `to:`, `subject:`, `is:unread`, `has:attachment`, `newer_than:` and the like, and leave out what only Gmail has (`label:`, `category:`); a Gmail account added with an app password keeps all of Gmail's syntax.

## Snooze and reminders

`H` (or ⌘K, or the clock in the toolbar or on a row) opens the Snooze menu: later today, tomorrow, tomorrow evening, this weekend or next week, each with the time it comes back, or **Pick a date** for a calendar: click a day and press Snooze (or Return). Its Time row opens a field to type any time in (“4:45 PM”, “16:45”, “430p”; a bare 1 to 7 is the afternoon) and the hours from 6 AM to 9 PM as shortcuts. The **Only if nobody replies** switch makes any of them a reminder.

Snoozed threads leave the inbox and come back unread with a notification; a reply that arrives first brings the thread back by itself and ends the snooze, as in Gmail, and a reminder only comes back if nobody replied. Snoozed and Reminders list every account's snoozes with the time each comes back. Each snooze is kept on this Mac with its own account, because Gmail's API has no snooze and IMAP has none at all, and happens while Nimbra is running (anything due while it was closed comes back when it opens).

## Blocking senders

**Block sender** (in the reading pane's ⋯ menu) moves the open thread to a Blocked label, made if the account has none, which is where Spark files blocked mail, and blocks the sender's address for that account. Their new mail goes straight to Blocked, marked read, and never notifies; what they left in the inbox goes too. The first time each account is opened, the senders already in its Blocked label are blocked as well. Undo takes the sender off the list again, and **Unblock sender** (on a thread in Blocked) moves it back to the inbox. The list is kept on this Mac, so mail that arrives while Nimbra is closed is moved when it opens.

## Writing and sending

Messages are written in a formatted editor: bold (⌘B), italic (⌘I), underline (⌘U), links (⌘K), bulleted and numbered lists, quotes. They are sent with both an HTML and a plain-text version, and pasted text keeps simple formatting only. Replies and forwards include the original message, folded behind •••; a forward also takes the original's attachments. With several accounts, a new message or a forward can be sent from any of them.

Each account has its own signatures, made in **Settings › Composing**: pick the account, then add as many as you like, name them and choose the default (or No signature). Edit one as formatted text, as HTML, or see it in Preview. Signatures can hold links, tables and images from the web; scripts, forms and unsafe links are taken out when they are saved. A new message, reply or forward starts with its account's default under what you write, kept apart from your text; click the small handle above it to pick another or none, and changing From switches to that account's default. The editor and Preview load nothing from the web themselves: Nimbra downloads each image once (https on a public host only, up to 1 MB, PNG, JPEG, GIF or WebP), keeps it in `~/Library/Caches/Nimbra/signature-images`, and shows that copy; **Refresh images** in Preview downloads them again. The message that is sent still points to the image on the web.

The composer has Cc and Bcc, and takes attachments from the paperclip or by dropping files on it (25 MB in total). As you type in To, Cc or Bcc it suggests people from every account by name, address or domain, the ones you write to first. It learns them from the mail you list, open and send, and once per account from the headers of your newest 2,000 sent and 1,000 received messages (read slowly in the background, and never Promotions, Social, Updates or Forums). They're kept encrypted on this Mac, and Settings › Privacy's clear forgets them too. Drafts save themselves a moment after you stop typing. Sending waits six seconds so you can undo it (⌘Z). From Send until the server has it, a message is kept encrypted on this Mac: quitting sends it first, and one that could not leave is sent when its account next connects. **Send later** sends the saved draft at the chosen time while Nimbra is running.

Offline, archive, star, read and the like still work: changes are kept (encrypted, on this Mac) and sent once the server can be reached, and an Offline pill shows how many are waiting.
