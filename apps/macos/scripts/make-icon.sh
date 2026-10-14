#!/usr/bin/env bash
# Regenerates Resources/AppIcon.icns from the pixel lantern in Sources/Vigilia/Lantern.swift.
# The .icns is committed; run this again only when the lantern art changes.
set -euo pipefail
cd "$(dirname "$0")/.."

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

swiftc -O -o "$work/make-icon" Sources/Vigilia/Lantern.swift scripts/make-icon/main.swift
"$work/make-icon" "$work/AppIcon.iconset"
iconutil -c icns -o Resources/AppIcon.icns "$work/AppIcon.iconset"
echo "wrote Resources/AppIcon.icns"
