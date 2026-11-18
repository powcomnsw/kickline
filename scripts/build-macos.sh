#!/usr/bin/env bash
# Build Kickline.app and Kickline-<version>.dmg on a Mac.
#
#   ./scripts/build-macos.sh            universal app (Apple Silicon + Intel)
#   ./scripts/build-macos.sh --native   only this Mac's architecture (faster)
#
# Needs: Xcode Command Line Tools (xcode-select --install), Rust (https://rustup.rs)
# and CMake for Ableton Link (installed automatically via Homebrew or pip if missing).
#   --no-link   build without Ableton Link (no CMake needed; the app is then MIT/Apache only)
#
# Signing (optional):
#   DEVELOPER_ID="Developer ID Application: Your Name (TEAMID)"  sign for distribution
#   NOTARY_PROFILE=lumen-notary   also notarise, using a profile made with
#                                 xcrun notarytool store-credentials lumen-notary ...
# Without DEVELOPER_ID the app is ad-hoc signed: it runs on this Mac, and on other
# Macs after "Open Anyway" in System Settings > Privacy & Security.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
PKG="$ROOT/packaging/macos"
OUT="$ROOT/dist"
APP="$OUT/Kickline.app"

[[ "$(uname)" == "Darwin" ]] || { echo "This script must run on macOS." >&2; exit 1; }
command -v cargo >/dev/null || { echo "Rust not found. Install from https://rustup.rs" >&2; exit 1; }
xcode-select -p >/dev/null 2>&1 || { echo "Xcode Command Line Tools missing: run xcode-select --install" >&2; exit 1; }

FEATURES=()
for a in "$@"; do [[ "$a" == "--no-link" ]] && FEATURES=(--no-default-features); done
if (( ${#FEATURES[@]} == 0 )) && ! command -v cmake >/dev/null; then
  echo "==> CMake not found (needed for Ableton Link); installing"
  if command -v brew >/dev/null; then
    brew install cmake
  else
    python3 -m pip install --user --quiet cmake
    export PATH="$(python3 -c 'import site; print(site.USER_BASE)')/bin:$PATH"
  fi
  command -v cmake >/dev/null || { echo "Couldn't install CMake. Install it from https://cmake.org or rerun with --no-link." >&2; exit 1; }
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
BUILD="$(date +%Y%m%d%H%M)"
export MACOSX_DEPLOYMENT_TARGET=11.0

if [[ " $* " == *" --native "* ]]; then
  case "$(uname -m)" in
    arm64) TARGETS=(aarch64-apple-darwin) ;;
    *) TARGETS=(x86_64-apple-darwin) ;;
  esac
else
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
fi

echo "==> Kickline $VERSION ($BUILD) for ${TARGETS[*]}"
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null
  cargo build --release --locked -p lumen-app --target "$t" ${FEATURES[@]+"${FEATURES[@]}"}
done

echo "==> Assembling Kickline.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
BINS=()
for t in "${TARGETS[@]}"; do BINS+=("target/$t/release/lumen-app"); done
if (( ${#BINS[@]} > 1 )); then
  lipo -create "${BINS[@]}" -output "$APP/Contents/MacOS/Kickline"
else
  cp "${BINS[0]}" "$APP/Contents/MacOS/Kickline"
fi
sed -e "s/__VERSION__/$VERSION/" -e "s/__BUILD__/$BUILD/" "$PKG/Info.plist" > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

echo "==> Icon"
ICONSET="$OUT/AppIcon.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s "$PKG/icon_1024.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) "$PKG/icon_1024.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

echo "==> Signing"
if [[ -n "${DEVELOPER_ID:-}" ]]; then
  codesign --force --options runtime --timestamp \
    --entitlements "$PKG/Kickline.entitlements" --sign "$DEVELOPER_ID" "$APP"
else
  codesign --force --sign - --entitlements "$PKG/Kickline.entitlements" "$APP"
  echo "    ad-hoc signed (set DEVELOPER_ID to sign for distribution)"
fi
codesign --verify --strict "$APP"

echo "==> Disk image"
DMG="$OUT/Kickline-$VERSION.dmg"
STAGE="$OUT/dmg"
rm -rf "$STAGE" "$DMG"; mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp "$ROOT/packaging/macos/INSTALL.txt" "$STAGE/Read me first.txt"
hdiutil create -volname "Kickline $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
if [[ -n "${DEVELOPER_ID:-}" ]]; then
  codesign --force --timestamp --sign "$DEVELOPER_ID" "$DMG"
fi

if [[ -n "${NOTARY_PROFILE:-}" ]]; then
  echo "==> Notarising (takes a few minutes)"
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

echo "==> Checks"
lipo -archs "$APP/Contents/MacOS/Kickline"
echo
echo "Done:"
echo "  $APP"
echo "  $DMG  ($(du -h "$DMG" | cut -f1))"
