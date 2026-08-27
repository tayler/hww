# What license texts a distribution has to carry, for whoever is packaging it.
#
# Sourced, not executed, by every packaging script, so a new distribution format inherits the
# full set by writing one line instead of rediscovering it. The crate's own license is the
# obvious half. The other half is less obvious and easier to ship without: `fonts.rs` compiles
# every face in `fonts/` into the binary with `include_bytes!`, so a bare `hww` binary is
# already a redistribution of Atkinson Hyperlegible, IBM Plex, Noto, and DejaVu, and the OFL
# requires their copyright and license to travel with the copy. A package with no font licenses
# is not merely untidy; it is distributing those fonts without their terms.
#
# The set is a glob rather than a list. A face added to `fonts/` brings a `LICENSE-*.txt`, and
# `hww_check_font_licenses` fails the build when it does not, which is what keeps a future
# format honest without anyone editing this file.

# Every font license text in the repository, one per line.
hww_font_licenses() {
    local license found=0
    for license in fonts/LICENSE-*.txt; do
        [[ -f "$license" ]] || continue
        printf '%s\n' "$license"
        found=1
    done
    if (( ! found )); then
        printf 'no font licenses found in fonts/; the binary embeds the faces\n' >&2
        return 1
    fi
}

# Fail unless every embedded face is covered by a license text and every license text still has
# a face. The two names are matched by prefix: `LICENSE-IBMPlex.txt` covers `IBMPlexMono-Bold`
# and `IBMPlexSerif-Italic`, `LICENSE-NotoSans.txt` covers the Devanagari and Thai subsets
# without claiming `NotoEmoji-Regular`, which carries its own. A face whose file name does not
# begin with the tag of some license fails here rather than shipping uncovered.
hww_check_font_licenses() {
    local status=0 font license tag base covered used

    for font in fonts/*.ttf; do
        [[ -f "$font" ]] || continue
        base=$(basename "$font")
        covered=0
        for license in fonts/LICENSE-*.txt; do
            [[ -f "$license" ]] || continue
            tag=$(basename "$license" .txt)
            tag=${tag#LICENSE-}
            [[ "$base" == "$tag"* ]] && covered=1
        done
        if (( ! covered )); then
            printf '%s is embedded in the binary and no fonts/LICENSE-*.txt covers it\n' \
                "$font" >&2
            status=1
        fi
    done

    for license in fonts/LICENSE-*.txt; do
        [[ -f "$license" ]] || continue
        tag=$(basename "$license" .txt)
        tag=${tag#LICENSE-}
        used=0
        for font in fonts/*.ttf; do
            [[ -f "$font" ]] || continue
            [[ "$(basename "$font")" == "$tag"* ]] && used=1
        done
        if (( ! used )); then
            printf '%s covers no font in fonts/; remove it or the face it names is missing\n' \
                "$license" >&2
            status=1
        fi
    done

    return "$status"
}

# Copy every license text into a staged package rooted at $1. `LICENSE` and `NOTICE` sit at the
# root the reader lands on; the font texts sit under `fonts/` beside them, in the same shape in
# every distribution, so one answer to "which fonts is this and under what terms" fits all of
# them. `NOTICE` is not decoration: it carries the copyright line and the section 7 term about
# the name and the logo, and a package that ships `LICENSE` alone states neither to the only
# person who needs them, who has the binary and not the repository.
hww_install_licenses() {
    local root=$1 license list

    hww_check_font_licenses || return 1

    # Collected before the loop rather than piped into it. A `done < <(hww_font_licenses)`
    # discards that function's exit status, so its empty-`fonts/` guard could not fail a build:
    # the body would simply not run and this would return 0 having installed no font licenses.
    # `hww_check_font_licenses` passes vacuously on an empty `fonts/` — both of its loops skip —
    # so this is the only thing standing between an empty `fonts/` and a package that ships the
    # crate's own two files and nothing else.
    list=$(hww_font_licenses) || return 1

    mkdir -p "$root"
    cp LICENSE NOTICE "$root/"
    chmod 0644 "$root/LICENSE" "$root/NOTICE"

    mkdir -p "$root/fonts"
    while IFS= read -r license; do
        [[ -n "$license" ]] || continue
        cp "$license" "$root/fonts/"
        chmod 0644 "$root/fonts/$(basename "$license")"
    done <<<"$list"
}
