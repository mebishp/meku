# README clips

How the GIFs in `docs/images/` are made: `meku-live`, `-menu`, `-themes`,
`-navigate` and `-speed`.

Meku runs on its own throwaway X display (Xwayland) and is driven there with
XTEST key and pointer events, so **your real keyboard and mouse are never used**
and the clips do not depend on your desktop, window manager or theme.

Needs `Xwayland`, `python-xlib`, `ffmpeg`, [gifski](https://gif.ski) and a release
build of Meku (`cargo build --release`). Run from the repository root.

1. Put the demo notes in `/tmp/claude-1000/clips-vault.orig` (set `CLIPS_VAULT`
   and `CLIPS_DIR` to use other places). Every clip starts from a fresh copy.
2. `python3 tools/demo/clips.py live menu themes navigate speed` records
   `$CLIPS_DIR/<name>.mp4`. `python3 tools/demo/clips.py still menu` saves a frame
   at each step instead, which is how click positions are found.
3. `tools/demo/make_gifs.sh` turns the clips into GIFs. `WIDTH`, `FPS` and
   `QUALITY` trade sharpness for size: the README clips are 900 px wide, 25 to
   30 fps, quality 85 (the scrolling one is 760 px, 15 fps, quality 60) and stay
   under 1.5 MB each.

`drive.py`, `record.py` and `remotion/` are the older, edited demo (real input
through `/dev/uinput`, GNOME screencast, Remotion camera moves). That one takes
over the pointer and keyboard of the desktop it runs on, so use it only on a
machine you are not working on. It produced `docs/images/meku-demo.mp4`.
