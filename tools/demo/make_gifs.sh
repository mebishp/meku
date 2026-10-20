#!/usr/bin/env bash
# Turn the recorded clips (mp4) into small GIFs for the README with ffmpeg + gifski.
#   tools/demo/make_gifs.sh [clip ...]      default: every clip in $CLIPS_DIR
# WIDTH, FPS and QUALITY trade sharpness for file size.
set -euo pipefail
CLIPS=${CLIPS_DIR:-/tmp/claude-1000/clips}
OUT=${OUT_DIR:-docs/images}
WIDTH=${WIDTH:-900}
FPS=${FPS:-25}
QUALITY=${QUALITY:-85}
GIFSKI=${GIFSKI:-$(command -v gifski || echo "$HOME/.cargo/bin/gifski")}
names=("$@")
[ ${#names[@]} -gt 0 ] || names=(live themes menu navigate speed)
mkdir -p "$OUT"
for n in "${names[@]}"; do
    tmp=$(mktemp -d)
    ffmpeg -y -loglevel error -i "$CLIPS/$n.mp4" -vf "fps=$FPS,scale=$WIDTH:-1:flags=lanczos" "$tmp/f%04d.png"
    "$GIFSKI" --quiet --fps "$FPS" --quality "$QUALITY" --width "$WIDTH" -o "$OUT/meku-$n.gif" "$tmp"/f*.png
    rm -rf "$tmp"
    printf '%s  %s\n' "$(du -h "$OUT/meku-$n.gif" | cut -f1)" "$OUT/meku-$n.gif"
done
