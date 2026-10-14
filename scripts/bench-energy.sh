#!/usr/bin/env bash
# Measures what keep-awake tools cost while they keep the Mac awake: CPU time, idle wake-ups,
# macOS energy impact and memory of each tool's own processes, first during a session, then
# with no session. Staying awake costs the same with every tool; this measures the tool itself.
# Needs no sudo: the numbers come from `top -c a`.
#
#   scripts/bench-energy.sh [options] [TOOL...]
#
# Tools (default: every one that is installed; missing ones are skipped):
#   vigilia          Vigilia.app: the menu bar app and the service, sessions through its CLI
#   vigilia-cli      the service alone, with the menu bar app quit
#   caffeinate       built into macOS (`caffeinate -i`)
#   keepingyouawake  KeepingYouAwake.app (`brew install --cask keepingyouawake`)
#   caffeine         Caffeine.app by domzilla
#   amphetamine      Amphetamine.app (App Store). Open it once first; macOS asks once to let
#                    Terminal control it.
#
#   --session SECS   length of the session window (default 300)
#   --idle SECS      length of the no-session window (default 120)
#   --settle SECS    wait after launching an app, before measuring it (default 60)
#   --quick          --session 120 --idle 60 --settle 30
#   --out DIR        where results go (default target/bench-energy/<date>-<time>)
#
# Before it starts, it checks that no Vigilia session is running and quits the apps under
# test, so every tool starts clean. During the run it checks that each tool holds a sleep
# assertion in its session window and none in its idle window, and marks a window invalid
# otherwise. Writes summary.md (machine, versions, a results table), results.csv and the raw
# `top` samples. Apps that were open before the run are reopened afterwards.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
apps=/Applications
vigilia_app="$apps/Vigilia.app"
all_tools=(vigilia vigilia-cli caffeinate keepingyouawake caffeine amphetamine)

session_s=300
idle_s=120
settle_s=60
out=""
tools=()
while (($#)); do
    case "$1" in
    --session) session_s="$2"; shift ;;
    --idle) idle_s="$2"; shift ;;
    --settle) settle_s="$2"; shift ;;
    --quick) session_s=120; idle_s=60; settle_s=30 ;;
    --out) out="$2"; shift ;;
    -h | --help)
        sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    -*)
        echo "unknown option: $1 (see --help)" >&2
        exit 2
        ;;
    *) tools+=("$1") ;;
    esac
    shift
done

step() { printf '\n==> %s\n' "$*"; }
note() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*"; }
die() { echo "error: $*" >&2; exit 1; }

[[ "$(uname)" == Darwin ]] || die "this benchmark only runs on macOS"

# --- Which tools, and are they here --------------------------------------------------------

vigilia_cli() {
    # The app's own CLI when the app is installed, so the CLI and the service match.
    if [[ -x "$vigilia_app/Contents/Helpers/vigilia" ]]; then
        echo "$vigilia_app/Contents/Helpers/vigilia"
    else
        command -v vigilia || true
    fi
}

app_of() {
    case "$1" in
    vigilia) echo Vigilia ;;
    keepingyouawake) echo KeepingYouAwake ;;
    caffeine) echo Caffeine ;;
    amphetamine) echo Amphetamine ;;
    *) echo "" ;;
    esac
}

installed() {
    case "$1" in
    vigilia) [[ -d "$vigilia_app" ]] ;;
    vigilia-cli) [[ -n "$(vigilia_cli)" ]] ;;
    caffeinate) [[ -x /usr/bin/caffeinate ]] ;;
    *) [[ -d "$apps/$(app_of "$1").app" ]] ;;
    esac
}

version_of() {
    case "$1" in
    vigilia-cli) "$(vigilia_cli)" --version 2>/dev/null | awk '{print $2}' ;;
    caffeinate) echo "macOS $(sw_vers -productVersion)" ;;
    *) defaults read "$apps/$(app_of "$1").app/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo "?" ;;
    esac
}

