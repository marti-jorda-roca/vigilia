#!/usr/bin/env bash
# `swift test` for this package. With only the Command Line Tools installed (no Xcode),
# SwiftPM finds neither the Swift Testing framework nor its runtime, so pass their paths.
# Extra arguments go to `swift test`, e.g. `scripts/test.sh --filter Countdown`.
set -euo pipefail
cd "$(dirname "$0")/.."

args=()
dev="$(xcode-select -p)"
if [[ "$dev" == */CommandLineTools ]]; then
    frameworks="$dev/Library/Developer/Frameworks"
    args+=(-Xswiftc -F -Xswiftc "$frameworks"
        -Xlinker -F -Xlinker "$frameworks"
        -Xlinker -rpath -Xlinker "$frameworks"
        -Xlinker -rpath -Xlinker "$dev/Library/Developer/usr/lib")
fi
exec swift test ${args[@]+"${args[@]}"} "$@"
