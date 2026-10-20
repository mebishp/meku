#!/usr/bin/env bash
# Build the Linux packages (x86_64) into dist/: .deb, AppImage, Arch .pkg.tar.zst, .tar.gz.
#
#   tools/package.sh                  # build, then all four
#   FORMATS="deb tar" tools/package.sh
#   SKIP_BUILD=1 tools/package.sh     # reuse target/release/meku
#
# Build on an OLD system (the Release workflow uses ubuntu-22.04): a binary only runs
# on systems whose glibc is at least as new as the one it was built on. This script
# refuses a binary that needs a newer glibc than MAX_GLIBC. For a quick local try on a
# newer machine: ALLOW_NEW_GLIBC=1 (the result will not run on older distros).
#
# Needs: cargo, tar, xz, gzip, zstd, ar, rsvg-convert, objdump, ldd. AppImage also needs
# curl (it downloads appimagetool once into dist/.tools).
set -euo pipefail

cd "$(dirname "$0")/.."
VER=$(grep -m1 '^version' crates/meku/Cargo.toml | cut -d'"' -f2)
FORMATS=${FORMATS:-"deb appimage arch tar"}
BIN=${BIN:-target/release/meku}
MAX_GLIBC=${MAX_GLIBC:-2.35}
OUT=dist
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$OUT"

if [ "${SKIP_BUILD:-}" != 1 ]; then
    cargo build --release -p meku
