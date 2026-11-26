# Releasing Nimbra

A release is one command on the maintainer's Mac:

```sh
./scripts/release 0.2.0
```

It builds Nimbra, signs it with the Developer ID, has Apple notarize it, packs a disk image, and publishes it as a
GitHub Release with the changelog as its notes. Nothing is released by pushing to `main`; only this command
releases, and only the maintainer runs it.

## One-time setup

You need, on the Mac you release from:

1. **Xcode 26** (for the icon compiler and `notarytool`) and the **GitHub CLI**, signed in: `gh auth login`.
2. **The Developer ID Application certificate** for team `62TSR9B3L6`, in the login keychain.
   `security find-identity -v -p codesigning` should list `Developer ID Application: Luan Gjokaj (62TSR9B3L6)`.
3. **A notarytool profile** for the same team, saved in the keychain:

   ```sh
   xcrun notarytool store-credentials nimbra --apple-id <your Apple ID> --team-id 62TSR9B3L6
   ```

   It asks for an app-specific password (appleid.apple.com › Sign-In and Security › App-Specific Passwords).
   `xcrun notarytool history --keychain-profile nimbra` should then work.
4. **`~/.config/nimbra/build.env`** (mode 600, never in the repo):

   ```sh
   NIMBRA_POSTHOG_KEY=phc_…        # the PostHog project key; release builds carry it (see the README's Telemetry)
   NIMBRA_NOTARY_PROFILE=nimbra    # the notarytool profile above (default: nimbra)
   ```

## What the command does

`./scripts/release <version>` goes through these steps and stops at the first problem. Nothing is committed or
pushed until the notarized disk image exists, so a failed run leaves nothing half-released (the version in
`Cargo.toml` is put back).

1. **Checks** that you're on `main`, it's clean (except `CHANGELOG.md`) and up to date with GitHub, CI passed for
   this commit, the version is newer than the last release, and `gh`, the notary profile and the PostHog key work.
2. **Changelog.** If `CHANGELOG.md` already has a section for the version, it's used as it is. If not, one is drafted
   from the commits since the last release, `feat:` under New and `fix:`/`perf:` under Fixed, and the script waits
   for you to edit it (it opens `$EDITOR` if set).
3. **Version.** Sets it in `Cargo.toml` and `Cargo.lock`.
4. **App.** Builds `Nimbra.app` (`scripts/bundle-macos.sh`) with the PostHog key, signs it for distribution
   (hardened runtime, secure timestamp: `scripts/sign-macos.sh`), and notarizes and staples it
   (`scripts/notarize-macos.sh`).
5. **Disk image.** Packs `dist/Nimbra-<version>.dmg` (`scripts/dmg-macos.sh`): it opens on the Inbox zero sky
   (`assets/dmg/background.png` and `@2x`), with Nimbra and an Applications shortcut either side of an arrow. Then it
   signs, notarizes and staples it too, and checks both with Gatekeeper. The window's layout is in
   `scripts/dmg-settings.py`, written by [dmgbuild](https://dmgbuild.readthedocs.io), which the script installs once
   into its own Python environment in `~/.cache/nimbra`.
6. **Commit and tag.** Commits `chore(release): <version>`, tags `v<version>`, and pushes both.
7. **GitHub Release.** Publishes `v<version>` with `Nimbra-<version>.dmg`, a copy named `Nimbra.dmg`, and
   `SHA256SUMS.txt`, with the changelog section as the notes.

Notarizing takes a minute or two per upload, so a whole release takes about five to ten minutes.

### Try it first

```sh
./scripts/release 0.2.0 --dry-run
```

does steps 1 to 5 and stops: you get the notarized disk image in `dist/` to open and test, and nothing is committed,
tagged or published. `--skip-ci` skips the CI check, for when CI is down and you've run the checks yourself.

## Versions

Nimbra follows [semantic versioning](https://semver.org). You pick the number:

| Since the last release | Example |
| --- | --- |
| Only fixes | 0.2.0 → 0.2.1 |
| Anything new | 0.2.1 → 0.3.0 |
| Something that breaks (a changed data format, a removed feature) | 0.3.0 → 0.4.0 while on 0.x, then 1.0.0 → 2.0.0 |

## Writing the changelog

`CHANGELOG.md` is what users read, and it becomes the release notes. Each release is a `## [x.y.z] - YYYY-MM-DD`
section with `### New` and `### Fixed`.

- One line per change, in user words: what a person notices, not what the code does.
  "Snoozed threads show when they come back", not "add snooze_chip to list.rs".
- Lead with the change in bold when it matters most: `- **Block senders.** Their mail goes straight to Blocked.`
- Add the issue or pull request number and thank contributors: `(#42, thanks @someone)`.
- Leave out internal changes (refactors, CI, docs) and fixes for bugs that never shipped.

The draft from commits is a starting point. Rewrite its lines the way you'd tell a friend what changed.

## Downloads

Every release has `Nimbra.dmg`, so this link always gets the newest version, for the website's download button:

```
https://github.com/luangjokaj/nimbra/releases/latest/download/Nimbra.dmg
```

While the repository is private, only people with access can download from it.

## When something goes wrong

- **Apple rejects the app.** The script prints Apple's log. The usual causes are a binary signed without the hardened
  runtime or a timestamp, or a certificate other than the Developer ID.
- **The notary profile doesn't work.** Run `xcrun notarytool history --keychain-profile nimbra`; if it fails, save the
  profile again (setup step 3). The keychain item can't be read while the screen is locked.
- **It stopped after the tag was pushed** (the GitHub Release failed). Publish it by hand:
  `gh release create v<version> dist/Nimbra-<version>.dmg dist/Nimbra.dmg dist/SHA256SUMS.txt --title "Nimbra <version>" --notes-file dist/notes.md`.
- **A release went out broken.** Don't delete or move its tag. Fix the problem and release the next patch version.

## Later

- **Automatic updates** with [Sparkle](https://sparkle-project.org): the release would also publish an update feed,
  and Nimbra would check it and offer new versions.
- **Homebrew:** `brew install --cask nimbra`, once releases are public.
- **Releasing from GitHub Actions** instead of a Mac: the same script on a macOS runner, with the certificate, the
  notary credentials and the PostHog key as repository secrets.
