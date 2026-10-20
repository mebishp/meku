"""Record short feature clips of Meku for the README, without touching the desktop.

Meku runs on its own throwaway X display (Xwayland), is driven there with XTEST
(`xdrive.py`) and recorded with ffmpeg's x11grab. Your keyboard and mouse are
never used. `make_gifs.sh` turns the clips into small GIFs.

    python3 clips.py still live      # save a frame of each step, for placing clicks
    python3 clips.py live themes menu navigate speed

Needs: Xwayland, python-xlib, ffmpeg, a release build of Meku.
"""
import json, os, shutil, signal, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

DISP = os.environ.get("CLIPS_DISPLAY", ":99")
os.environ["DISPLAY"] = DISP
os.environ.pop("WAYLAND_DISPLAY", None)

ROOT = os.environ.get("CLIPS_DIR", "/tmp/claude-1000/clips")
VAULT = os.environ.get("CLIPS_VAULT", "/tmp/claude-1000/clips-vault")
BIN = os.environ.get("MEKU_BIN", os.path.abspath("target/release/meku"))
CFG = os.path.join(ROOT, "cfg")
SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vault")
FPS = int(os.environ.get("CLIPS_FPS", "50"))
SCREEN = (1280, 840)
os.makedirs(ROOT, exist_ok=True)


