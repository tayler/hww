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

# `~` sorts below everything in a Debian version, so a development package built after 0.1.0
# was tagged still upgrades cleanly to 0.1.0 itself. That ordering is dpkg's alone, which is
# why it is spelled here and not in version.sh.
version=${HWW_DEB_VERSION:-"${cargo_version}~git${revision}.${short_sha}"}
architecture=$(dpkg --print-architecture)

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
chmod 0755 "$stage"

install -Dm755 "$binary" "$stage/usr/bin/hww"
install -Dm644 packaging/hww.desktop "$stage/usr/share/applications/hww.desktop"
for size in 16 32 48 64 128 256 512; do
    install -Dm644 \
        "assets/logo/hww-${size}.png" \
        "$stage/usr/share/icons/hicolor/${size}x${size}/apps/hww.png"
done
hww_install_licenses "$stage/usr/share/doc/hww"

installed_size=$(du -sk "$stage" | cut -f1)
install -d "$stage/DEBIAN"
# `desktop-file-utils` is a dependency for one reason: its dpkg trigger is what refreshes the
# MIME cache, and without that refresh the `MimeType=` line in hww.desktop never reaches any
# other application's "Open with…" menu. It is installed on every desktop Ubuntu already, so it
# costs nothing in practice; the installs where it is absent are exactly the minimal ones where
# nothing else would run `update-desktop-database`, and there the door would fail silently and
# read as "hww does not do that".
cat >"$stage/DEBIAN/control" <<EOF
Package: hww
Version: $version
Section: web
Priority: optional
Architecture: $architecture
Maintainer: hww contributors
Installed-Size: $installed_size
Depends: libc6 (>= 2.35), libgcc-s1, desktop-file-utils
Description: quiet reading client for the non-app web
 hww reads stripped, server-rendered HTML without JavaScript, CSS, cookies,
 advertisements, or automatic third-party article-image requests.
EOF

mkdir -p "$out_dir"
deb="$out_dir/hww_${version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$stage" "$deb"
printf '%s\n' "$deb"
