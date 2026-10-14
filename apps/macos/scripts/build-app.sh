#!/usr/bin/env bash
# Builds Vigilia.app (the Swift app with the Rust `vigilia` binary inside) and a .dmg for it,
# into apps/macos/dist/. Both binaries are universal (arm64 + x86_64) unless ARCHS says otherwise.
#
#   SIGNING_IDENTITY    codesign identity; "-" (the default) signs ad-hoc
#   ARCHS               "arm64 x86_64" by default; ARCHS=arm64 for a quicker local build
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD
#                       when all three are set and the identity is real, the dmg is notarized
#                       and stapled
#
# Each arch is built on its own and joined with lipo: `swift build` with several --arch flags
# needs Xcode, and this also works with only the Command Line Tools.
set -euo pipefail

app_dir="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$app_dir/../.." && pwd)"
dist="$app_dir/dist"
identity="${SIGNING_IDENTITY:--}"
read -r -a archs <<<"${ARCHS:-arm64 x86_64}"

version="$(cd "$root" && cargo metadata --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "vigilia"))')"
echo "Building Vigilia $version for ${archs[*]}"

rm -rf "$dist"
mkdir -p "$dist/bin"
# The two binaries differ only in case, so they get distinct names until they are in the bundle.

# 1. The Rust helper.
helpers=()
for arch in "${archs[@]}"; do
    case "$arch" in
    arm64) target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *) echo "unknown arch: $arch" >&2; exit 2 ;;
    esac
    (cd "$root" && rustup target add "$target" >/dev/null)
    cargo build --manifest-path "$root/Cargo.toml" --release --locked -p vigilia --target "$target"
    helpers+=("$root/target/$target/release/vigilia")
done
lipo -create -output "$dist/bin/helper" "${helpers[@]}"

# 2. The app.
apps=()
for arch in "${archs[@]}"; do
    swift build --package-path "$app_dir" -c release --arch "$arch" --product Vigilia
    apps+=("$(swift build --package-path "$app_dir" -c release --arch "$arch" --show-bin-path)/Vigilia")
done
lipo -create -output "$dist/bin/app" "${apps[@]}"

# 3. The bundle. The helper goes in Helpers: MacOS/vigilia and MacOS/Vigilia would be the same
# file on a case-insensitive volume.
bundle="$dist/Vigilia.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Helpers" "$bundle/Contents/Resources"
cp "$dist/bin/app" "$bundle/Contents/MacOS/Vigilia"
cp "$dist/bin/helper" "$bundle/Contents/Helpers/vigilia"
cp "$app_dir/Resources/AppIcon.icns" "$bundle/Contents/Resources/AppIcon.icns"
cp "$app_dir/Resources/Info.plist" "$bundle/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$version" "$bundle/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$version" "$bundle/Contents/Info.plist"
rm -rf "${dist:?}/bin"

# 4. Sign, inside out. Ad-hoc signatures can't carry a secure timestamp.
sign=(codesign --force --options runtime --sign "$identity")
if [[ "$identity" != "-" ]]; then
    sign+=(--timestamp)
fi
"${sign[@]}" --identifier com.vigilia.helper "$bundle/Contents/Helpers/vigilia"
"${sign[@]}" "$bundle"
codesign --verify --strict --deep "$bundle"
# The helper must be the Rust binary, not a second copy of the app.
"$bundle/Contents/Helpers/vigilia" --version

# 5. The disk image: the app and a link to Applications to drag it onto.
dmg="$dist/Vigilia-$version.dmg"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp -R "$bundle" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -volname Vigilia -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg" >/dev/null
if [[ "$identity" != "-" ]]; then
    codesign --force --sign "$identity" --timestamp "$dmg"
fi

# 6. Notarize, when the credentials are there.
if [[ "$identity" != "-" && -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
    xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$dmg"
else
    echo "Not notarized: set SIGNING_IDENTITY, APPLE_ID, APPLE_TEAM_ID and APPLE_APP_PASSWORD to notarize."
fi

# 7. Checksum, with the bare file name so `shasum -c` works next to the download.
(cd "$dist" && shasum -a 256 "$(basename "$dmg")" >"$(basename "$dmg").sha256")

echo "Built $bundle"
echo "Built $dmg"
