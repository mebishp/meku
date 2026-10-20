# Demo video

How `docs/images/meku-demo.gif` was made. It is a real screen recording of Meku
(real key presses and mouse), edited with [Remotion](https://www.remotion.dev).

Needs GNOME on Wayland, write access to `/dev/uinput`, Python with `evdev` and
`gi`, ffmpeg, Node and a release build of Meku. The paths inside the scripts
are the ones used on the author's machine; adjust them to yours.

1. Open Meku on a demo folder, with an empty note:
   `target/release/meku /path/to/vault "Notes/Quick start.md"`
2. `python3 record.py` presses the keys, moves the pointer, switches themes and
   records the window with GNOME's screencast service. It writes `raw.mp4` and
   `events.json` (pointer path and click times).
3. In `remotion/`: copy `raw.mp4` to `public/`, `events.json` to `src/`, run
   `npm install`, then
   `npx remotion render src/index.ts MekuDemo meku-demo.mp4 --codec h264 --crf 17`.
4. Make the GIF with [gifski](https://gif.ski) (clean colours, no smearing):
   `ffmpeg -i meku-demo.mp4 -vf "fps=20,scale=960:-1:flags=lanczos" frames/f%04d.png`
   then `gifski --fps 20 --quality 100 --width 960 -o meku-demo.gif frames/f*.png`

`record.py` records the maximized window (1920x1048) at 60 fps with a high quality
x264 pipeline, so the video stays sharp when the edit zooms in. The final render
is 1920x1080 at 60 fps.
