# Shared by the Linux packaging scripts (bash counterpart of release-common.ps1).
# Source it; requires cargo, jq, curl and sha256sum.
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() { echo "error: $*" >&2; exit 1; }

# Validates a SemVer tag (lowercase v) against Cargo.toml and package.json,
# then prints the bare version.
release_version() {
    local tag=$1 n='(0|[1-9][0-9]*)' id='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
    local pattern="^v$n\\.$n\\.$n(-$id(\\.$id)*)?(\\+[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?$"
    [[ $tag =~ $pattern ]] || die "Expected a SemVer tag starting with lowercase v, got '$tag'."
    local version=${tag#v} cargo frontend
    cargo=$(cargo metadata --no-deps --locked --format-version 1 --manifest-path "$PROJECT_ROOT/Cargo.toml" |
        jq -r '.packages[] | select(.name == "miko-kagura") | .version')
    [ "$cargo" = "$version" ] || die "Tag $tag does not match Cargo.toml version $cargo."
    frontend=$(jq -r .version "$PROJECT_ROOT/package.json")
    [ "$frontend" = "$version" ] || die 'Update package.json to the same version as Cargo.toml.'
    echo "$version"
}

# Downloads an HTTPS URL into the cache (keyed by its SHA-256) and prints the path.
verified_download() {
    local url=$1 sha=$2 cache=$3
    [[ $sha =~ ^[a-f0-9]{64}$ ]] || die "Invalid SHA-256 for $url"
    [[ $url == https://* ]] || die "Expected an HTTPS source: $url"
    mkdir -p "$cache"
    local file="$cache/$sha-$(basename "${url%%\?*}")"
    if [ ! -f "$file" ]; then
        curl -fsSL --retry 3 --retry-delay 5 -o "$file.partial" "$url"
        if [ "$(sha256sum "$file.partial" | cut -d' ' -f1)" != "$sha" ]; then
            rm -f -- "$file.partial"
            die "SHA-256 mismatch: $url"
        fi
        mv -f -- "$file.partial" "$file"
    fi
    [ "$(sha256sum "$file" | cut -d' ' -f1)" = "$sha" ] || die "Corrupt cached download: $file"
    echo "$file"
}

# Linux counterpart of Assert-Payload: checks a portable payload directory.
assert_payload() {
    local dir=$1 edition=$2 lock="$PROJECT_ROOT/packaging/upscalers-linux-x64.lock.json" relative
    for relative in miko-kagura lib/libxdo.so.3 lib/libxdo3-copyright.txt LICENSE README.md README.es.md THIRD_PARTY_NOTICES.md assets \
        docs/en/installation.md docs/es/installation.md; do
        [ -e "$dir/$relative" ] || die "Missing payload file: $relative"
    done
    [ -x "$dir/miko-kagura" ] || die 'miko-kagura is not executable.'
    [ -n "$(find "$dir/assets" -type f -print -quit)" ] || die 'The assets directory is empty.'
    if [ "$edition" = lite ]; then
        [ ! -e "$dir/binaries" ] || die 'Lite unexpectedly contains upscaling engines.'
        return 0
    fi
    local count i pkg base file model param weights
    count=$(jq '.packages | length' "$lock")
    for ((i = 0; i < count; i++)); do
        pkg=$(jq -c ".packages[$i]" "$lock")
        base="$dir/binaries/$(jq -r .id <<<"$pkg")"
        [ -x "$base/$(jq -r .executable <<<"$pkg")" ] || die "Missing Full executable in $base"
        while IFS= read -r file; do
            [ -f "$base/$file" ] || die "Missing Full resource: $base/$file"
        done < <(jq -r '.requiredFiles[]' <<<"$pkg")
        while IFS= read -r model; do
            [ -n "$(find "$base/$model" -maxdepth 1 -name '*.param' -print -quit)" ] ||
                die "Missing model parameters: $base/$model"
            for param in "$base/$model"/*.param; do
                [ -f "${param%.param}.bin" ] || die "Missing model weights for $param"
            done
            for weights in "$base/$model"/*.bin; do
                [ -f "${weights%.bin}.param" ] || die "Missing model parameters for $weights"
            done
        done < <(jq -r '.modelDirectories[]' <<<"$pkg")
    done
    while IFS=$'\t' read -r file sha; do
        [ -f "$dir/binaries/licenses/$file" ] &&
            [ "$(sha256sum "$dir/binaries/licenses/$file" | cut -d' ' -f1)" = "$sha" ] ||
            die "Missing or altered notice: $file"
    done < <(jq -r '.notices[] | [.file, .sha256] | @tsv' "$lock")
}

# Debian sorts '~' before release: 1.2.0-beta.1 -> 1.2.0~beta.1.
deb_version() { local tilde='~'; echo "${1/-/$tilde}"; }
