#!/usr/bin/env bash
# Development loop for the menu bar app: removes every trace of the installed Vigilia, builds
# the app from this checkout, installs it in /Applications and opens it.
#
# Removed: any running session (stopped first, so normal sleep is restored), the app, the
# LaunchAgents (whichever `vigilia` they point at, the app's or a cargo-installed one), the
# sudoers rule and the /usr/local/bin/vigilia link (with sudo), the app's preferences, and
# ~/Library/Application Support/vigilia (config, journal, logs, socket). The cargo-installed
# CLI binary itself, if any, is left alone.
#
# Afterwards the app starts as on a new Mac: Settings opens on Setup, where you install the
# service and lid-closed sessions again.
#
#   -y, --yes        don't ask before removing anything
#   --keep-data      keep ~/Library/Application Support/vigilia (config such as the battery floor)
#   ARCHS=...        passed to build-app.sh; defaults to this Mac's arch for a quicker build
set -euo pipefail

app_dir="$(cd "$(dirname "$0")/.." && pwd)"
app=/Applications/Vigilia.app
data="$HOME/Library/Application Support/vigilia"
agents="$HOME/Library/LaunchAgents"
labels=(com.vigilia.service com.vigilia.recover com.vigilia.backstop)
sudoers=/etc/sudoers.d/vigilia
link=/usr/local/bin/vigilia

yes=false
keep_data=false
for arg in "$@"; do
    case "$arg" in
    -y | --yes) yes=true ;;
    --keep-data) keep_data=true ;;
    -h | --help)
        sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *)
        echo "unknown option: $arg (see --help)" >&2
        exit 2
        ;;
    esac
done

step() { printf '\n==> %s\n' "$*"; }

if ! $yes; then
    echo "This removes the installed Vigilia app, its service, the sudoers rule and the"
    echo "/usr/local/bin/vigilia link$($keep_data || echo ", and $data"),"
    echo "then builds and installs the app from this checkout."
    read -r -p "Continue? [y/N] " answer
    [[ "$answer" =~ ^[Yy]([Ee][Ss])?$ ]] || { echo "Aborted."; exit 0; }
fi

# Ask for the password now rather than halfway through.
if [[ -e "$sudoers" || -L "$link" ]]; then
    sudo -v
fi

step "Stopping any session"
# Ending the session while the sudoers rule is still there lets the service undo
# `pmset disablesleep`. With no service running there is no session, so don't start one.
cli=""
for candidate in "$app/Contents/Helpers/vigilia" "$(command -v vigilia || true)"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then
        cli="$candidate"
        break
    fi
done
if [[ -n "$cli" && -S "$data/vigilia.sock" ]]; then
    "$cli" stop 2>/dev/null || true
else
    echo "no service socket, nothing to stop"
fi

step "Quitting the app"
if pgrep -x Vigilia >/dev/null; then
    pkill -x Vigilia || true
    for _ in $(seq 1 50); do
        pgrep -x Vigilia >/dev/null || break
        sleep 0.1
    done
    pgrep -x Vigilia >/dev/null && pkill -9 -x Vigilia || true
else
    echo "not running"
fi

step "Removing the LaunchAgents"
for label in "${labels[@]}"; do
    plist="$agents/$label.plist"
    if [[ -f "$plist" ]]; then
        launchctl unload "$plist" 2>/dev/null || true
        rm -f "$plist"
        echo "removed $label"
    fi
done
# A service the app or the CLI started by hand isn't a launchd job; it has no session now.
pkill -f '/vigilia daemon$' 2>/dev/null || true

step "Removing the sudoers rule and the CLI link"
if [[ -e "$sudoers" ]]; then
    sudo rm -f "$sudoers"
    echo "removed $sudoers"
fi
if [[ -L "$link" ]]; then
    case "$(readlink "$link")" in
    */Contents/Helpers/vigilia)
        sudo rm -f "$link"
        echo "removed $link"
        ;;
    *) echo "kept $link: it doesn't point into a Vigilia app" ;;
    esac
fi

step "Removing the app and its data"
if [[ -d "$app" ]]; then
    rm -rf "$app"
    echo "removed $app"
fi
defaults delete com.vigilia.app 2>/dev/null && echo "removed preferences com.vigilia.app" || true
if ! $keep_data && [[ -d "$data" ]]; then
    rm -rf "$data"
    echo "removed $data"
fi

step "Building"
ARCHS="${ARCHS:-$(uname -m)}" "$app_dir/scripts/build-app.sh"

step "Installing"
ditto "$app_dir/dist/Vigilia.app" "$app"
echo "installed $app ($(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist"))"
open "$app"
echo "Opened Vigilia: Settings → Setup installs the service and lid-closed sessions again."