fi
[ -x "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }

# --- the binary must not need a newer glibc than we promise in the packages ---
need=$(objdump -T "$BIN" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1)
if [ -n "$need" ] && [ "$(printf '%s\n%s\n' "$need" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]; then
    if [ "${ALLOW_NEW_GLIBC:-}" = 1 ]; then
        echo "WARNING: $BIN needs glibc $need (> $MAX_GLIBC); it will not run on older distros" >&2
    else
        echo "ERROR: $BIN needs glibc $need, newer than $MAX_GLIBC. Build on ubuntu-22.04 (or set ALLOW_NEW_GLIBC=1)." >&2
        exit 1
    fi
fi

rsvg-convert -w 256 -h 256 packaging/meku.svg -o "$STAGE/meku.png"

# What Meku needs from the system. The GPU driver (Vulkan) always comes from the host.
DESC="GPU-accelerated native markdown editor"
LONGDESC="Meku is a native markdown editor for folders of plain notes (a \"mekuto\"), with live preview, tabs, an outline and themes. Needs a Vulkan-capable GPU driver."

# Shared install tree (deb).
tree() {
    local root=$1
    install -Dm755 "$BIN" "$root/usr/bin/meku"
    install -Dm644 packaging/meku.desktop "$root/usr/share/applications/meku.desktop"
    install -Dm644 "$STAGE/meku.png" "$root/usr/share/icons/hicolor/256x256/apps/meku.png"
    install -Dm644 packaging/meku.svg "$root/usr/share/icons/hicolor/scalable/apps/meku.svg"
    install -Dm644 LICENSE "$root/usr/share/doc/meku/copyright"
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
Depends: libc6 (>= $MAX_GLIBC), libgcc-s1, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1
Recommends: mesa-vulkan-drivers | nvidia-vulkan-icd, libwayland-client0, libwayland-egl1, libdbus-1-3, xclip, fonts-inter, fonts-jetbrains-mono
Installed-Size: $(du -sk "$d/root" | cut -f1)
Maintainer: mebishp <noreply@users.noreply.github.com>
Homepage: https://github.com/mebishp/meku
Description: $DESC
 $LONGDESC
CONTROL
    (cd "$d/ctl" && tar --owner=0 --group=0 -czf ../control.tar.gz ./control)
    (cd "$d/root" && tar --owner=0 --group=0 -cJf ../data.tar.xz .)
    printf '2.0\n' > "$d/debian-binary"
    rm -f "$OUT/meku_${VER}_amd64.deb"
    (cd "$d" && ar rc "$OLDPWD/$OUT/meku_${VER}_amd64.deb" debian-binary control.tar.gz data.tar.xz)
}

# Copy a shared library (resolved through ldd) into $2, following the soname.
bundle_lib() {
    local lib=$1 dest=$2 path
    path=$(ldd "$BIN" | awk -v l="$lib" '$1 == l {print $3}')
    [ -n "$path" ] && [ -e "$path" ] && cp -L "$path" "$dest/$lib"
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
    # Bundle the small libraries Meku links (they are stable and not tied to the GPU).
    # The Vulkan loader and driver, Wayland, EGL and D-Bus come from the host system.
    mkdir -p "$a/usr/lib"
    for lib in libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb.so.1 libxcb-xkb.so.1 libXau.so.6 libXdmcp.so.6; do
        bundle_lib "$lib" "$a/usr/lib" || echo "note: $lib not bundled (not linked here)" >&2
    done
    cat > "$a/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$HERE/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$HERE/usr/bin/meku" "$@"
APPRUN
    chmod +x "$a/AppRun"
    ARCH=x86_64 "$tool" --appimage-extract-and-run "$a" "$OUT/Meku-$VER-x86_64.AppImage"
}

# An Arch package is a tar.zst with a .PKGINFO first; no makepkg needed.
build_arch() {
    local d=$STAGE/arch
    mkdir -p "$d"
    tree "$d"
    local size
    size=$(du -sb "$d" | cut -f1)
    cat > "$d/.PKGINFO" <<PKGINFO
pkgname = meku
pkgbase = meku
pkgver = $VER-1
pkgdesc = $DESC for folders of notes
url = https://github.com/mebishp/meku
builddate = $(date +%s)
packager = mebishp
size = $size
arch = x86_64
license = Apache-2.0
depend = glibc
depend = gcc-libs
depend = libxcb
depend = libxkbcommon
depend = libxkbcommon-x11
depend = vulkan-icd-loader
depend = wayland
depend = dbus
depend = fontconfig
optdepend = xclip: copy pictures out of Meku on Wayland
optdepend = vulkan-driver: a Vulkan driver for your GPU (vulkan-radeon, vulkan-intel, nvidia-utils)
optdepend = inter-font: preferred interface font
optdepend = ttf-jetbrains-mono: preferred code font
PKGINFO
    rm -f "$OUT/meku-$VER-1-x86_64.pkg.tar.zst"
    (cd "$d" && tar --owner=0 --group=0 --numeric-owner -I zstd -cf "$OLDPWD/$OUT/meku-$VER-1-x86_64.pkg.tar.zst" .PKGINFO usr)
}

build_tar() {
    local t=$STAGE/meku-$VER-x86_64
    mkdir -p "$t"
    cp "$BIN" packaging/meku.desktop packaging/meku.svg "$STAGE/meku.png" LICENSE "$t/"
    cat > "$t/INSTALL.txt" <<'INSTALL'
Meku: unpack anywhere and run ./meku  (or ./meku /path/to/your/notes)

It needs these system libraries and a Vulkan-capable GPU driver.

Debian / Ubuntu (22.04 or newer):
  sudo apt install libxcb1 libxkbcommon0 libxkbcommon-x11-0 libvulkan1 \
                   mesa-vulkan-drivers libwayland-client0 libwayland-egl1 libdbus-1-3

Arch Linux:
  sudo pacman -S libxcb libxkbcommon libxkbcommon-x11 vulkan-icd-loader wayland dbus fontconfig
  (plus a Vulkan driver: vulkan-radeon, vulkan-intel or nvidia-utils)

Optional: xclip (copy pictures out of Meku on Wayland), the Inter and JetBrains Mono fonts.
To add Meku to your app menu, copy meku.desktop to ~/.local/share/applications/ and the
icon (meku.png) to ~/.local/share/icons/, and put the full path to meku in its Exec= line.
INSTALL
    tar -C "$STAGE" -czf "$OUT/meku-$VER-x86_64-linux.tar.gz" "meku-$VER-x86_64"
}

for f in $FORMATS; do
    echo "==> $f"
    "build_$f"
done

(cd "$OUT" && sha256sum meku_* Meku-* meku-* 2>/dev/null > SHA256SUMS || true)
echo "Packages in $OUT/:"
ls -1 "$OUT" | grep -v '^\.'
