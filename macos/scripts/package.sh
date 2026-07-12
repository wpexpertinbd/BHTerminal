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

# ---------- PKG (branded installer via productbuild) ----------
# A plain pkgbuild has no Welcome/Conclusion screens — this wraps the payload
# in a productbuild "distribution" so the installer shows branded BiswasHost
# intro + finish pages (matching the BHServe / Bijoy installers).
PKGROOT=$(mktemp -d)
mkdir -p "$PKGROOT/Applications"
cp -R "$APP" "$PKGROOT/Applications/"

PKGTMP=$(mktemp -d)
mkdir -p "$PKGTMP/res"
# --scripts adds a preinstall step that quits any running BHTerminal first, so
# a reinstall replaces the live app instead of leaving the old version running.
pkgbuild --root "$PKGROOT" --install-location / \
  --identifier com.biswashost.BHTerminal --version "$VERSION" \
  --scripts packaging/pkg-scripts \
  --ownership recommended "$PKGTMP/component.pkg"

# Branded Welcome + Conclusion (version substituted in from the templates).
sed "s/__VERSION__/$VERSION/g" packaging/welcome.html   > "$PKGTMP/res/welcome.html"
sed "s/__VERSION__/$VERSION/g" packaging/conclusion.html > "$PKGTMP/res/conclusion.html"

cat > "$PKGTMP/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="1">
  <title>BHTerminal</title>
  <welcome file="welcome.html" mime-type="text/html"/>
  <conclusion file="conclusion.html" mime-type="text/html"/>
  <volume-check><allowed-os-versions><os-version min="14.0"/></allowed-os-versions></volume-check>
  <options customize="never" require-scripts="false" hostArchitectures="arm64,x86_64"/>
  <choices-outline><line choice="default"/></choices-outline>
  <choice id="default"><pkg-ref id="com.biswashost.BHTerminal"/></choice>
  <pkg-ref id="com.biswashost.BHTerminal" version="$VERSION">component.pkg</pkg-ref>
</installer-gui-script>
XML

rm -f "dist/BHTerminal-$VERSION.pkg"
productbuild --distribution "$PKGTMP/distribution.xml" \
  --resources "$PKGTMP/res" --package-path "$PKGTMP" "dist/BHTerminal-$VERSION.pkg"
rm -rf "$PKGROOT" "$PKGTMP"

echo "==> Done:"
ls -lh "dist/BHTerminal-$VERSION.dmg" "dist/BHTerminal-$VERSION.pkg"