if ((${#tools[@]} == 0)); then
    tools=("${all_tools[@]}")
    explicit=false
else
    explicit=true
fi
selected=()
for tool in "${tools[@]}"; do
    [[ " ${all_tools[*]} " == *" $tool "* ]] || die "unknown tool: $tool (see --help)"
    if installed "$tool"; then
        selected+=("$tool")
    elif $explicit; then
        die "$tool is not installed"
    else
        echo "skipping $tool: not installed"
    fi
done
((${#selected[@]} > 0)) || die "none of the tools is installed"

# --- Clean start ---------------------------------------------------------------------------

cli="$(vigilia_cli)"
if [[ " ${selected[*]} " == *" vigilia"* && -n "$cli" ]] && "$cli" status --json 2>/dev/null | grep -q '"state":"Active"'; then
    die "a Vigilia session is running ($("$cli" status 2>/dev/null)). End it with \`vigilia stop\` first: it would hold the Mac awake through every window and skew Vigilia's numbers."
fi

# Remember which apps were open, to reopen them at the end, then quit them all.
was_running=()
for tool in "${selected[@]}"; do
    app="$(app_of "$tool")"
    if [[ -n "$app" ]] && pgrep -xq "$app"; then
        was_running+=("$app")
    fi
done

quit_app() {
    local app="$1"
    pgrep -xq "$app" || return 0
    osascript -e "quit app \"$app\"" >/dev/null 2>&1 || true
    for _ in $(seq 1 50); do
        pgrep -xq "$app" || return 0
        sleep 0.2
    done
    pkill -x "$app" || true
}

for app in "${was_running[@]+"${was_running[@]}"}"; do
    note "quitting $app so it starts clean"
    quit_app "$app"
done

caffeine_prefs=""
cleanup() {
    [[ -n "${current:-}" ]] && { stop_session "$current" 2>/dev/null || true; }
    for app in Vigilia KeepingYouAwake Caffeine Amphetamine; do
        [[ " ${selected[*]} " == *" $(echo "$app" | tr '[:upper:]' '[:lower:]') "* ]] && quit_app "$app"
    done
    if [[ -n "$caffeine_prefs" ]]; then
        defaults import net.domzilla.caffeine "$caffeine_prefs" 2>/dev/null || true
    fi
    for app in "${was_running[@]+"${was_running[@]}"}"; do
        open -g -a "$app" || true
    done
}
trap cleanup EXIT

# --- Driving each tool ---------------------------------------------------------------------

caff_pid=""

pids_for() {
    local p
    case "$1" in
    vigilia) pgrep -f "$vigilia_app/Contents/" || true ;;
    vigilia-cli) pgrep -f "vigilia daemon" || true ;;
    caffeinate) if [[ -n "$caff_pid" ]] && kill -0 "$caff_pid" 2>/dev/null; then echo "$caff_pid"; fi ;;
    *)
        # The app and anything it spawned (KeepingYouAwake runs caffeinate as a child).
        for p in $(pgrep -f "$apps/$(app_of "$1").app/Contents/MacOS/" || true); do
            echo "$p"
            pgrep -P "$p" || true
        done
        ;;
    esac | sort -u
}

launch() {
    case "$1" in
    vigilia | keepingyouawake | amphetamine)
        open -g -a "$(app_of "$1")"
        sleep "$settle_s"
        ;;
    vigilia-cli) quit_app Vigilia ;;
    caffeine)
        # Caffeine has no scripting interface: it's turned on by launching it with "activate
        # at launch" set, and off by quitting it. Its preferences are restored at the end.
        if [[ -z "$caffeine_prefs" ]]; then
            caffeine_prefs="$out/caffeine-prefs.plist"
            defaults export net.domzilla.caffeine "$caffeine_prefs" 2>/dev/null || caffeine_prefs=""
        fi
        defaults write net.domzilla.caffeine CASuppressLaunchMessage -bool true
        ;;
    esac
}

