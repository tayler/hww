#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

binary=${1:-target/release/hww.exe}
out_dir=${2:-dist}

if [[ ! -f "$binary" ]]; then
    printf 'hww binary is missing: %s\n' "$binary" >&2
    exit 1
fi

# shellcheck source=packaging/version.sh
source packaging/version.sh
# shellcheck source=packaging/licenses.sh
source packaging/licenses.sh

# A plain `-git`, not the deb's `~git`. The tilde is dpkg's ordering rule and means nothing to
# anyone downloading a zip; here it would only be a character Windows readers have to squint at.
version=${HWW_ZIP_VERSION:-"${cargo_version}-git${revision}.${short_sha}"}
name="hww-${version}"

# Everything sits under one directory inside the archive. A Debian package decides where its
# own files land; a zip does not, and one that unpacks flat empties an executable and a stack of
# license texts into whatever folder the reader happened to be in. "Extract Here" is a real
# button that real people press.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name"
cp "$binary" "$stage/$name/hww.exe"
hww_install_licenses "$stage/$name"

mkdir -p "$out_dir"
zip="$out_dir/${name}-x86_64-windows.zip"
zip_abs="$(cd "$out_dir" && pwd)/${name}-x86_64-windows.zip"
rm -f "$zip_abs"

# 7-Zip ships on the GitHub Windows image and is the only zip tool on it that a POSIX shell can
# drive directly; Git Bash carries no `zip`. PowerShell's Compress-Archive is the fallback if
# that ever stops being true, and it has its own history of writing backslash-separated entry
# names, so it is not the default. Say which tool is missing rather than letting the shell say
# `command not found` about a name nobody chose to type.
if ! command -v 7z >/dev/null; then
    printf '7z is not on PATH; it is what builds the archive\n' >&2
    exit 1
fi
(cd "$stage" && 7z a -tzip -mx=9 -bso0 -bsp0 "$zip_abs" "$name" >/dev/null)

printf '%s\n' "$zip"
