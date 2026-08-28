#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

binary=${1:-target/release/hww}
out_dir=${2:-dist}

if [[ ! -x "$binary" ]]; then
    printf 'hww binary is missing or not executable: %s\n' "$binary" >&2
    exit 1
fi

# shellcheck source=packaging/version.sh
source packaging/version.sh
# shellcheck source=packaging/licenses.sh
source packaging/licenses.sh

# A plain `-git`, as in the zip. The tilde is dpkg's ordering rule and means nothing here.
version=${HWW_DMG_VERSION:-"${cargo_version}-git${revision}.${short_sha}"}

# Info.plist cannot carry that string, and this is the non-obvious half of packaging for this
# platform. `CFBundleShortVersionString` is the version a reader sees and Apple accepts one to
# three integers there and nothing else, so it takes the Cargo version alone.
# `CFBundleVersion` is meant to be a build counter that only ever increases, which is exactly
# what `version.sh` already computes as `revision`: the CI run number on a runner, the commit
# count off one. Neither key can hold the `-git<n>.<sha>` string, so the full spelling lives in
# the filename, where it always did.
short_version=$cargo_version
build_version=$revision

for tool in iconutil codesign hdiutil lipo python3; do
    if ! command -v "$tool" >/dev/null; then
        printf '%s is not on PATH; a macOS bundle cannot be built without it\n' "$tool" >&2
        exit 1
    fi
done

# The filename below claims an architecture, so ask the binary rather than the host. The CI
# action pins this by checking the rustc host triple, which says nothing about a local build on
# an Intel Mac or about a binary handed in as `$1`: either would produce a file called `aarch64`
# holding an app no Apple Silicon Mac runs natively, with nothing anywhere reporting it.
arch=$(lipo -archs "$binary")
if [[ "$arch" != arm64 ]]; then
    printf 'the image is named aarch64 and %s is %s\n' "$binary" "$arch" >&2
    exit 1
fi

# The image's root, laid out the way it will be seen when it mounts: the application beside a
# symlink to /Applications, which is the drag-to-install gesture every Mac reader already knows.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
chmod 0755 "$stage"

# Named once. The window layout below records icon positions against this name, and a name it
# does not find is a bundle Finder places wherever it likes.
bundle=hww.app
app=$stage/$bundle
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$binary" "$app/Contents/MacOS/hww"
chmod 0755 "$app/Contents/MacOS/hww"

# `CFBundleIdentifier` is pinned, not derived. macOS keys every per-app permission, window
# position, and defaults domain to this string, so changing it later silently resets all of
# them on every reader's Mac. Reverse-DNS of the repository's GitHub Pages domain, which is the
# closest thing this project has to a name it owns.
#
# `LSMinimumSystemVersion` is the reader-facing half of the floor `.cargo/config.toml` sets with
# `MACOSX_DEPLOYMENT_TARGET`; the two are one number and move together. Without it macOS lets an
# older system launch a binary built against a newer SDK and the failure is a crash on a missing
# symbol rather than a sentence saying which macOS this needs.
cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>hww</string>
	<key>CFBundleIdentifier</key>
	<string>io.github.tayler.hww</string>
	<key>CFBundleName</key>
	<string>hww</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleIconFile</key>
	<string>hww</string>
	<key>CFBundleShortVersionString</key>
	<string>$short_version</string>
	<key>CFBundleVersion</key>
	<string>$build_version</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
EOF
chmod 0644 "$app/Contents/Info.plist"

# The icon, built here rather than committed: `iconutil` exists only on macOS, and generating it
# on the runner from the committed rasters keeps a Mac out of the loop for anyone regenerating
# the logo. Every entry is *copied* from the `hww-N.png` drawn at that size rather than
# resampled from a larger one, which is the rule `assets/logo/README.md` already states for the
# `.ico` and holds here for the same reason: the 16 and the 32 are redrawn on their own grids.
#
# If `iconutil` ever refuses these rasters, the cause to check first is that they are colour
# type 2 — truecolour, no alpha — and the smallest fix is `sips -s format png` on each staged
# copy, which normalises the encoding and leaves the committed rasters untouched.
iconset=$stage/hww.iconset
mkdir -p "$iconset"
for entry in \
    16:icon_16x16 \
    32:icon_16x16@2x \
    32:icon_32x32 \
    64:icon_32x32@2x \
    128:icon_128x128 \
    256:icon_128x128@2x \
    256:icon_256x256 \
    512:icon_256x256@2x \
    512:icon_512x512 \
    1024:icon_512x512@2x; do
    size=${entry%%:*}
    name=${entry#*:}
    cp "assets/logo/hww-${size}.png" "$iconset/${name}.png"
    chmod 0644 "$iconset/${name}.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/hww.icns"
rm -rf "$iconset"

# Inside the bundle rather than beside it in the image. Readers drag the `.app` to /Applications
# and leave the image behind, so anything sitting next to it is lost, and the binary
# `include_bytes!`s every face in `fonts/`: a copy without those texts redistributes Atkinson
# Hyperlegible, IBM Plex, Noto, and DejaVu without their terms. `hww_install_licenses` runs
# `hww_check_font_licenses` first and returns non-zero if a face is uncovered or a text covers no
# face, which under `set -euo pipefail` ends this build before any image exists.
hww_install_licenses "$app/Contents/Resources"

# Last, because a signature seals everything under `Contents` — the licenses and the icon
# included — and re-signing is not optional on arm64: the kernel refuses to execute a Mach-O
# with no signature at all, and the one `cargo build`'s linker applied covers the binary before
# it was a bundle. This is ad-hoc: it makes the app runnable, and it does not satisfy Gatekeeper.
# See README.md's macOS section for what a reader sees on first launch as a result.
#
# `--timestamp=none` is not noise. An ad-hoc signature cannot be timestamped anyway, and saying
# so guarantees this step makes no network call, which is the argument the rest of this crate
# makes about itself.
codesign --force --sign - --timestamp=none "$app"

ln -s /Applications "$stage/Applications"

# Last thing into the folder, because it names what is already in it. Without this the volume
# opens at whatever size and view Finder last used for an unknown folder, with the sidebar and
# toolbar showing. See packaging/build-DS_Store.py for the records and for why Finder is not
# asked to lay the window out itself.
python3 packaging/build-DS_Store.py "$stage" "$bundle"
chmod 0644 "$stage/.DS_Store"

mkdir -p "$out_dir"
dmg="$out_dir/hww-${version}-aarch64-macos.dmg"
dmg_abs="$(cd "$out_dir" && pwd)/hww-${version}-aarch64-macos.dmg"
rm -f "$dmg_abs"

# `-srcfolder` still needs no attach step: the window layout is a file staged into the folder
# above, not Finder state that has to be applied to a mounted volume, so there is no attach,
# arrange, detach, convert dance here and no GUI session in the requirements. Unlike `zip`, it
# copies `Contents/_CodeSignature/CodeResources` intact, so the signature survives the trip.
#
# The volume name is bare `hww` rather than `hww $version`: HFS+ caps a volume name at 27
# characters and a development version already spends 24 of them. The version is in the filename.
hdiutil create -srcfolder "$stage" -volname hww -format UDZO -fs HFS+ -quiet "$dmg_abs"

printf '%s\n' "$dmg"
