#!/bin/sh
# Builds Oorapp.app, signed with a fixed identity when there is one, so macOS
# keeps the microphone, system audio, Input Monitoring and Accessibility
# permissions from one build to the next. Without it every build is ad-hoc
# signed and macOS treats it as a new app.
#
#   APPLE_SIGNING_IDENTITY  a Developer ID, or the "Oorapp" certificate the
#                           releases are signed with (~/.config/oorapp/signing.p12)
set -e
cd "$(dirname "$0")/.."
if [ -z "$APPLE_SIGNING_IDENTITY" ] && security find-identity -p codesigning | grep -q '"Oorapp"'; then
  APPLE_SIGNING_IDENTITY=$(security find-identity -p codesigning | awk '/"Oorapp"/ { print $2; exit }')
  export APPLE_SIGNING_IDENTITY
fi
# The update package is signed with the updater key, when it is on this Mac.
if [ -z "$TAURI_SIGNING_PRIVATE_KEY" ] && [ -f "$HOME/.config/oorapp/updater.key" ]; then
  TAURI_SIGNING_PRIVATE_KEY=$(cat "$HOME/.config/oorapp/updater.key")
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
  export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
pnpm tauri build --bundles app

# --install: replace /Applications/Oorapp.app with this build and open it.
if [ "$1" = "--install" ] || [ "$2" = "--install" ]; then
  osascript -e 'quit app "Oorapp"' 2>/dev/null || true
  sleep 1
  rm -rf /Applications/Oorapp.app
  ditto target/release/bundle/macos/Oorapp.app /Applications/Oorapp.app
  open /Applications/Oorapp.app
fi