start_session() {
    case "$1" in
    vigilia | vigilia-cli) "$cli" start 2h >/dev/null ;;
    caffeinate)
        caffeinate -i -t 7200 >/dev/null 2>&1 &
        caff_pid=$!
        ;;
    keepingyouawake) open -g "keepingyouawake:///activate" ;;
    caffeine)
        defaults write net.domzilla.caffeine CAActivateAtLaunch -bool true
        open -g -a Caffeine
        ;;
    amphetamine)
        osascript -e 'tell application "Amphetamine" to start new session with options {duration:2, interval:hours, displaySleepAllowed:true}' >/dev/null
        ;;
    esac
    # Starting Caffeine launches it, so it gets the same settle time as the other apps.
    if [[ "$1" == caffeine ]]; then sleep "$settle_s"; else sleep 5; fi
}

stop_session() {
    case "$1" in
    vigilia | vigilia-cli) "$cli" stop >/dev/null 2>&1 || true ;;
    caffeinate)
        [[ -n "$caff_pid" ]] && { kill "$caff_pid" 2>/dev/null || true; wait "$caff_pid" 2>/dev/null || true; }
        caff_pid=""
        ;;
    keepingyouawake) open -g "keepingyouawake:///deactivate" ;;
    caffeine)
        quit_app Caffeine
        defaults write net.domzilla.caffeine CAActivateAtLaunch -bool false
        open -g -a Caffeine
        sleep "$settle_s"
        ;;
    amphetamine) osascript -e 'tell application "Amphetamine" to end session' >/dev/null 2>&1 || true ;;
    esac
}

after_stop() {
    case "$1" in
    vigilia | vigilia-cli)
        # The service exits 5 minutes after its last session ends: measure the state it settles in.
        note "waiting up to 330s for the Vigilia service to exit"
        for _ in $(seq 1 66); do
            pgrep -fq "vigilia daemon" || break
            sleep 5
        done
        ;;
    caffeine) ;;
    *) sleep 10 ;;
    esac
}

teardown() {
    local app
    app="$(app_of "$1")"
    [[ -n "$app" ]] && quit_app "$app"
    if [[ "$1" == caffeine ]]; then
        defaults write net.domzilla.caffeine CAActivateAtLaunch -bool false
    fi
    sleep 3
}

# Sleep assertions held by these pids, one per line.
assertions_of() {
    local pids="$1" p
    for p in $pids; do
        pmset -g assertions | grep -E "pid $p\(" | grep -E "Prevent(UserIdle)?(System|Display)Sleep|NoIdleSleep" || true
    done
}

# --- Measuring -----------------------------------------------------------------------------

[[ -n "$out" ]] || out="$repo/target/bench-energy/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out/raw"
csv="$out/results.csv"
echo "tool,version,phase,seconds,processes,cpu_seconds,idle_wakeups,avg_energy_impact,memory_mb,holds_assertion,valid" > "$csv"

