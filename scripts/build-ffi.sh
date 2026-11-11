#!/usr/bin/env bash
# Builds the Rust core for the macOS app: a static library to link, and the
# Swift bindings UniFFI generates from it. Xcode runs this before compiling
# (Debug or Release from $CONFIGURATION); it also runs by hand.
#
#   apps/macos/Generated/include/   C header and module map (module scry_ffiFFI)
#   apps/macos/Scry/Generated/      scry_ffi.swift, compiled with the app
#   Scry.app/Contents/Helpers/      git-scry, which the app's merge-tool
#                                   setting points git at (from Xcode only)
#
# The app links target/<profile>/libscry_ffi.a by its full path (see the
# Xcode project's OTHER_LDFLAGS).
set -euo pipefail

cd "$(dirname "$0")/.."
# Xcode starts scripts with a minimal PATH: add rustup's usual places.
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
# Built for the app's minimum macOS, to avoid linker warnings.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"

if [ "${CONFIGURATION:-Debug}" = "Release" ]; then
  profile=release
  cargo build --quiet -p scry-ffi -p git-scry --release
else
  profile=debug
  cargo build --quiet -p scry-ffi -p git-scry
fi

# The app carries its command-line tool, so the merge-tool setting can
# point git at a git-scry that exists, whatever the PATH.
if [ -n "${TARGET_BUILD_DIR:-}" ] && [ -n "${CONTENTS_FOLDER_PATH:-}" ]; then
  helpers="$TARGET_BUILD_DIR/$CONTENTS_FOLDER_PATH/Helpers"
  mkdir -p "$helpers"
  if ! cmp -s "target/$profile/git-scry" "$helpers/git-scry"; then
    cp "target/$profile/git-scry" "$helpers/git-scry"
    codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY:--}" "$helpers/git-scry"
  fi
fi

out=apps/macos/Generated
staging="$out/staging"
rm -rf "$staging"
mkdir -p "$staging" "$out/include" apps/macos/Scry/Generated
cargo run --quiet -p scry-ffi --bin uniffi-bindgen -- \
  generate --library "target/$profile/libscry_ffi.dylib" \
  --language swift --out-dir "$staging" > /dev/null

# Replace files only when they change, so Xcode does not recompile for nothing.
update() {
  if ! cmp -s "$1" "$2"; then cp "$1" "$2"; fi
}
update "$staging/scry_ffi.swift" apps/macos/Scry/Generated/scry_ffi.swift
update "$staging/scry_ffiFFI.h" "$out/include/scry_ffiFFI.h"
update "$staging/scry_ffiFFI.modulemap" "$out/include/module.modulemap"
rm -rf "$staging"
