#!/usr/bin/env bash
# Installs the Linux packages from dist/<tag>-linux in disposable Docker containers
# and checks that every ELF resolves its libraries and that the app starts and
# finds its bundled engines (GET /health -> "upscaler_ready":true) under Xvfb:
#   ubuntu:24.04   the .deb (dependencies from its Depends) and the AppImage
#   archlinux      the AUR PKGBUILD built with makepkg from the local tarball
# Writes the tested PKGBUILD and its .SRCINFO to <aur-out-dir>.
# Usage: scripts/test-linux-packages.sh <tag> [aur-out-dir]   (DOCKER=podman to switch engines)
set -euo pipefail
. "$(dirname "$0")/release-common.sh"

TAG=${1:?usage: test-linux-packages.sh <tag> [aur-out-dir]}
DIST="$PROJECT_ROOT/dist/$TAG-linux"
PREFIX="Miko-Kagura-$TAG-linux-x64"
AUR_OUT="$(realpath -m "${2:-$PROJECT_ROOT/target/aur/miko-kagura-bin}")"
DOCKER=${DOCKER:-docker}
command -v "$DOCKER" >/dev/null || die "$DOCKER is required."
[ -f "$DIST/SHA256SUMS.txt" ] || die "Build the packages first: $DIST"

# Shared by both containers: fail on unresolved libraries, then start the app
# headless and wait for the server it spawns before opening the window.
read -r -d '' CHECKS <<'EOF' || true
check_libs() {
    local missing
    missing=$(find "$1" -type f -perm -u+x -exec sh -c 'head -c4 "$1" | grep -q ELF' _ {} \; \
        -exec ldd {} \; 2>&1 | grep 'not found' | sort -u || true)
    [ -z "$missing" ] || { echo "unresolved libraries under $1:"; echo "$missing"; exit 1; }
}
smoke() {
    local port=$1; shift
    LIBGL_ALWAYS_SOFTWARE=1 WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 \
        xvfb-run -a "$@" --port "$port" >/tmp/app-$port.log 2>&1 &
    local pid=$! body=
    for _ in $(seq 60); do
        body=$(curl -fsS "http://127.0.0.1:$port/health" 2>/dev/null) && break
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
    done
    kill "$pid" 2>/dev/null || true
    echo "health on $port: ${body:-<no response>}"
    if ! grep -q '"upscaler_ready":true' <<<"$body"; then
        cat /tmp/app-$port.log; exit 1
    fi
}
EOF

echo '── ubuntu:24.04: .deb + AppImage'
"$DOCKER" run --rm -i -e CHECKS="$CHECKS" -e PREFIX="$PREFIX" \
    -v "$DIST:/dist:ro" ubuntu:24.04 bash -s <<'EOF'
set -euo pipefail
eval "$CHECKS"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends "/dist/$PREFIX-full.deb" xvfb xauth curl >/dev/null
echo "Depends: $(dpkg-query -W -f '${Depends}' miko-kagura)"
check_libs /opt/miko-kagura
smoke 8282 miko-kagura
cp "/dist/$PREFIX-full.AppImage" /tmp/app.AppImage
# Containers have no FUSE; the AppImage unpacks itself instead of mounting.
smoke 8283 env APPIMAGE_EXTRACT_AND_RUN=1 /tmp/app.AppImage
EOF

echo '── archlinux: AUR PKGBUILD'
BUILD=$(mktemp -d)
trap 'rm -rf -- "${BUILD:?}"' EXIT
"$PROJECT_ROOT/scripts/update-aur.sh" "$TAG" "$DIST/SHA256SUMS.txt" "$BUILD"
# Pre-placed sources make makepkg verify these exact files instead of downloading.
cp "$DIST/$PREFIX-portable-full.tar.gz" "$BUILD/"
cp "$PROJECT_ROOT"/packaging/linux/{miko-kagura.sh,miko-kagura.desktop,miko-kagura.svg} "$BUILD/"
mkdir -p "$AUR_OUT"
"$DOCKER" run --rm -i -e CHECKS="$CHECKS" \
    -v "$BUILD:/src:ro" -v "$AUR_OUT:/out" archlinux:latest bash -s <<'EOF'
set -euo pipefail
eval "$CHECKS"
pacman -Syu --noconfirm --needed base-devel xorg-server-xvfb xorg-xauth curl >/dev/null
useradd -m builder
cp -r /src /home/builder/pkg && chown -R builder /home/builder/pkg
cd /home/builder/pkg
deps=$(su builder -c 'source PKGBUILD; printf "%s\n" "${depends[@]}"')
pacman -S --noconfirm --needed $deps >/dev/null
su builder -c 'makepkg --printsrcinfo > .SRCINFO && makepkg -f --noconfirm'
pacman -U --noconfirm ./miko-kagura-bin-*.pkg.tar.zst >/dev/null
check_libs /opt/miko-kagura
smoke 8282 miko-kagura
cp PKGBUILD .SRCINFO /out/
EOF
echo "Tested AUR files: $AUR_OUT"
