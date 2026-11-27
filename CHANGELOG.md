# Nimbra changelog

## [Unreleased]

### Changed
- **Pop-ups centre on the mail.** With the sidebar fixed, the Undo and sending pill, notices and the update's card sit in the middle of the mail, right of the sidebar; with the sidebar hiding, in the middle of the window as before.

## [0.1.8] - 2026-11-27

### Changed
- **The list keeps its width.** Dragged wider or narrower, it opens at that width next time, and double-clicking its edge puts it back to its first width (as the sidebar's edge does), rather than closing the list.
- **Changes show everywhere at once.** A thread you star is in Starred straight away, and one you archive, move to Trash or Spam, or label is in that mailbox (and gone from the one it left) without waiting for the next check. Its star and read state follow it into the other mailboxes, and Sent is listed again right after a message goes.
- **Search by Cc and Bcc.** The search tips show `cc:` and `bcc:`, and IMAP accounts now understand them too, with `has:attachment` and `newer_than:` (or `older_than:`) days, weeks, months or years.
- **Pop-ups come and go smoothly.** The Undo and sending pill, notices and the update's card rise into place as they fade in, and sink back as they fade out; menus fade. They stay with Nimbra's window: they move with it, and go behind other apps' windows when Nimbra is not in front.
- **Check for Updates** answers at once: the update's card says "Checking for updates…" and fades, in place, into the download's progress or "Already up to date" with a green check.

### Fixed
- **Reply all keeps Cc and Bcc.** Who was on Cc goes on Cc again, and on a message you sent with Gmail, who was on Bcc goes on Bcc; they used to all go on To.
- **Long addresses in a message's details** end in "…" when there is no room for them, rather than running out of their chip.
- **Pop-ups on a second screen.** Menus, notices, address suggestions and tooltips open on the screen Nimbra's window is on, in the right place and at that screen's size, rather than on the main screen.

## [0.1.7] - 2026-11-27

### New
- **The whole conversation in its original formatting.** The original view showed only the latest email of a thread. It now shows the conversation as the reader does: earlier messages fold to a line and open in place, several at once, with a smooth animation that leaves the sender where it was. Each email shows in its own HTML at the pane's full width and as tall as it is, so the conversation scrolls as one, and a message with no HTML shows its text. Threads kept from before are downloaded again when opened, so their earlier emails have their HTML too.
- **Reply from another account.** A reply shows From like a new message, with your accounts to choose from. It goes from the conversation's account unless you change it; from another account it is still a reply for whoever gets it (it answers their message), and changing back puts it in the conversation again.

### Changed
- **Reply all in the toolbar.** Reply all sits beside Reply, as an icon with Forward after it, in the main window and in a message's own window; it is no longer in More actions. Both show their name when you point at them.
- **Message details.** The addresses and date in a message's details sit evenly spaced, and each shows its copy button only as you point at it, making room for it without moving to another line or past the card.
- **Opening a folded message.** Its header takes the place of its line, so the sender no longer shows twice; a click on the header folds it again. This is in the reader too.

### Fixed
- **Replying to your own message.** Reply on a message you sent goes to the people you sent it to (Reply all adds those on Cc), not back to you.
- **Restart from the update card.** The card that says an update is ready no longer flickers, and its Restart button works on the first click. It used to close and reopen whenever one of Nimbra's own windows took the keyboard, which could swallow the click.

## [0.1.6] - 2026-11-27

### New
- **Brand logos.** Mail from a company that publishes its logo (BIMI), such as PayPal, Apple or LinkedIn, shows that logo instead of initials, but only when the message passed the sender's email authentication (DMARC) and the company enforces it. The logo is looked up once per company and kept on your Mac.
- **Interface size.** Settings › Appearance › Interface size draws everything in Nimbra's windows at 90%, 100%, 110% or 125%: text, icons, rows and spacing. ⌘+ and ⌘- step through the sizes and ⌘0 goes back to 100%; they're in the View menu too.

### Changed
- **Sending, in one place.** The Sending pill shows a bar filling for as long as Undo can still stop the message, stays until it's sent, then says "Sent" and goes by itself, instead of a second notice.
- **Checking for mail.** The bar under the list's header runs from edge to edge.
- **Mail text size reaches all your mail.** Settings › Appearance › Mail text size now sizes the message list and folded messages too, and emails shown in their original formatting, not only the reader's text. There's a new Extra large step.
- **Settings at the bottom of the sidebar.** The whole row (your profile, its name and a gear) opens Settings. The light and dark switch that was there moved to Settings › Appearance, and ⌘⇧L still switches.

### Fixed
- **Long recipient lists.** In a message's header, a long "to" line now shortens to fit instead of running under the reply buttons.
- **Even spacing between folded messages.** In a thread, the last of the folded earlier messages had extra room under it; every one now sits evenly between its lines.
- **Make default after an update.** After Nimbra updated itself, Settings › General could not make it the default mail app, and macOS ran it from a hidden temporary copy. Updates now install as a normal app in Applications.

## [0.1.5] - 2026-11-27

### New
- **Every mailbox across your accounts.** Under All inboxes, Sent, Drafts, Trash, Starred and the rest of Mailbox show every account's mail together, newest first, and Labels lists every account's labels by name. All inboxes stays selected while you move between them; open an account to see its own, Split Inbox included.
- **Address suggestions from all your mail.** To, Cc and Bcc now suggest anyone you've written to or heard from, on every account, by name, address or domain, with the people you write to first. Nimbra learns them from the mail you read and send, and once from the headers of your newest sent and received mail, read slowly in the background. They stay encrypted on your Mac.
- **Check for mail every 10 seconds.** A new choice in Settings › General, for when you want Gmail mail sooner. Every 30 seconds is still the default.

### Changed
- **Sign out of an account.** Settings › Accounts has a sign-out button on each account where the trash can was. It does the same: Nimbra signs out and deletes that account's mail from this Mac.

### Fixed
- **Acting on another account's mail.** Deleting forever, opening a draft and Open in Gmail now use the account the email belongs to, and its rows no longer show the labels of the account you had open.
- **Buttons in dark emails.** A light button in an email that's already dark (like Vercel's) stays as the sender made it, instead of turning dark with unreadable text.
- **Emails built with modern colours.** Dark mode now reads the newer colour formats many emails use, so their text and boxes are darkened properly instead of showing as white bars.

