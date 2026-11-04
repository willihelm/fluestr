#!/usr/bin/env bash
# The release notes for a tag, from the commit subjects since the tag before it.
#
# Usage: scripts/release-notes.sh v0.3.0
#
# The commits follow Conventional Commits, so the type already says which list
# a change belongs in. Only what a user of the app notices is listed — `feat`,
# `fix` and `perf`; `ci`, `chore`, `docs`, `refactor`, `test` and `build` are
# left out, because "cache sherpa-onnx in CI" is noise in a changelog that pops
# up in front of somebody setting up a show. The scope is dropped for the same
# reason — `remote` means something here, not to them — and so is a trailing
# plan reference like `(P26)`.
#
# These notes are what the release page shows and what `latest.json` carries
# into the app's changelog dialog (`ui/src/lib/ChangelogDialog.svelte`), which
# reads `### ` headings and `- ` items and nothing else.

set -euo pipefail

tag="${1:?usage: release-notes.sh <tag>}"
previous=$(git describe --tags --abbrev=0 "$tag^" 2>/dev/null || true)
range="${previous:+$previous..}$tag"

subjects=$(git log --no-merges --format='%s' "$range")

section() {
    local title="$1" pattern="$2" items
    # `type(scope)!: subject` -> `Subject`
    items=$(printf '%s\n' "$subjects" |
        grep -E "^($pattern)(\([^)]*\))?!?: " |
        sed -E -e 's/^[a-z]+(\([^)]*\))?!?: +//' -e 's/ \(P[0-9]+\)$//' |
        awk '{ print "- " toupper(substr($0, 1, 1)) substr($0, 2) }' || true)
    if [ -n "$items" ]; then
        printf '### %s\n\n%s\n' "$title" "$items"
    fi
}

sections=()
for spec in "New:feat" "Fixed:fix" "Faster:perf"; do
    text=$(section "${spec%%:*}" "${spec#*:}")
    [ -n "$text" ] && sections+=("$text")
done

if [ ${#sections[@]} -eq 0 ]; then
    echo "Maintenance release with no visible changes."
    exit 0
fi
# A blank line between the sections.
printf '%s\n' "${sections[0]}"
for text in "${sections[@]:1}"; do
    printf '\n%s\n' "$text"
done
