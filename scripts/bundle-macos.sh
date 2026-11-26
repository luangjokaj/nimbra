#!/usr/bin/env bash
# Builds target/bundle/Nimbra.app: the release binary plus the Icon Composer icon.
# Needs Xcode 26 or newer for `actool` support of .icon files.
set -euo pipefail
cd "$(dirname "$0")/.."

APP="target/bundle/Nimbra.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

cargo build --release

# Moved aside, never deleted in place: a running copy whose bundle is deleted fails its
# signature check, and the keychain may then refuse it.
if [ -e "$APP" ]; then
  mv "$APP" "target/bundle/Nimbra-$(date +%Y%m%d%H%M%S)-$$.app.old"
fi
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
  <!-- Attachments saved from mail are marked as downloaded, so Gatekeeper checks them. -->
  <key>LSFileQuarantineEnabled</key><true/>
</dict>
</plist>
PLIST

# Sign with a stable identity. macOS ties Keychain permission to the app's signature, so an
# unsigned build is a "new app" after every rebuild and asks for the Keychain again.
"$(dirname "$0")/sign-macos.sh" "$APP"

echo "Built $APP"