## [0.1.4] - 2026-11-26

### New
- **Watch an update come in.** When a new version is found, a card at the bottom of the window shows it downloading with a progress bar, then while it's checked. Once it's ready, the same card says so, with a Restart button.
- **What's new, after each update.** The first time a new version opens, its release notes open in your browser. Only once, and never on a fresh install.

### Changed
- **Pick From with photos.** The composer's From shows the account's profile photo, and its menu lists each account with its photo and a check on the one you're sending from, like Settings › Composing.
- **A tidier composer header.** From has a line under it like the other fields, and the buttons beside it sit level with it and line up with Cc Bcc.

## [0.1.3] - 2026-11-26

### New
- **Finds more mail servers on its own.** When a domain publishes nothing, Nimbra now tries the usual names itself (mail., then imap. and smtp. of the domain), so most cPanel and shared hosting accounts need no typing. It only counts a server whose certificate is valid for that name, and sends no password while looking.
- **Clear a search in one click.** While a search is on, the sidebar's search box shows ✕ to end it, and the search panel has ✕ to empty what you typed.

### Fixed
- **The shortcuts table's corners.** Its header no longer shows past the table's rounded border.

## [0.1.2] - 2026-11-26

### New
- **Signatures for each account.** Make as many as you like in Settings › Composing, as formatted text or HTML with links and images, and pick one (or none) from the handle above it while you write. Images in a signature (a logo, say) show while you write and in the Preview: Nimbra downloads each one once, over https only, and keeps it on your Mac.
- **Check for Updates.** In the Nimbra menu, and in Settings › About with the update's status and a Restart button once one is ready.
- **A clearer reading toolbar.** Done, Snooze, Reply and Forward as words on the left; star, label, move, trash and more on the right. Reply all and Show original are in the more menu (and A and V). A message in its own window has the same look.
- **A new thought every day.** Inbox Zero has 365 lines now, one for each day of the year: some quiet, some silly, and some famous quotes, with their authors set apart below them.
- **A sound of its own.** New mail, snoozes coming back and reminders now play Nimbra's own chime instead of the system's (it is kept in your Sounds folder, where macOS looks for notification sounds).
- **Arrange the sidebar.** Drag accounts, Split Inbox, Mailbox and labels into the order you like, each within its own section; right-click a mailbox to hide it, or to reset its section. A "hidden" row shows them again.
- **A hello on the first launch.** The welcome screen says hello as it comes in, once, the very first time Nimbra opens.
- **Hear the notification sound.** A play button next to "Play a sound" in Settings › Notifications.

