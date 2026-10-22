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

echo "▸ asmr-core (rust, release)"
cargo build --manifest-path "$ROOT/core/Cargo.toml" --release

echo "▸ macOS shell (swift, release)"
BIN_DIR="$(swift build --package-path "$APP_PKG" -c release --show-bin-path)"
# SwiftPM does not track the Rust static library, so force a relink.
rm -f "$BIN_DIR/ASMRX3000"
swift build --package-path "$APP_PKG" -c release

echo "▸ bundle"
pkill -x ASMRX3000 2>/dev/null || true
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN_DIR/ASMRX3000" "$APP/Contents/MacOS/ASMRX3000"
cp "$APP_PKG/Support/Info.plist" "$APP/Contents/Info.plist"
cp "$APP_PKG/Resources/models.bin" "$APP/Contents/Resources/models.bin"
cp "$APP_PKG/Resources/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp -R "$APP_PKG/Resources/Fonts" "$APP/Contents/Resources/Fonts"
cp -R "$APP_PKG/Resources/Onboarding" "$APP/Contents/Resources/Onboarding"
# Scenes and their sound files (the scene picker; the app points ASMR_SOUNDS_DIR at sounds/).
mkdir -p "$APP/Contents/Resources/scenes" "$APP/Contents/Resources/sounds"
# The scenes the picker offers (Scenes.swift; the *_real variants on field recordings are
# kept in the repo for comparison but not shipped), and only the sound files they use.
# The .wav files are not in git: fetch them first (assets/sounds/SOURCES.md, tools/).
SCENES=(forest_river forest_waterfall underwater_drift space_still demo_flyby open_country)
for s in "${SCENES[@]}"; do cp "$ROOT/core/scenes/$s.json" "$APP/Contents/Resources/scenes/"; done
missing=$(cd "$APP/Contents/Resources/scenes" && grep -ho '"[^"]*\.wav"' *.json | tr -d '"' | sort -u | while read -r f; do
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
