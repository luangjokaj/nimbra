<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/icon-dark.png" />
    <img src=".github/assets/icon-light.png" alt="Nimbra" width="128" height="128" />
  </picture>

  <h1>Nimbra</h1>

  <p><strong>Blazing fast mail. No bloat.</strong></p>

  <p>
    A native email client for macOS, written in Rust and drawn on the GPU with <a href="https://www.gpui.rs">GPUI</a>.<br />
    Every action is one key, your mail stays on your Mac, and there is nothing in it you didn’t ask for.
  </p>

  <p>
    <img src="https://img.shields.io/badge/macOS-26%2B-black?style=flat&logo=apple" alt="macOS 26 or newer" />
    <img src="https://img.shields.io/badge/Rust-2024-b7410e?style=flat&logo=rust" alt="Rust" />
    <img src="https://img.shields.io/badge/UI-GPUI-8b7cff?style=flat" alt="GPUI" />
    <img src="https://img.shields.io/badge/mail-Gmail%20%C2%B7%20Microsoft%20%C2%B7%20IMAP-555?style=flat" alt="Gmail, Microsoft and IMAP" />
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3da639?style=flat" alt="MIT license" /></a>
  </p>

  <p>
    <a href="#features">Features</a> ·
    <a href="#install">Install</a> ·
    <a href="#keys">Keys</a> ·
    <a href="#privacy">Privacy</a> ·
    <a href="#telemetry">Telemetry</a> ·
    <a href="docs/guide.md">Guide</a> ·
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>
</div>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/showcase-dark.png" />
    <img src=".github/assets/showcase-light.png" alt="Nimbra's window: the sidebar with the split inbox, mailboxes and labels, the thread list, and an open email." width="100%" />
  </picture>
</p>

## Features

- **Fast by nature.** Native Rust, drawn on the GPU, no Electron. Mailboxes open from an encrypted local store and refresh in the background, so the app opens straight into your mail.
- **One key for everything.** `E` done, `H` snooze, `R` reply, `S` star, `/` search, `Z` undo. Hints show the key only while it would do something, `?` lists them all, and any of them can be changed in Settings › Shortcuts.
- **Split inbox.** Gmail's Primary, Updates, Promotions and Social, each with its unread count, plus Starred, Important, Snoozed, Reminders, Drafts, Sent, Done, All Mail, Spam, Trash and your labels. IMAP accounts keep one inbox.
- **Snooze and reminders.** Quick times, or any day and time from the calendar; “Only if nobody replies” turns a snooze into a reminder. Works for Gmail and IMAP.
- **Block senders.** Their mail goes straight to a Blocked label and never notifies. It uses the same label as Spark, so a list kept there carries on.
- **Every account in one place.** Several Gmail, Microsoft and IMAP accounts, All inboxes merged newest first, one search across every account (or just the ones you pick), and replies that go out from the right address. Nimbra opens again on the account and mailbox you left.
- **Conversations that read well.** Every message folds to one line, the latest included, and can show newest first. Emails you have read, and the ones next to the open conversation, open at once. Brands that pass DMARC show their own logo (BIMI) instead of initials.
- **Writing that gets out of the way.** A formatted editor, address suggestions, attachments by drag and drop, drafts that save themselves, undo send and send later.
- **Works offline.** Archive, star, read and send while offline; Nimbra sends it all once it can.
- **At home on the Mac.** Nimbra can be the default mail app for email links, notifications can be archived or marked read where they appear, the Dock icon has a menu and the unread count, and mail is checked the moment the Mac wakes or the network returns.
- **Light and dark,** with emails darkened to match in dark mode, a sidebar of glass over your desktop picture, and an app icon with every macOS appearance (light, dark, clear and tinted).
- **Made to fit you.** Four mail text sizes up to Extra large, an interface zoom, a list row with the subject on its own line or beside the sender, and a list as wide as you drag it.

## Install

Download the signed build from [Releases](https://github.com/nimbra-mail/desktop/releases), open the disk image and drag Nimbra to Applications. It updates itself, and [CHANGELOG.md](CHANGELOG.md) has what's new. Nimbra is in alpha: expect rough edges.

Sign in with Google or Microsoft, or add any IMAP account with its password; [Connecting your mail](docs/accounts.md) has the details.

To build it yourself, with a Rust toolchain and Xcode 26:

```sh
git clone https://github.com/nimbra-mail/desktop
cd desktop
cargo run    # opens on sample mail; no account or network needed
```

[Developing Nimbra](docs/development.md) has the rest: the Mac app bundle, signing, and running against your own copy of the mail engine.

## Keys

Everything has a key. The ones you'll use most:

| Key | Action |
| --- | --- |
| `J` / `K` | Next / previous thread |
| `E` | Mark done |
| `H` | Snooze |
| `R` / `C` | Reply / new message |
| `/` | Search |
| `Z` | Undo |
| `?` | Every shortcut on one sheet |

The full list is in [docs/keys.md](docs/keys.md). Every key can be changed in **Settings › Shortcuts**: a key already in use asks before it moves, and **Reset all shortcuts** brings back the ones above.

## Privacy

- **Your mail stays on your Mac.** Nimbra talks straight to your mail servers. There is no Nimbra server in between.
- **Encrypted at rest.** Everything Nimbra keeps is encrypted, with a key that lives only in the Keychain.
- **Secrets in the Keychain.** Sign-ins and passwords are never written anywhere else.
- **Locked-down email.** No scripts, no navigation, nothing remote but images; and in the reader, images only when you ask.
- **TLS everywhere,** with certificates always checked.

[Privacy and security](docs/privacy.md) has every detail.

## Telemetry

Nimbra counts two things, anonymously: how many Macs open it each day, and how many times it's opened. Each count is one event with a random id, the app version and the macOS version. Nothing about you, your accounts or your mail is ever sent. Turn it off in **Settings › Privacy & Security › Anonymous usage counts**. [Telemetry](docs/privacy.md#telemetry) shows the exact event, and [`src/usage.rs`](src/usage.rs) is all of the code.

## Documentation

- [Using Nimbra](docs/guide.md): reading, printing, organising, snooze, blocking, writing
- [Keyboard shortcuts](docs/keys.md)
- [Connecting your mail](docs/accounts.md): Gmail, Microsoft, IMAP and several accounts
- [Privacy and security](docs/privacy.md), with the telemetry in full
- [Developing Nimbra](docs/development.md) and [testing](docs/testing.md)

## Contributing

Bug fixes are welcome; for anything bigger, start a discussion first. [CONTRIBUTING.md](CONTRIBUTING.md) has how to
run it, what a good change looks like, and the commit style ([Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)).
Found a security problem? Report it privately: see [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE)
