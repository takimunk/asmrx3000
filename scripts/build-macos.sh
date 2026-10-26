#!/usr/bin/env bash
# Builds asmr-core (Rust) + the macOS menu bar app and assembles build/ASMR X3000.app.
#   scripts/build-macos.sh          build and launch
#   OPEN=0 scripts/build-macos.sh   build only
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_PKG="$ROOT/apps/macos"
APP="$ROOT/build/ASMR X3000.app"

# rustup installs into ~/.cargo; pick it up even if the shell profile was not updated.
if ! command -v cargo >/dev/null && [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

# The app runs on Apple silicon with macOS 13 (Ventura) or later. Both halves are built for
# that target: Rust reads MACOSX_DEPLOYMENT_TARGET, SwiftPM takes it from Package.swift.
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "! build on an Apple silicon Mac: the app is arm64 only" >&2
  exit 1
fi
export MACOSX_DEPLOYMENT_TARGET=13.0

echo "▸ asmr-core (rust, release)"
cargo build --manifest-path "$ROOT/core/Cargo.toml" --release

echo "▸ macOS shell (swift, release)"
BIN_DIR="$(swift build --package-path "$APP_PKG" -c release --arch arm64 --show-bin-path)"
# SwiftPM does not track the Rust static library, so force a relink.
rm -f "$BIN_DIR/ASMRX3000"
swift build --package-path "$APP_PKG" -c release --arch arm64

echo "▸ bundle"
pkill -x ASMRX3000 2>/dev/null || true
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_DIR/ASMRX3000" "$APP/Contents/MacOS/ASMRX3000"
# Local symbols are only for debugging: stripping them takes the binary from ~6.7 to ~4.4 MB.
strip -x "$APP/Contents/MacOS/ASMRX3000"
# Guard the targets: arm64 only, loadable on macOS 13.
archs="$(lipo -archs "$APP/Contents/MacOS/ASMRX3000")"
minos="$(vtool -show-build "$APP/Contents/MacOS/ASMRX3000" | awk '/minos/ {print $2}')"
if [[ "$archs" != "arm64" || "$minos" != "13.0" ]]; then
  echo "! binary is $archs, minos $minos (expected arm64, 13.0)" >&2
  exit 1
fi
cp "$APP_PKG/Support/Info.plist" "$APP/Contents/Info.plist"
# The system permission prompts (microphone, motion) follow the macOS language.
for l in en ru; do mkdir -p "$APP/Contents/Resources/$l.lproj"; cp "$APP_PKG/Support/$l.lproj/InfoPlist.strings" "$APP/Contents/Resources/$l.lproj/"; done
cp "$APP_PKG/Resources/models.bin" "$APP/Contents/Resources/models.bin"
cp "$APP_PKG/Resources/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp -R "$APP_PKG/Resources/Fonts" "$APP/Contents/Resources/Fonts"
cp -R "$APP_PKG/Resources/Onboarding" "$APP/Contents/Resources/Onboarding"
# Strings (L10n.swift reads Localization/<lang>.lproj/*.strings itself: SwiftPM's Bundle.module does not resolve in a hand-made .app).
cp -R "$APP_PKG/Resources/Localization" "$APP/Contents/Resources/Localization"
# Scenes (the scene picker). The app ships no sound files: all three scenes are procedural.
# The loop below copies any .wav a scene references into sounds/ (the app points
# ASMR_SOUNDS_DIR there); it is kept for experiments with the clip engine and is a no-op otherwise.
mkdir -p "$APP/Contents/Resources/scenes" "$APP/Contents/Resources/sounds"
SCENES=(forest_river underwater_drift body_scan)
for s in "${SCENES[@]}"; do cp "$ROOT/core/scenes/$s.json" "$APP/Contents/Resources/scenes/"; done
missing=$(cd "$APP/Contents/Resources/scenes" && { grep -ho '"[^"]*\.wav"' *.json || true; } | tr -d '"' | sort -u | while read -r f; do
  if [[ -f "$ROOT/assets/sounds/$f" ]]; then
    mkdir -p "$APP/Contents/Resources/sounds/$(dirname "$f")"
    cp "$ROOT/assets/sounds/$f" "$APP/Contents/Resources/sounds/$f"
  else
    echo "$f"
  fi
done)
if [[ -n "$missing" ]]; then
  echo "! sound files missing from the bundle (those scenes will not load):"
  echo "$missing" | sed 's/^/    /'
fi
codesign --force --sign - "$APP"
echo "✓ $APP"

if [[ "${OPEN:-1}" == "1" ]]; then
  open "$APP"
fi
