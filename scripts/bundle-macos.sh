#!/usr/bin/env bash
# Builds target/bundle/Nimbra.app: the release binary plus the Icon Composer icon.
# Needs Xcode 26 or newer for `actool` support of .icon files.
set -euo pipefail
cd "$(dirname "$0")/.."

APP="target/bundle/Nimbra.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

# Build-time settings for official builds (the usage-count key), kept outside the repo.
# Without the file the build sends no usage counts.
BUILD_ENV="${HOME}/.config/nimbra/build.env"
if [ -f "$BUILD_ENV" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$BUILD_ENV"
  set +a
fi

# A release build carries the Google and Microsoft sign-in clients, so that anyone can add
# those accounts (see `built_in` in src/gmail.rs). They are read from oauth.env, line by
# line and never run as a script; other builds read that file at run time instead.
OAUTH_ENV="${HOME}/.config/nimbra/oauth.env"
if [ "${NIMBRA_RELEASE:-}" = "1" ] && [ -f "$OAUTH_ENV" ]; then
  oauth_value() {
    sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*//p" "$OAUTH_ENV" | head -1 | tr -d '"'
  }
  export NIMBRA_BUILTIN_GOOGLE_CLIENT_ID="$(oauth_value NIMBRA_GOOGLE_CLIENT_ID)"
  export NIMBRA_BUILTIN_GOOGLE_CLIENT_SECRET="$(oauth_value NIMBRA_GOOGLE_CLIENT_SECRET)"
  export NIMBRA_BUILTIN_MICROSOFT_CLIENT_ID="$(oauth_value NIMBRA_MICROSOFT_CLIENT_ID)"
  for name in NIMBRA_BUILTIN_GOOGLE_CLIENT_ID NIMBRA_BUILTIN_GOOGLE_CLIENT_SECRET NIMBRA_BUILTIN_MICROSOFT_CLIENT_ID; do
    [ -n "${!name}" ] || { echo "error: $name is empty: check $OAUTH_ENV" >&2; exit 1; }
  done
  echo "Including the Google and Microsoft sign-in clients"
fi

# Release builds update themselves from the website's manifest (src/update.rs); others never.
if [ "${NIMBRA_RELEASE:-}" = "1" ]; then
  export NIMBRA_UPDATE_FEED="https://nimbra.me/releases/latest.json"
fi

cargo build --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/nimbra "$APP/Contents/MacOS/nimbra"

# Compile the layered icon into Assets.car (for macOS 26+) and AppIcon.icns (fallback).
xcrun actool assets/AppIcon.icon \
  --compile "$APP/Contents/Resources" \
  --app-icon AppIcon \
  --platform macosx \
  --target-device mac \
  --minimum-deployment-target 13.0 \
  --output-partial-info-plist target/bundle/icon-info.plist \
  --output-format human-readable-text --errors --warnings >/dev/null

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Nimbra</string>
  <key>CFBundleDisplayName</key><string>Nimbra</string>
  <key>CFBundleIdentifier</key><string>com.riangle.nimbra</string>
  <key>CFBundleExecutable</key><string>nimbra</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleIconName</key><string>AppIcon</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>© 2026 Luan Gjokaj</string>
  <key>LSFileQuarantineEnabled</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>Email Address URL</string>
      <key>CFBundleURLSchemes</key><array><string>mailto</string></array>
      <key>CFBundleTypeRole</key><string>Editor</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Sign with a stable identity. macOS ties Keychain permission to the app's signature, so an
# unsigned build is a "new app" after every rebuild and asks for the Keychain again.
"$(dirname "$0")/sign-macos.sh" "$APP"

echo "Built $APP"
