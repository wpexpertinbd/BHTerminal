#!/bin/bash
#
# Builds BHTerminal (Release) and packages it as a .dmg and .pkg into dist/.
#
# Distribution model: self-signed ("BHTerminal Dev") + Hardened Runtime, NOT
# notarized (no paid Apple Developer account). On another Mac the app is
# flagged as an unverified developer; the user approves it once via
# System Settings > Privacy & Security > "Open Anyway" (see packaging/dmg-readme.txt,
# which is bundled into the DMG). The signature is intact, so it shows the
# allowable "unverified" prompt — never the "damaged" error.
#
# Usage:  scripts/package.sh
set -euo pipefail
cd "$(dirname "$0")/.."

DD=build/pkg
APP="$DD/Build/Products/Release/BHTerminal.app"

echo "==> Generating project + building Release"
if command -v xcodegen >/dev/null; then xcodegen generate; fi
# Build products can end up read-only; make writable before removing. If the
# dir is somehow owned by another user (root), bail with a clear message.
chmod -R u+w "$DD" 2>/dev/null || true
rm -rf "$DD" 2>/dev/null || true
if [ -d "$DD" ]; then
    echo "❌ Couldn't clear $DD (likely owned by root). Run: sudo rm -rf $DD" >&2
    exit 1
fi
xcodebuild -project BHTerminal.xcodeproj -scheme BHTerminal \
  -configuration Release -destination 'platform=macOS' \
  -derivedDataPath "$DD" build

VERSION=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$APP/Contents/Info.plist")
echo "==> Packaging v$VERSION"

echo "==> Verifying signature"
codesign --verify --deep --strict "$APP"

mkdir -p dist

# ---------- DMG ----------
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp packaging/dmg-readme.txt "$STAGE/Read Me — First Launch.txt"
rm -f "dist/BHTerminal-$VERSION.dmg"
hdiutil create -volname "BHTerminal $VERSION" -srcfolder "$STAGE" \
  -ov -format UDZO "dist/BHTerminal-$VERSION.dmg"
rm -rf "$STAGE"

# ---------- PKG ----------
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
rm -f "dist/BHTerminal-$VERSION.pkg"
pkgbuild --root "$STAGE" --install-location /Applications \
  --identifier com.biswashost.BHTerminal --version "$VERSION" \
  "dist/BHTerminal-$VERSION.pkg"
rm -rf "$STAGE"

echo "==> Done:"
ls -lh "dist/BHTerminal-$VERSION.dmg" "dist/BHTerminal-$VERSION.pkg"
