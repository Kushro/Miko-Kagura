#!/usr/bin/env bash
# Linux counterpart of fetch-upscalers.ps1: downloads the hash-pinned engines
# from packaging/upscalers-linux-x64.lock.json. Requires curl, jq, unzip, sha256sum.
# Usage: scripts/fetch-upscalers.sh [destination] [cache-directory]
set -euo pipefail
. "$(dirname "$0")/release-common.sh"

DEST="$(realpath -m "${1:-$PROJECT_ROOT/target/release-packaging-linux/binaries}")"
CACHE="${2:-$PROJECT_ROOT/target/download-cache}"
LOCK="$PROJECT_ROOT/packaging/upscalers-linux-x64.lock.json"
[ ! -e "$DEST" ] || die "Destination must not exist: $DEST"
mkdir -p "$DEST"

count=$(jq '.packages | length' "$LOCK")
for ((i = 0; i < count; i++)); do
    pkg=$(jq -c ".packages[$i]" "$LOCK")
    field() { jq -r "$1" <<<"$pkg"; }
    id=$(field .id) exe=$(field .executable) root=$(field .archiveRoot)
    echo "Fetching $(field .repository) $(field .version)"
    archive=$(verified_download "$(field .url)" "$(field .sha256)" "$CACHE")
    extraction=$(mktemp -d)
    unzip -q "$archive" -d "$extraction"
    src="$extraction/$root"
    [ -f "$src/$exe" ] || die "Unexpected upstream archive layout: $id"
    while IFS= read -r excluded; do
        rm -f -- "${src:?}/$excluded"
    done < <(jq -r '.exclude // [] | .[]' <<<"$pkg")
    mkdir "$DEST/$id"
    cp -a "$src/." "$DEST/$id/"
    # Some upstream zips (Real-ESRGAN) were made on Windows and drop the exec bit.
    chmod 755 "$DEST/$id/$exe"
    rm -rf -- "${extraction:?}"
done

mkdir "$DEST/licenses"
while IFS=$'\t' read -r file url sha; do
    cp "$(verified_download "$url" "$sha" "$CACHE")" "$DEST/licenses/$file"
done < <(jq -r '.notices[] | [.file, .url, .sha256] | @tsv' "$LOCK")
cp "$LOCK" "$DEST/upscalers.lock.json"
