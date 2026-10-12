#!/usr/bin/env bash
#
# Wraps a built BHTerminal.app in a branded .pkg installer.
#
# Tauri has no .pkg bundle target, so this is productbuild by hand — the same
# shape the other BiswasHost installers use: a BiswasHost welcome and
# conclusion page, a preinstall that quits a running copy so the reinstall
# replaces the LIVE app, and a postinstall that reopens it afterwards.
#
# Why a .pkg at all, when there is already a .dmg: a .dmg cannot run scripts,
# so it cannot quit the old version — and the in-app update path installs from
# a .pkg for exactly that reason.
#
# ONE universal .pkg is built, never one per architecture. The updater shipped
# in v1.x picks the first asset whose name ends in .pkg with no regard for the
# architecture, so a pair of them would eventually hand an Intel Mac an
# arm64-only build.
#
# Usage: scripts/make-macos-pkg.sh <path/to/BHTerminal.app> <out-dir>

set -euo pipefail

APP=${1:?usage: make-macos-pkg.sh <BHTerminal.app> <out-dir>}
OUT=${2:?usage: make-macos-pkg.sh <BHTerminal.app> <out-dir>}
HERE=$(cd "$(dirname "$0")/.." && pwd)
PACKAGING="$HERE/app/packaging"
IDENTIFIER=com.biswashost.bhterminal

[ -d "$APP" ] || { echo "no such app bundle: $APP" >&2; exit 1; }
VERSION=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$APP/Contents/Info.plist")
[ -n "$VERSION" ] || { echo "the app bundle has no version" >&2; exit 1; }

# The executable is NOT named after the bundle — Tauri names it after the
# crate (lowercase "bhterminal") — so it is read from the plist rather than
# guessed.
EXEC=$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$APP/Contents/Info.plist")
[ -n "$EXEC" ] || { echo "the app bundle names no executable" >&2; exit 1; }

# A universal binary is the whole point of this package; refuse to ship a
# single-architecture one under a name that promises both.
ARCHS=$(/usr/bin/lipo -archs "$APP/Contents/MacOS/$EXEC")
case " $ARCHS " in
  *" arm64 "*) : ;;
  *) echo "not universal (archs: $ARCHS)" >&2; exit 1 ;;
esac
case " $ARCHS " in
  *" x86_64 "*) : ;;
  *) echo "not universal (archs: $ARCHS)" >&2; exit 1 ;;
esac
echo "==> BHTerminal $VERSION ($ARCHS)"

mkdir -p "$OUT"
PKGROOT=$(mktemp -d)
PKGTMP=$(mktemp -d)
trap 'rm -rf "$PKGROOT" "$PKGTMP"' EXIT

mkdir -p "$PKGROOT/Applications"
cp -R "$APP" "$PKGROOT/Applications/"

mkdir -p "$PKGTMP/res"
sed "s/__VERSION__/$VERSION/g" "$PACKAGING/welcome.html"    > "$PKGTMP/res/welcome.html"
sed "s/__VERSION__/$VERSION/g" "$PACKAGING/conclusion.html" > "$PKGTMP/res/conclusion.html"

pkgbuild --root "$PKGROOT" --install-location / \
  --identifier "$IDENTIFIER" --version "$VERSION" \
  --scripts "$PACKAGING/pkg-scripts" \
  --ownership recommended "$PKGTMP/component.pkg"

cat > "$PKGTMP/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="1">
  <title>BHTerminal</title>
  <welcome file="welcome.html" mime-type="text/html"/>
  <conclusion file="conclusion.html" mime-type="text/html"/>
  <volume-check><allowed-os-versions><os-version min="10.15"/></allowed-os-versions></volume-check>
  <options customize="never" require-scripts="false" hostArchitectures="arm64,x86_64"/>
  <choices-outline><line choice="default"/></choices-outline>
  <choice id="default"><pkg-ref id="$IDENTIFIER"/></choice>
  <pkg-ref id="$IDENTIFIER" version="$VERSION">component.pkg</pkg-ref>
</installer-gui-script>
XML

PKG="$OUT/BHTerminal-$VERSION-universal.pkg"
rm -f "$PKG"
productbuild --distribution "$PKGTMP/distribution.xml" \
  --resources "$PKGTMP/res" --package-path "$PKGTMP" "$PKG"

echo "==> $PKG"
ls -lh "$PKG"