def start_display():
    proc = subprocess.Popen(["Xwayland", DISP, "-geometry", "%dx%d" % SCREEN, "-noreset"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(50):
        if os.path.exists("/tmp/.X11-unix/X" + DISP.lstrip(":")):
            break
        time.sleep(0.1)
    time.sleep(1.0)
    return proc


def long_note(path):
    """The 10,000-line note for the speed clip (not kept in the repository)."""
    import random
    rnd = random.Random(1)
    words = "the quick note about plans and ideas for a long project that keeps growing every single week".split()
    out = []
    for i in range(1, 10001):
        if i % 60 == 0:
            out.append("\n## Section %d\n" % (i // 60))
        out.append(" ".join(rnd.choice(words) for _ in range(rnd.randint(8, 16))).capitalize() + ".\n")
    open(path, "w").write("\n".join(out))


def settings(theme, extra=None, keep_vault=False):
    d = os.path.join(CFG, "meku")
    shutil.rmtree(CFG, ignore_errors=True)
    os.makedirs(d)
    # every clip starts from the same pristine notes
    if not keep_vault:
        shutil.rmtree(VAULT, ignore_errors=True)
        shutil.copytree(SRC, VAULT)
        long_note(os.path.join(VAULT, "Long note.md"))
    cfg = {"theme": theme, "follow_system": False, "check_updates": False,
           "reopen_last_mekuto": False}
    cfg.update(extra or {})
    json.dump(cfg, open(os.path.join(d, "settings.json"), "w"))


def window_box(xd):
    """Bounds of Meku's window on the display: (x, y, w, h)."""
    best = None
    for w in xd.root.query_tree().children:
        g = w.get_geometry()
        a = w.get_attributes()
        if a.map_state == 2 and g.width > 400:
            best = (g.x, g.y, g.width, g.height)
    return best


class Session:
    def __init__(self, note, theme="Meku Dark", extra=None, keep_vault=False):
        settings(theme, extra, keep_vault)
        env = dict(os.environ, XDG_CONFIG_HOME=CFG)
        self.proc = subprocess.Popen([BIN, VAULT, note], env=env,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(3.0)
        import xdrive
        self.x = xdrive
        self.box = window_box(xdrive)
        self.wx, self.wy, self.ww, self.wh = self.box
        self.rec = None
        self.n = 0

    # window-relative pointer helpers
    def move(self, x, y, dur=0.7):
        self.x.glide(self.wx + x, self.wy + y, dur)

    def jump(self, x, y):
        self.x.moveto(self.wx + x, self.wy + y)

    def click(self, x=None, y=None, dur=0.6, times=1):
        if x is not None:
            self.move(x, y, dur)
            self.x.pause(0.12)
        self.x.click(1, times)

    def right_click(self, x, y, dur=0.6):
        self.move(x, y, dur)
        self.x.pause(0.12)
        self.x.right_click()

    # recording
    def start(self, name, crop=None):
        x, y, w, h = crop or (0, 0, self.ww, self.wh)
        w -= w % 2
        h -= h % 2
        path = os.path.join(ROOT, name + ".mp4")
        self.rec = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-framerate", str(FPS),
             "-draw_mouse", "1", "-video_size", "%dx%d" % (w, h),
             "-i", "%s+%d,%d" % (DISP, self.wx + x, self.wy + y),
             "-c:v", "libx264", "-preset", "veryfast", "-crf", "12", "-pix_fmt", "yuv420p", path],
            stdin=subprocess.PIPE)
        time.sleep(0.5)
        return path

    def stop(self):
        time.sleep(0.4)
        self.rec.stdin.write(b"q")
        self.rec.stdin.flush()
        self.rec.wait(20)

    def still(self, label):
        path = os.path.join(ROOT, "still-%02d-%s.png" % (self.n, label))
        self.n += 1
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-video_size",
                        "%dx%d" % (self.ww, self.wh), "-i",
                        "%s+%d,%d" % (DISP, self.wx, self.wy), "-frames:v", "1", path])
        print("still", path, flush=True)

    def close(self):
        self.proc.terminate()
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def run(name, fn, note, theme="Meku Dark", extra=None, still=False):
    s = Session(note, theme, extra)
    print(name, "window", s.box, flush=True)
    try:
        fn(s, still)
    finally:
        s.close()


# ---------------------------------------------------------------- clips

def live(s, still):
    x = s.x
    s.click(700, 400)
    x.pause(0.3)
    if not still:
        s.start("live")
    x.pause(0.8)
    x.type_text("## This weekend\n\n", 1.7)
    x.pause(0.3)
    x.type_text("Write in **markdown** and it turns into *live* formatting.\n\n", 1.7)
    x.pause(0.3)
    x.type_text("- [ ] Pack the camera\nBook the train\nBuy snacks\n\n\n", 1.7)
    x.pause(0.5)
    if still:
        s.still("tasks")
        return
    s.click(CHECK[0], CHECK[1])
    x.pause(0.9)
    s.move(700, 560, 0.5)
    x.type_text("> [!tip] Callouts\nLike this one.", 1.7)
    x.pause(1.8)
    s.stop()


CHECK = (378, 289)


def themes(s, still):
    x = s.x
    s.click(1040, 400, 0.3)     # focus the window
    if still:
        s.still("trip")
        return
    s.start("themes")
    x.pause(0.9)
    for hold in (1.3, 1.1, 1.1, 1.1, 1.1, 1.3):
        x.combo("ctrl", "alt", "t")
        x.pause(hold)
    s.stop()


def menu(s, still):
    x = s.x
    s.move(1040, 420, 0.3)
    x.pause(0.4)
    if not still:
        s.start("menu")
    x.pause(0.8)
    if still and os.environ.get("IMG"):
        s.right_click(IMAGE[0], IMAGE[1], 0.3)
        x.pause(0.9)
        s.still("img-menu")
        return
    s.click(WORD[0], WORD[1], 0.8, times=2)
    x.pause(0.5)
    s.right_click(WORD[0], WORD[1], 0.2)
    x.pause(0.9)
    if still:
        s.still("menu-open")
        return
    s.move(*MENU_ITEM, dur=0.7)
    x.pause(0.5)
    s.click()
    x.pause(0.6)
    s.click(1040, 420, 0.7)     # caret elsewhere, so the bold word shows rendered
    x.pause(1.2)
    s.right_click(IMAGE[0], IMAGE[1], 0.9)
    x.pause(1.0)
    s.move(*IMG_SIZE, dur=0.6)
    x.pause(0.8)
    s.move(*IMG_SMALL, dur=0.4)
    x.pause(0.3)
    s.click()
    x.pause(1.6)
    s.stop()


WORD = (595, 196)
MENU_ITEM = (640, 305)
IMAGE = (670, 400)
IMG_SIZE = (710, 537)
IMG_SMALL = (566, 538)

def navigate(s, still):
    x = s.x
    s.click(1040, 420, 0.3)     # focus the window
    x.pause(0.4)
    if not still:
        s.start("navigate")
    x.pause(0.8)
    x.combo("ctrl", "p")
    x.pause(0.5)
    x.type_text("road", 1.2)
    x.pause(0.9)
    if still:
        s.still("quick-open")
    x.combo("enter")
    x.pause(1.0)
    x.combo("ctrl", "shift", "f")
    x.pause(0.5)
    x.type_text("photos", 1.2)
    x.pause(1.2)
    if still:
        s.still("search")
        x.combo("esc")
        x.combo("ctrl", "shift", "p")
        x.pause(0.4)
        x.type_text("theme", 1.2)
        x.pause(0.8)
        s.still("palette")
        return
    x.combo("esc")
    x.pause(0.9)
    s.stop()


def speed(s, still):
    x = s.x
    s.click(1040, 420, 0.3)     # focus the window
    x.pause(0.4)
    x.combo("ctrl", "p")
    x.pause(0.4)
    x.type_text("long", 1.5)
    x.pause(0.5)
    x.combo("enter")
    x.pause(1.2)
    if still:
        s.still("long")
        return
    s.start("speed")
    x.pause(0.8)
    # drag the scroll bar down through ten thousand lines and back
    s.move(1091, 110, 0.7)
    x.pause(0.2)
    x.down()
    x.pause(0.1)
    for ty in (420, 690, 300, 90):
        s.move(1091, ty, 0.8)
    x.up()
    x.pause(0.5)
    s.click(700, 300, 0.7)
    x.pause(0.3)
    x.type_text("Typing stays instant, even here.", 2.2)
    x.pause(1.4)
    s.stop()


SCENES = {"live": ("Weekend plans.md", live), "themes": ("Trip to Lisbon.md", themes),
          "menu": ("Trip to Lisbon.md", menu),
          "navigate": ("Trip to Lisbon.md", navigate),
          "speed": ("Trip to Lisbon.md", speed)}

if __name__ == "__main__":
    args = sys.argv[1:]
    still = bool(args) and args[0] == "still"
    names = args[1:] if still else args
    disp = start_display()
    try:
        for n in names:
            note, fn = SCENES[n]
            run(n, fn, note, still=still)
    finally:
        disp.terminate()
