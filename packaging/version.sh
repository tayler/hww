# What version a commit is, for whoever is packaging it.
#
# Sourced, not executed. Both packaging scripts read the same three values here so a deb and a
# zip built from one commit cannot disagree about what commit that was. How each spells the
# result is its own business: dpkg orders `~git` below the release it precedes, and that rule
# has no meaning outside dpkg, so it stays in build-deb.sh.
#
# In CI the revision is the run number, which increases for every build of every commit. Off a
# runner it is the commit count, which is the closest thing a checkout has to the same idea.

hww_cargo_version() {
    awk -F ' *= *' '
        /^\[package\]$/ { package = 1; next }
        /^\[/ { package = 0 }
        package && /^version *=/ {
            gsub(/"/, "", $2)
            print $2
            exit
        }
    ' Cargo.toml
}

cargo_version=$(hww_cargo_version)
if [[ -z "$cargo_version" ]]; then
    printf 'could not read the package version from Cargo.toml\n' >&2
    exit 1
fi

revision=${GITHUB_RUN_NUMBER:-$(git rev-list --count HEAD)}
sha=${GITHUB_SHA:-$(git rev-parse HEAD)}
short_sha=${sha:0:7}
