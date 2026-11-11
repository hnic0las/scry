#!/usr/bin/env bash
# Builds Scry in Release and installs it for daily use, without Xcode:
#
#   scripts/install-app.sh
#
# - /Applications/Scry.app (or $SCRY_INSTALL_DIR/Scry.app), replacing the
#   one there;
# - `git-scry`, so `git scry` works: a link to the app's own git-scry in the
#   first of ~/.local/bin, /opt/homebrew/bin or /usr/local/bin that is on
#   your PATH and writable;
# - when Scry already is git's merge tool, its command now runs this copy.
#   Otherwise, Scry › Settings › CLI › Install sets it up.
#
# The app is signed ad hoc and built on this Mac, so Gatekeeper lets it
# open; it is not notarised for other Macs.
set -euo pipefail

workspace="$(cd "$(dirname "$0")/.." && pwd)"
destination="${SCRY_INSTALL_DIR:-/Applications}"
app="$destination/Scry.app"
cd "$workspace"

if pgrep -f "$app/Contents/MacOS/Scry" > /dev/null; then
  echo "Scry is running from $app: quit it, then run this again." >&2
  exit 1
fi

echo "Building Scry (Release)…"
derived="$workspace/target/xcode"
xcodebuild -project apps/macos/Scry.xcodeproj -scheme Scry -configuration Release \
  -destination 'platform=macOS' -derivedDataPath "$derived" -quiet build
built="$derived/Build/Products/Release/Scry.app"

echo "Installing $app"
rm -rf "$app"
ditto "$built" "$app"
cli="$app/Contents/Helpers/git-scry"

# The first suitable folder, in the order the PATH lists them.
link_dir=""
IFS=: read -r -a path_dirs <<< "$PATH"
for dir in "${path_dirs[@]}"; do
  case "$dir" in
    "$HOME/.local/bin" | /opt/homebrew/bin | /usr/local/bin)
      if [ -d "$dir" ] && [ -w "$dir" ]; then
        link_dir="$dir"
        break
      fi
      ;;
  esac
done
if [ -n "$link_dir" ]; then
  ln -sf "$cli" "$link_dir/git-scry"
  echo "Linked $link_dir/git-scry → $cli"
  first="$(command -v git-scry || true)"
  if [ -n "$first" ] && [ "$first" != "$link_dir/git-scry" ]; then
    echo "Note: $first comes first on your PATH and is used instead." >&2
  fi
else
  echo "No writable ~/.local/bin, /opt/homebrew/bin or /usr/local/bin on your PATH:" >&2
  echo "  ln -s '$cli' <a folder on your PATH>/git-scry" >&2
fi

# The same command the app's settings install, so they show it as current.
if [ "$(git config --global --get merge.tool || true)" = scry ]; then
  git config --global mergetool.scry.cmd \
    "\"$cli\" resolve --app=\"$app\" \"\$BASE\" \"\$LOCAL\" \"\$REMOTE\" \"\$MERGED\""
  git config --global mergetool.scry.trustExitCode true
  echo "git mergetool now opens $app"
else
  echo "To resolve with git mergetool: Scry › Settings › CLI › Install."
fi

echo
echo "Done. In a repository with a merge or rebase in progress, run: git scry"
