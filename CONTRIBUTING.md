# Contributing to Nimbra

Nimbra is a native email client for macOS, written in Rust on [GPUI](https://www.gpui.rs). It's small on purpose:
fast, keyboard-first, private, and nothing in it you didn't ask for. Thanks for wanting to help. This page is the
rules, and the things review keeps asking for, written down so you don't have to learn them there.

## What we take

Bug fixes are always welcome. For anything else (a new feature, a setting, a shortcut, a restyle, a new provider),
open a [discussion](https://github.com/luangjokaj/nimbra/discussions) first and wait for a yes. Reviewing a pull
request takes longer than writing one, and a feature that doesn't fit gets declined however good the code is.

A crash, a regression, and something that shipped and behaves wrong are bugs. A new setting, a new shortcut and a
restyle are features, even when the pull request calls them a fix.

The best help often isn't a pull request:

- **Duplicates.** Comment on the issue with a link to the original.
- **Fixed bugs.** If an open issue no longer happens on `main`, say so and give the commit.
- **A pull request that already covers your bug.** Test it and comment with what you saw. That beats a second one.

### Out of scope

- **Accounts, servers or sync of our own.** Nimbra talks straight to Gmail, IMAP and SMTP. Nothing goes through a
  server of ours.
- **AI features in the base app.** Not now; open a discussion if you want to argue for one.
- **Bloat.** Anything that makes the app slower to open or heavier to use needs a very good reason.

## Getting it running

You need a Mac on macOS 26 or later with Xcode 26 and a Rust toolchain (`rustup`, stable).

```sh
cargo run                       # sample data, no network
NIMBRA_DEMO=1 cargo run         # the same, even with an account set up
./scripts/run-dev.sh            # signed debug build, for the Keychain and notifications
./scripts/bundle-macos.sh       # the release Nimbra.app
```

Without an OAuth client in `~/.config/nimbra/oauth.env`, Nimbra runs on built-in sample data and never touches the
network. That's all most changes need. To use a real account, follow [Connect your mail](README.md#connect-your-mail)
with a Google Cloud project of your own. Never commit a client id, a secret or a token.

Test copies of the app should never touch your real mail store. Run them with a throwaway home folder:

```sh
HOME=/tmp/nimbra-test NIMBRA_DEMO=1 cargo run
```

`NIMBRA_SCREEN`, `NIMBRA_THEME` and the other variables in [Screenshot states](README.md#screenshot-states) open a
given screen directly, which is how screenshots for issues and pull requests are made.

Where things live is in [Layout](README.md#layout).

## What a good change looks like

### It matches the design

A UI change looks like it was always there. Use the colours, fonts and sizes in `src/theme.rs` and the building
blocks in `src/ui.rs` (buttons, icons, pills, keys) rather than new values, and match the spacing of the screens next
to it. "Looks close" isn't a match: compare screenshots at the same size, in light and dark.

Things review checks on every UI change:

- **Both themes.** Light and dark, and the sidebar and the list each open and closed.
- **Hover never moves anything.** A state (hovered, picked, active) changes colour, never size or a border.
- **Nothing snaps.** Panels slide, glows ease. A change in motion comes with a screen recording.
- **Every way in.** If a feature can be reached from a key, a menu, a button and a row, all of them go through one
  function.
- **Hints tell the truth.** A key hint shows only while that key would do something.
- **Every account.** A change to a mailbox works in All inboxes too, for an account that isn't the one shown.

### It's simple

A bug that happens sometimes usually means two pieces of code disagree about who owns the state. Prefer the change
that removes code. A large change to save a little isn't worth it.

### It's fast

Nimbra opens into your mail from its local store and refreshes in the background. Don't add work to launch, to the
main thread, or to every frame. Anything cached has to stay correct: change it on the server and Nimbra shows the
change; delete it and it's gone.

### It's private and safe

- Secrets (sign-ins, tokens, passwords) live in the Keychain only. Mail on disk goes through the encrypted store
  (`src/store.rs`), never a plain file.
- Email HTML is untrusted. Don't loosen the reader's sanitising or the original view's content security policy
  (`src/webview.rs`) without a strong reason in the pull request.
- TLS certificates are always checked.
- No new network calls beyond the user's own mail servers without a discussion first.

Found a security problem? Don't open an issue: see [SECURITY.md](SECURITY.md).

## Code style

- `cargo fmt` before you commit. CI checks it.
- `cargo clippy` adds no new warnings.
- Read the surrounding code first and match its naming and idioms. New features go in new files where that makes
  sense.
- Doc comments say what a thing is for, in plain sentences, and the why goes in the commit message. Don't comment
  what the code already says.
- No new dependencies without a reason in the pull request. Every one is code we ship and have to trust.

## Testing

Reproduce the bug before you fix it, ideally with a test that fails. Then fix it and watch the same test pass.
Tests run the real code; a copy of the logic inside a test proves nothing about the app.

```sh
cargo test
```

The IMAP and SMTP code also has live tests against local Dovecot and Mailpit servers. If you touch `imap_mail.rs`,
`smtp.rs` or `mime.rs`, run them (setup in [docs/testing.md](docs/testing.md)):

```sh
NIMBRA_TEST_CA=certs/ca.pem cargo test live -- --ignored
```

Never use a real mailbox in a test. Build fixtures with `example.com` addresses. Screenshots for issues, pull requests
and the README use the sample data, never someone's real mail.

## Commit messages

Every commit follows [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

- **Types:** `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
- **Description:** lowercase, imperative mood, no period at the end. Say what changes for the user:
  `fix: keep the undo pill from sticking after a snooze`, not `fix: update menus.rs`.
- **Scope** when it makes the area clearer: `feat(snooze): type any time in the calendar`,
  `fix(imap): move archived copies back on unsnooze`.
- **Body** for anything bigger than a one-liner: what changed and why, in plain sentences, after a blank line. When
  a change has several parts, describe each one.
- **Breaking changes** get a `!` after the type or scope and a `BREAKING CHANGE:` footer saying what breaks and what
  to do about it.

Pull requests are squashed when merged, so the pull request title becomes the commit subject on `main`. Write it as a
Conventional Commit too. CI checks it.

## Reporting a bug

Use the bug report form; blank issues are off, and feature ideas go to
[Discussions](https://github.com/luangjokaj/nimbra/discussions). The fastest bug to fix is one with a screen
recording. Include:

- the commit or version (Settings › About shows the version),
- your macOS version,
- the account type (Gmail, IMAP with the provider, Microsoft) and whether you use All inboxes,
- the steps, and how often it happens if it's only sometimes,
- the crash report from `~/Library/Logs/DiagnosticReports`, if it crashed.

Leave out real addresses, subjects and message text. Redact screenshots.

## Pull requests

- **Issue first.** Every pull request closes an issue: open one (or find it) before you start, and put
  `Closes #<issue>` in the description. Comment on the issue that you're taking it, so two people don't fix the same
  bug.
- **One problem per pull request,** with everything that fix needs: code, tests, docs. Cleanup, refactors and a
  second fix go in their own pull requests.
- **Show that it works.** Under Verification, say how you reproduced the bug and what you saw after the fix, with
  screenshots, or a video for anything about motion or timing. "Tests pass" is not evidence. A change with nothing
  to see still has something to show: the failing test before and the passing one after.
- **Keep it green and mergeable.** CI (format, clippy, tests, the title check) passes, and the branch rebases cleanly
  on `main`.
- **Update the docs** a user would read: the README when behaviour or keys change.
- We don't owe a review or a merge, and a good pull request can still wait. Sometimes your work lands in a different
  shape; you stay on it as a `Co-authored-by:` trailer.

Releases, version bumps and anything posted publicly are the maintainer's job. Don't bump the version or edit
`CHANGELOG.md` in a pull request.

## Releases

Only the maintainer releases, with `./scripts/release <version>`: it builds, signs and notarizes Nimbra and
publishes the disk image as a GitHub Release. Pushing to `main` never releases anything.
[docs/releasing.md](docs/releasing.md) has the whole process.

`CHANGELOG.md` gets a `## [x.y.z] - YYYY-MM-DD` section for every release, with `### New` and `### Fixed`, written
at release time from the commits since the last one. One line per change, in user words (what a person notices,
not code), with the issue or pull request number and `thanks @name` for contributors. Internal changes and fixes
for bugs that never shipped are left out. That's why commit subjects matter: they're where the changelog starts.

## Working with a coding agent

Using one is fine. Point it at this file and the README first. Check its diff yourself before you open the pull
request: you're the one answering for it. For risky logic (sync, the outbox, encryption, sign-in, races), get a
second review before and after, and reproduce each finding before acting on it. Never let an agent near a real
mailbox or your Keychain while it tests.

## License

Nimbra is [MIT licensed](LICENSE). By sending a pull request you agree that your contribution is licensed under the
same terms, with no further conditions.
