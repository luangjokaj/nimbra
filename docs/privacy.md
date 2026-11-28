# Privacy and security

What Nimbra keeps, where it keeps it, what it sends, and to whom.

- [Your mail and your Mac](#your-mail-and-your-mac)
- [Telemetry](#telemetry)

## Your mail and your Mac

- **Your mail stays on your Mac.** Nimbra talks straight to Gmail or your IMAP and SMTP servers; there is no server of its own in between, and your mail never goes anywhere else.
- **Encrypted at rest.** Mailboxes, opened threads, counts, the offline queue and snoozes are kept in `~/Library/Application Support/Nimbra/store`. Each record is encrypted with ChaCha20-Poly1305 using a key kept only in the system keychain (`nimbra` / `store-key`); the files alone, or a backup of them, are unreadable. Search results are never stored. When a Google sign-in ends, that account's store and its key are deleted; a keychain that cannot be read deletes nothing.
- **Secrets in the Keychain.** Sign-ins, refresh tokens and passwords live only in the system keychain. OAuth client files stay in `~/.config/nimbra`, never in the repo.
- **Locked-down email.** Mail is drawn natively by default, with scripts, styles and forms dropped. **Show original** (`V`) renders the email's own HTML in a system web view with scripts off, navigation blocked, and remote scripts, frames, fonts and forms blocked by a content security policy; mail that tries to send you somewhere by itself (a meta refresh) is stopped. Only `http`, `https` and `mailto` links are clickable, and they open in your browser when you click them.
- **Remote images.** In the reader view they stay hidden until you click **Load images**, and tracking pixels are never loaded. The original view shows them by default (senders can then tell the mail was opened); **Block images** turns them off for the open thread.
- **TLS everywhere.** IMAP and SMTP use rustls with certificates always checked.
- **Updates.** Downloaded builds look for a new version at `nimbra.me/releases/latest.json` a minute after opening and every six hours after that; that request carries nothing about you or your mail. A new version is downloaded from GitHub and only kept if its checksum matches, it is signed with Nimbra's Developer ID (Team 62TSR9B3L6) and notarized by Apple, and it is the version it says. Then the sidebar offers **Restart**, and it replaces the old app when Nimbra quits. Turn it off in **Settings › Privacy & Security › Update automatically**; builds you make yourself never update.
- **Telemetry: two anonymous counts, nothing else.** See [Telemetry](#telemetry) for exactly what is sent and how to turn it off.

## Telemetry

Nimbra has telemetry, and this is all of it. We count two things: how many Macs open Nimbra each day, and how many times a day it is opened. That's how we know whether anyone uses it.

**What is sent.** Each time an official build of Nimbra is launched, it sends this one event to [PostHog](https://posthog.com), hosted in the EU:

```json
{
  "api_key": "<Nimbra's PostHog project key: public, and can only send events>",
  "event": "app_opened",
  "distinct_id": "<random, new for every event>",
  "properties": {
    "first_today": true,
    "app_version": "0.1.0",
    "macos_version": "26",
    "$process_person_profile": false,
    "$geoip_disable": true
  }
}
```

- `first_today` is true on the first launch of the day on that Mac. Counting those gives daily users; counting every event gives opens. The day of the last launch is kept on your Mac in `~/Library/Application Support/Nimbra/last-opened`, and never sent.
- `distinct_id` is random and new every time, so two launches can't be linked to each other or to you.
- `$process_person_profile: false` tells PostHog to keep no profile, and `$geoip_disable: true` to skip working out where you are. The project is also set to discard IP addresses.

**What is never sent.** Your name, your addresses, your accounts or which providers you use, anything about your mail (senders, subjects, counts, labels), what you do in the app, crash data, or any ID that lasts beyond one event.

**When it isn't sent.** When you turn off **Settings › Privacy & Security › Anonymous usage counts**; in debug builds and test runs; in builds made without `NIMBRA_POSTHOG_KEY`, which includes any build you make yourself; and whenever you're offline (that launch just isn't counted).

**Check it yourself.** [`src/usage.rs`](../src/usage.rs) is the whole thing, and a test there fails if the event ever carries anything more.