### Changed
- **A calmer list and toolbar.** Compose is a plain toolbar action, the list's top bar has a line under it, the rows line up with the bar above them, and a thread's two lines have a little more room. The view switch in the more menu reads "Switch to reader view" or "Switch to original", and an email's original view lines up with the toolbar.
- **The sample mail shows only what Nimbra really does.** Its made-up stats, meeting card and streak are gone.
- **Clearer sign-in errors.** When adding an account fails, the reason shows in a callout that opens smoothly instead of making the sheet jump.
- **Tidier email headers.** Long sender addresses end in "…" instead of being cut off, the reply icons sit closer together, labels line up with the sender's name, and the original view has the same room below the sender as beside it.
- **Theme previews fit any window.** Settings › Appearance shows System, Light and Dark as three equal previews that resize with the window.
- **About is simpler.** Check for updates and Visit website; the shortcuts are in Settings › Shortcuts and on ?.

## [0.1.1] - 2026-11-26

Still an alpha. Thanks for trying it.

### New
- **Automatic updates.** Nimbra checks for a new version every few hours, downloads it in the background, and offers a restart. Every update is checked first: its checksum, Nimbra's own Developer ID signature, and Apple's notarization. Turn it off in Settings › Privacy & Security. From this version on, you won't need to download Nimbra again.
- **A proper welcome.** The first screen, before any account is added, shows the Nimbra icon over a softly breathing glow and comes in part by part (still, if you've asked macOS to reduce motion).
- **Clearer accounts in Settings.** Each account shows where its mail comes from (Gmail, Microsoft or another server) as a badge on its photo, Default and Shown now as labels by the address, and quieter Switch to, Make default and Remove buttons.
- **Default mail app, at a glance.** Settings shows a green badge when Nimbra is the app that opens email links.

### Fixed
- **No unread count on an empty inbox.** The Dock badge and the sidebar now count what the inbox actually shows, so an inbox at zero no longer shows a number.
- **Honest sign-in text.** The Connect your inbox screen now says that deleting mail forever asks Google for permission first, instead of saying Nimbra can't delete anything permanently.

## [0.1.0] - 2026-11-26

The first release, as an alpha: things will change, and some will break. Please report what you find.

Google hasn't finished verifying Nimbra yet, so signing in with Google shows a "Google hasn't verified this app" screen first: choose **Advanced**, then **Go to Nimbra**. Until the verification is done, Google lets 100 people sign in.

### New
- **A native Mac email client, written in Rust and drawn on the GPU.** It opens straight into your mail from an encrypted store on your Mac and refreshes in the background.
- **Gmail, IMAP and Microsoft accounts,** several at once, with All inboxes merged newest first. Drag them into the order you like in Settings › Accounts, and remove any of them there.
- **One key for everything:** E done, H snooze, R reply, S star, / search, Z undo, and ? for every shortcut.
- **Split inbox:** Important, Team, Calendar, Newsletters and Other.
- **Snooze and reminders:** quick times or any day and time, and “Only if nobody replies” for reminders.
- **Block senders:** their mail goes straight to Blocked and never notifies, using the same label as Spark.
- **A formatted editor** with address suggestions that float under the field, attachments, drafts that save themselves, undo send and send later.
- **Works offline:** archive, star, read and send, and Nimbra catches up once it's back online.
- **Light and dark,** with emails darkened to match, and an app icon in every macOS appearance.
- **Private by default:** sign-ins in the Keychain, mail encrypted on disk, email shown with scripts off, and two anonymous usage counts you can turn off.
