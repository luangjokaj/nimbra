# Nimbra changelog

## [Unreleased]

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
