#!/usr/bin/env bash
# Writes the AUR PKGBUILD for a release into <out-dir> with pkgver, _tag and
# sha256sums filled in. Run from a checkout of <tag>: the launcher, desktop entry
# and icon are hashed from the working tree. Generate .SRCINFO next to it with
# `makepkg --printsrcinfo > .SRCINFO` (test-linux-packages.sh does this in CI).
# Usage: scripts/update-aur.sh <tag> [SHA256SUMS.txt] [out-dir]
set -euo pipefail
. "$(dirname "$0")/release-common.sh"

TAG=${1:?usage: update-aur.sh <tag> [SHA256SUMS.txt] [out-dir]}
# The tag was validated against Cargo.toml when the packages were built; this
# runs on the AUR maintainer's machine too, which needs no Rust toolchain.
[[ $TAG =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$ ]] || die "Expected a release tag, got '$TAG'."
VERSION=${TAG#v}
SUMS=${2:-$PROJECT_ROOT/dist/$TAG-linux/SHA256SUMS.txt}
OUT=${3:-$PROJECT_ROOT/target/aur/miko-kagura-bin}
BUNDLE="Miko-Kagura-$TAG-linux-x64-portable-full.tar.gz"

bundle_sha=$(awk -v name="$BUNDLE" '$2 == name || $2 == "*" name { print $1 }' "$SUMS")
[[ $bundle_sha =~ ^[a-f0-9]{64}$ ]] || die "No checksum for $BUNDLE in $SUMS"
file_sha() { sha256sum "$PROJECT_ROOT/packaging/linux/$1" | cut -d' ' -f1; }
# pacman versions cannot contain '-'. Dropping it keeps prereleases first:
# vercmp orders 1.2.0beta.1 < 1.2.0 (but 1.2.0_beta.1 > 1.2.0). Build metadata is dropped.
pkgver=${VERSION%%+*}
pkgver=${pkgver//-/}

mkdir -p "$OUT"
awk -v pkgver="$pkgver" -v tag="$TAG" \
    -v sums="('$bundle_sha'\n            '$(file_sha miko-kagura.sh)'\n            '$(file_sha miko-kagura.desktop)'\n            '$(file_sha miko-kagura.svg)')" '
    /^pkgver=/ { print "pkgver=" pkgver; next }
    /^_tag=/ { print "_tag=" tag; next }
    /^pkgrel=/ { print "pkgrel=1"; next }
    /^sha256sums=/ { print "sha256sums=" sums; skipping = !/\)$/; next }
    skipping { skipping = !/\)$/; next }
    { print }
' "$PROJECT_ROOT/packaging/aur/miko-kagura-bin/PKGBUILD" >"$OUT/PKGBUILD"
grep -q "'$bundle_sha'" "$OUT/PKGBUILD" || die 'Failed to write sha256sums.'
echo "Wrote $OUT/PKGBUILD ($pkgver)"
