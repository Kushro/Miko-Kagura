#!/usr/bin/env bash
# Linux counterpart of build-release.ps1. Builds into dist/<tag>-linux:
#   portable-lite.tar.gz, portable-full.tar.gz, full.AppImage, full.deb, SHA256SUMS.txt
# Requires dx 0.7.9, bun, cargo, jq, curl, unzip, dpkg-dev and the WebKitGTK 4.1
# development packages. Usage: scripts/build-release-linux.sh <tag> [--skip-tests]
set -euo pipefail
. "$(dirname "$0")/release-common.sh"

TAG=${1:?usage: build-release-linux.sh <tag> [--skip-tests]}
VERSION=$(release_version "$TAG")
[[ $(dx --version) == "dioxus 0.7.9 "* ]] || die 'Install Dioxus CLI 0.7.9.'
cd "$PROJECT_ROOT"

WORK="$PROJECT_ROOT/target/release-packaging-linux"
DIST="$PROJECT_ROOT/dist/$TAG-linux"
PREFIX="Miko-Kagura-$TAG-linux-x64"
# Only generated directories owned by this script are reset.
rm -rf -- "${WORK:?}" "${DIST:?}"
mkdir -p "$WORK" "$DIST"

bun install --frozen-lockfile
bun run tailwind:build
[ "${2:-}" = --skip-tests ] || cargo test --locked
dx build --platform desktop --release --locked
scripts/fetch-upscalers.sh "$WORK/binaries"

BUILT="$PROJECT_ROOT/target/dx/miko-kagura/release/linux/app"
[ -x "$BUILT/miko-kagura" ] || die "dx build produced no executable at $BUILT/miko-kagura"
# Arch's xdotool 4.x renamed libxdo.so.3 to .so.4, so ship the .so.3 we link
# against beside the binary; its own dependencies (libX11, libXtst, libXinerama,
# libxkbcommon) are stable across distributions.
APP="$WORK/app"
cp -a "$BUILT" "$APP"
xdo=$(ldd "$APP/miko-kagura" | awk '$1 == "libxdo.so.3" { print $3 }')
[ -f "$xdo" ] || die 'libxdo.so.3 is not linked or not installed (libxdo-dev).'
install -Dm644 "$xdo" "$APP/lib/libxdo.so.3"
install -Dm644 /usr/share/doc/libxdo3/copyright "$APP/lib/libxdo3-copyright.txt"
patchelf --set-rpath '$ORIGIN/lib' "$APP/miko-kagura"

for edition in lite full; do
    payload="$WORK/$edition/Miko-Kagura"
    mkdir -p "$payload/packaging"
    cp -a "$APP/." "$payload/"
    cp -a LICENSE LICENSES README.md README.es.md THIRD_PARTY_NOTICES.md docs "$payload/"
    cp packaging/upscalers-linux-x64.lock.json "$payload/packaging/"
    [ "$edition" = lite ] || cp -a "$WORK/binaries" "$payload/binaries"
    assert_payload "$payload" "$edition"
    tar --sort=name --owner=0 --group=0 --numeric-owner -C "$WORK/$edition" \
        -czf "$DIST/$PREFIX-portable-$edition.tar.gz" Miko-Kagura
done
FULL="$WORK/full/Miko-Kagura"

# Both system packages carry Full so upscaling works without extra setup. The app
# stays in one folder under /opt so assets/ and binaries/ resolve beside the executable.
install_desktop_files() {
    local root=$1
    install -Dm644 packaging/linux/miko-kagura.desktop "$root/usr/share/applications/miko-kagura.desktop"
    install -Dm644 packaging/linux/miko-kagura.svg "$root/usr/share/icons/hicolor/scalable/apps/miko-kagura.svg"
}

# ── .deb ──────────────────────────────────────────────────────────────
DEB="$WORK/deb"
mkdir -p "$DEB/DEBIAN" "$DEB/opt"
cp -a "$FULL" "$DEB/opt/miko-kagura"
install -Dm755 packaging/linux/miko-kagura.sh "$DEB/usr/bin/miko-kagura"
install_desktop_files "$DEB"
install -Dm644 LICENSE "$DEB/usr/share/doc/miko-kagura/copyright"
# dpkg-shlibdeps reads the libraries the app and engines link against; it needs a
# debian/control to run, so give it a throwaway one.
SHLIBS="$WORK/shlibs"
mkdir -p "$SHLIBS/debian"
printf 'Source: miko-kagura\n\nPackage: miko-kagura\nArchitecture: amd64\n' >"$SHLIBS/debian/control"
mapfile -t elfs < <(find "$DEB/opt/miko-kagura" -type f -perm -u+x -exec sh -c 'head -c4 "$1" | grep -q ELF' _ {} \; -print)
depends=$(cd "$SHLIBS" && dpkg-shlibdeps -O "${elfs[@]/#/-e}" | sed -n 's/^shlibs:Depends=//p')
[ -n "$depends" ] || die 'dpkg-shlibdeps found no dependencies.'
cat >"$DEB/DEBIAN/control" <<EOF
Package: miko-kagura
Version: $(deb_version "$VERSION")
Architecture: amd64
Maintainer: Octavio <Kushro@users.noreply.github.com>
Installed-Size: $(du -sk --exclude=DEBIAN "$DEB" | cut -f1)
Depends: $depends, libvulkan1
Recommends: xdg-utils, mesa-vulkan-drivers | vulkan-icd
Section: graphics
Priority: optional
Homepage: https://github.com/Kushro/Miko-Kagura
Description: Remote image upscaling companion for Miko
 Desktop dashboard and local HTTP server that upscales images for the Miko
 reader with the bundled Waifu2x, Real-CUGAN and Real-ESRGAN ncnn-vulkan engines.
EOF
dpkg-deb --root-owner-group -Zxz --build "$DEB" "$DIST/$PREFIX-full.deb"

# ── AppImage ─────────────────────────────────────────────────────────
# appimagetool and its runtime are pinned; unpinned, appimagetool downloads
# the latest runtime at build time.
TOOLS="$WORK/tools"
mkdir -p "$TOOLS"
appimagetool=$(verified_download \
    https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
    ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 "$PROJECT_ROOT/target/download-cache")
runtime=$(verified_download \
    https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
    2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d "$PROJECT_ROOT/target/download-cache")
install -m755 "$appimagetool" "$TOOLS/appimagetool"
APPDIR="$WORK/Miko-Kagura.AppDir"
mkdir -p "$APPDIR/opt"
cp -a "$FULL" "$APPDIR/opt/miko-kagura"
install -m755 packaging/linux/AppRun "$APPDIR/AppRun"
install -m644 packaging/linux/miko-kagura.desktop packaging/linux/miko-kagura.svg "$APPDIR/"
install_desktop_files "$APPDIR"
# CI runners have no FUSE; the tool unpacks itself instead of mounting.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOLS/appimagetool" --no-appstream \
    --runtime-file "$runtime" "$APPDIR" "$DIST/$PREFIX-full.AppImage"
chmod 755 "$DIST/$PREFIX-full.AppImage"

(cd "$DIST" && sha256sum -- * >SHA256SUMS.txt)
[ "$(wc -l <"$DIST/SHA256SUMS.txt")" = 4 ] || die 'Expected four Linux release packages.'
glibc=$(objdump -T "$APP/miko-kagura" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)
echo "Release packages: $DIST (requires $glibc)"
