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

cargo_version=$(
    awk -F ' *= *' '
        /^\[package\]$/ { package = 1; next }
        /^\[/ { package = 0 }
        package && /^version *=/ {
            gsub(/"/, "", $2)
            print $2
            exit
        }
    ' Cargo.toml
)
if [[ -z "$cargo_version" ]]; then
    printf 'could not read the package version from Cargo.toml\n' >&2
    exit 1
fi

revision=${GITHUB_RUN_NUMBER:-$(git rev-list --count HEAD)}
sha=${GITHUB_SHA:-$(git rev-parse HEAD)}
short_sha=${sha:0:7}
version=${HWW_DEB_VERSION:-"${cargo_version}+git${revision}.${short_sha}"}
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
install -Dm644 LICENSE-MIT "$stage/usr/share/doc/hww/LICENSE-MIT"
install -Dm644 LICENSE-APACHE "$stage/usr/share/doc/hww/LICENSE-APACHE"

installed_size=$(du -sk "$stage" | cut -f1)
install -d "$stage/DEBIAN"
cat >"$stage/DEBIAN/control" <<EOF
Package: hww
Version: $version
Section: web
Priority: optional
Architecture: $architecture
Maintainer: hww contributors
Installed-Size: $installed_size
Depends: libc6 (>= 2.35), libgcc-s1
Description: quiet reading client for the non-app web
 hww reads stripped, server-rendered HTML without JavaScript, CSS, cookies,
 advertisements, or automatic third-party article-image requests.
EOF

mkdir -p "$out_dir"
deb="$out_dir/hww_${version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$stage" "$deb"
printf '%s\n' "$deb"
