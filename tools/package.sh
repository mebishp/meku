#!/usr/bin/env bash
# Build release packages for Linux x86_64 into dist/.
#
#   tools/package.sh                 # build, then deb + AppImage + Arch + tar.gz
#   FORMATS="deb tar" tools/package.sh
#   SKIP_BUILD=1 tools/package.sh    # reuse target/release/meku
#
# Needs: cargo, ar, tar, xz, gzip, rsvg-convert. AppImage also needs curl (it
# downloads appimagetool once into dist/.tools); Arch needs makepkg.
set -euo pipefail

cd "$(dirname "$0")/.."
VER=$(grep -m1 '^version' crates/meku/Cargo.toml | cut -d'"' -f2)
FORMATS=${FORMATS:-"deb appimage arch tar"}
BIN=target/release/meku
OUT=dist
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$OUT"

if [ "${SKIP_BUILD:-}" != 1 ]; then
    cargo build --release -p meku
fi
[ -x "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

rsvg-convert -w 256 -h 256 packaging/meku.svg -o "$STAGE/meku.png"

# Shared install tree (deb and tar).
tree() {
    local root=$1
    install -Dm755 "$BIN" "$root/usr/bin/meku"
    install -Dm644 packaging/meku.desktop "$root/usr/share/applications/meku.desktop"
    install -Dm644 "$STAGE/meku.png" "$root/usr/share/icons/hicolor/256x256/apps/meku.png"
    install -Dm644 packaging/meku.svg "$root/usr/share/icons/hicolor/scalable/apps/meku.svg"
}

build_deb() {
    local d=$STAGE/deb
    mkdir -p "$d/root" "$d/ctl"
    tree "$d/root"
    cat > "$d/ctl/control" <<CONTROL
Package: meku
Version: $VER
Section: editors
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.35), libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1, libwayland-client0
Recommends: fonts-inter, fonts-jetbrains-mono
Installed-Size: $(du -sk "$d/root" | cut -f1)
Maintainer: mebishp <noreply@users.noreply.github.com>
Homepage: https://github.com/mebishp/meku
Description: GPU-accelerated native markdown editor
 Meku is a native markdown editor for folders of plain notes (a "mekuto"),
 with live preview, tabs, an outline and themes. Built with Rust and GPUI.
 Needs a Vulkan-capable GPU driver.
CONTROL
    (cd "$d/ctl" && tar --owner=0 --group=0 -czf ../control.tar.gz ./control)
    (cd "$d/root" && tar --owner=0 --group=0 -cJf ../data.tar.xz .)
    printf '2.0\n' > "$d/debian-binary"
    rm -f "$OUT/meku_${VER}_amd64.deb"
    (cd "$d" && ar rc "$OLDPWD/$OUT/meku_${VER}_amd64.deb" debian-binary control.tar.gz data.tar.xz)
}

build_appimage() {
    local tool=$OUT/.tools/appimagetool-x86_64.AppImage
    if [ ! -x "$tool" ]; then
        mkdir -p "$OUT/.tools"
        curl -fL -o "$tool" \
            https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
        chmod +x "$tool"
    fi
    local a=$STAGE/Meku.AppDir
    install -Dm755 "$BIN" "$a/usr/bin/meku"
    install -Dm644 packaging/meku.desktop "$a/meku.desktop"
    install -Dm644 "$STAGE/meku.png" "$a/meku.png"
    install -Dm644 "$STAGE/meku.png" "$a/.DirIcon"
    printf '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/meku" "$@"\n' > "$a/AppRun"
    chmod +x "$a/AppRun"
    ARCH=x86_64 "$tool" --appimage-extract-and-run "$a" "$OUT/Meku-$VER-x86_64.AppImage"
}

build_arch() {
    local d=$STAGE/arch
    mkdir -p "$d"
    cp packaging/PKGBUILD packaging/meku.desktop packaging/meku.svg "$d/"
    cp "$STAGE/meku.png" "$BIN" "$d/"
    mv "$d/meku" "$d/meku-bin"
    sed -i "s/^pkgver=.*/pkgver=$VER/" "$d/PKGBUILD"
    (cd "$d" && makepkg -f --nodeps)
    cp "$d"/meku-*.pkg.tar.zst "$OUT/"
}

build_tar() {
    local t=$STAGE/meku-$VER-x86_64
    mkdir -p "$t"
    cp "$BIN" packaging/meku.desktop packaging/meku.svg "$STAGE/meku.png" "$t/"
    tar -C "$STAGE" -czf "$OUT/meku-$VER-x86_64-linux.tar.gz" "meku-$VER-x86_64"
}

for f in $FORMATS; do
    echo "==> $f"
    "build_$f"
done

(cd "$OUT" && sha256sum meku_* Meku-* meku-* 2>/dev/null > SHA256SUMS || true)
echo "Packages in $OUT/:"
ls -1 "$OUT" | grep -v '^\.'
