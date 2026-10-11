#!/bin/sh
# Builds Oorapp.app, signed with a fixed identity when there is one, so macOS
# keeps the microphone, system audio, Input Monitoring and Accessibility
# permissions from one build to the next. Without it every build is ad-hoc
# signed and macOS treats it as a new app.
#
#   APPLE_SIGNING_IDENTITY  a Developer ID, or the local "Oorapp Local Signing"
set -e
cd "$(dirname "$0")/.."
if [ -z "$APPLE_SIGNING_IDENTITY" ] && security find-identity -p codesigning | grep -q "Oorapp Local Signing"; then
  APPLE_SIGNING_IDENTITY=$(security find-identity -p codesigning | awk '/Oorapp Local Signing/ { print $2; exit }')
  export APPLE_SIGNING_IDENTITY
fi
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
pnpm tauri build --bundles app "$@"