# measure TOOL PHASE SECONDS: one row of results.csv
measure() {
    local tool="$1" phase="$2" secs="$3" raw="$out/raw/$1-$2"
    local pids holds valid args=() p
    pids="$(pids_for "$tool" | tr '\n' ' ')"
    holds="$(assertions_of "$pids")"
    printf '%s\n' "$holds" > "$raw.assertions"
    [[ -n "${pids// /}" ]] && ps -o pid=,command= -p "$(echo "$pids" | xargs | tr ' ' ,)" > "$raw.procs" 2>/dev/null || true
    if [[ "$phase" == session ]]; then
        [[ -n "$holds" ]] && valid=yes || valid="no: no sleep assertion held"
    else
        [[ -z "$holds" ]] && valid=yes || valid="no: still holds a sleep assertion"
    fi
    if [[ -z "${pids// /}" ]]; then
        sleep "$secs"
        echo "$tool,$(version_of "$tool"),$phase,$secs,0,0.00,0,0.000,0.0,$([[ -n "$holds" ]] && echo yes || echo no),$valid" >> "$csv"
    else
        for p in $pids; do args+=(-pid "$p"); done
        top -l $((secs + 1)) -s 1 -c a -stats pid,command,cpu,idlew,power,mem,time "${args[@]}" > "$raw.top" 2>&1
        awk -v tool="$tool" -v ver="$(version_of "$tool")" -v phase="$phase" -v secs="$secs" \
            -v n="$(echo "$pids" | wc -w | tr -d ' ')" -v holds="$([[ -n "$holds" ]] && echo yes || echo no)" -v valid="$valid" '
            function mb(v,   u, x) { u = substr(v, length(v)); x = v + 0; return u == "K" ? x / 1024 : (u == "G" ? x * 1024 : x) }
            function sec(t,   a, k) { k = split(t, a, ":"); return k == 3 ? a[1] * 3600 + a[2] * 60 + a[3] : a[1] * 60 + a[2] }
            /^PID/ { sample++; next }
            $1 ~ /^[0-9]+$/ && sample > 0 {
                gsub(/[+-]$/, "", $4); gsub(/[+-]$/, "", $6)
                cpu[$1] = sec($7); wake[$1] = $4 + 0; mem[$1] = mb($6)
                if (sample > 1) power[sample] += $5
            }
            END {
                for (p in cpu) { c += cpu[p]; w += wake[p]; m += mem[p] }
                for (s in power) { ps += power[s]; ns++ }
                printf "%s,%s,%s,%d,%d,%.2f,%d,%.3f,%.1f,%s,%s\n", tool, ver, phase, secs, n, c, w, (ns ? ps / ns : 0), m, holds, valid
            }' "$raw.top" >> "$csv"
    fi
    note "$tool $phase: $(tail -1 "$csv")"
}

total=$(((session_s + idle_s + settle_s + 30) * ${#selected[@]}))
step "Measuring ${selected[*]}"
echo "about $((total / 60)) minutes, plus up to 5.5 minutes for each Vigilia tool; results in $out"
echo "The Mac can be used meanwhile: only each tool's own processes are counted."

for tool in "${selected[@]}"; do
    current="$tool"
    step "$tool $(version_of "$tool")"
    launch "$tool"
    start_session "$tool"
    measure "$tool" session "$session_s"
    stop_session "$tool"
    after_stop "$tool"
    measure "$tool" idle "$idle_s"
    teardown "$tool"
    current=""
done

# --- Summary -------------------------------------------------------------------------------

summary="$out/summary.md"
{
    echo "# Keep-awake overhead, $(date '+%Y-%m-%d %H:%M')"
    echo
    echo "Mac: $(sysctl -n hw.model), $(sysctl -n machdep.cpu.brand_string), macOS $(sw_vers -productVersion)."
    echo "Power: $(pmset -g batt | head -1 | sed "s/Now drawing from //; s/'//g")."
    echo "Windows: ${session_s}s with a session, ${idle_s}s without; apps settle ${settle_s}s after launch."
    echo "Counts each tool's own processes only, from \`top -c a\`. CPU and wake-ups are totals over the window;"
    echo "energy impact is macOS's per-process score, averaged over the window's 1-second samples."
    echo
    echo "| Tool | Version | Processes | CPU (s) | Idle wake-ups | Wake-ups/min | Energy impact | Memory (MB) | Valid |"
    echo "|---|---|---|---|---|---|---|---|---|"
    for phase in session idle; do
        echo "| **${phase}** | | | | | | | | |"
        awk -F, -v phase="$phase" 'NR > 1 && $3 == phase {
            printf "| %s | %s | %d | %.2f | %d | %.1f | %.3f | %.1f | %s |\n", $1, $2, $5, $6, $7, $7 * 60 / $4, $8, $9, $11
        }' "$csv"
    done
} > "$summary"

step "Done"
cat "$summary"
echo
echo "Summary: $summary"
