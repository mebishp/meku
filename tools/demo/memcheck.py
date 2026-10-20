"""Memory check for Meku: drive repeated actions on an isolated display and watch RSS.

    python3 tools/demo/memcheck.py [scenario ...]

For each scenario the action runs in rounds; after each round the resident memory
(RSS, and PSS from smaps_rollup) is sampled. A leak shows as a steady climb after
the warm-up rounds, a healthy app flattens out. Never touches the real desktop
(see clips.py).
"""
import os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import clips

ROUNDS = int(os.environ.get("ROUNDS", "8"))
PER = int(os.environ.get("PER", "10"))


def mem(pid):
    out = {}
    for line in open(f"/proc/{pid}/status"):
        if line.startswith(("VmRSS", "VmHWM")):
            k, v = line.split(":")
            out[k] = int(v.split()[0]) // 1024
    try:
        for line in open(f"/proc/{pid}/smaps_rollup"):
            if line.startswith("Pss:"):
                out["Pss"] = int(line.split()[1]) // 1024
    except OSError:
        pass
    return out


def series(s, name, action):
    x = s.x
    s.click(1040, 420, 0.3)
    x.pause(1.5)
    base = mem(s.proc.pid)
    print(f"\n[{name}] start RSS {base['VmRSS']} MB", flush=True)
    rss = []
    for r in range(ROUNDS):
        for i in range(PER):
            action(s, r * PER + i)
        x.pause(0.8)
        m = mem(s.proc.pid)
        rss.append(m["VmRSS"])
        if s.proc.poll() is not None:
            print(f"  Meku exited with {s.proc.poll()}", flush=True)
            break
        print(f"  round {r + 1}: RSS {m['VmRSS']} MB  PSS {m.get('Pss', '?')} MB  peak {m['VmHWM']} MB", flush=True)
    tail = rss[len(rss) // 2:]
    slope = (tail[-1] - tail[0]) / max(1, len(tail) - 1)
    print(f"  second-half growth: {slope:+.1f} MB per round of {PER}", flush=True)
    return rss


def a_menu_text(s, i):
    s.right_click(595, 196, 0.05)
    s.x.pause(0.12)
    s.x.combo("esc")
    s.x.pause(0.05)


def a_menu_image(s, i):
    s.right_click(670, 400, 0.05)
    s.x.pause(0.12)
    s.x.combo("esc")
    s.x.pause(0.05)


def a_theme(s, i):
    s.x.combo("ctrl", "alt", "t")
    s.x.pause(0.15)


NOTES = ["Ideas", "Roadmap", "Reading list", "Meeting notes", "Long note", "Trip to Lisbon", "Weekend plans"]


def a_tabs(s, i):
    x = s.x
    x.combo("ctrl", "p")
    x.pause(0.15)
    x.type_text(NOTES[i % len(NOTES)].lower(), 3.0)
    x.pause(0.15)
    x.combo("enter")
    x.pause(0.35)
    if i % 2:
        x.combo("ctrl", "w")
        x.pause(0.1)


def a_lightbox(s, i):
    s.click(670, 400, 0.05)
    s.x.pause(0.3)
    s.x.combo("esc")
    s.x.pause(0.1)


def a_type(s, i):
    s.x.type_text("memory check line number %d\n" % i, 6.0)


def a_scroll(s, i):
    s.x.wheel(25, 0.004)
    s.x.combo("pgdn")


BIG = ["Big note %02d" % i for i in range(40)]


def a_tabs_big(s, i):
    """Open and close many large notes, one after another."""
    x = s.x
    x.combo("ctrl", "p")
    x.pause(0.12)
    x.type_text(BIG[i % len(BIG)].lower(), 4.0)
    x.pause(0.15)
    x.combo("enter")
    x.pause(0.4)
    x.combo("ctrl", "w")
    x.pause(0.15)


def make_big_notes():
    """Forty notes of 4,000 lines each, added to the demo vault copy."""
    import random
    rnd = random.Random(7)
    words = "alpha beta gamma delta note plan idea draft review budget travel".split()
    for n in BIG:
        lines = ["## %s\n" % n]
        for i in range(4000):
            lines.append(" ".join(rnd.choice(words) for _ in range(12)) + ".\n")
        open(os.path.join(clips.VAULT, n + ".md"), "w").write("\n".join(lines))


PICS = ["Pic note %02d" % i for i in range(30)]


def a_pics(s, i):
    """Open and close notes that each show a different large picture."""
    x = s.x
    x.combo("ctrl", "p")
    x.pause(0.12)
    x.type_text(PICS[i % len(PICS)].lower(), 4.0)
    x.pause(0.15)
    x.combo("enter")
    x.pause(0.7)
    x.combo("ctrl", "w")
    x.pause(0.15)


def make_pic_notes():
    """Thirty notes, each with its own 1600x900 picture."""
    from PIL import Image
    import random
    rnd = random.Random(3)
    for k, n in enumerate(PICS):
        im = Image.effect_noise((1600, 900), 60).convert("RGB")
        im = Image.blend(im, Image.new("RGB", im.size, tuple(rnd.randrange(256) for _ in range(3))), 0.5)
        im.save(os.path.join(clips.VAULT, "pic%02d.png" % k))
        open(os.path.join(clips.VAULT, n + ".md"), "w").write("Picture %d\n\n![[pic%02d.png]]\n" % (k, k))


SCENARIOS = {
    "pics": ("Trip to Lisbon.md", a_pics),
    "tabs-big": ("Trip to Lisbon.md", a_tabs_big),
    "menu-text": ("Trip to Lisbon.md", a_menu_text),
    "menu-image": ("Trip to Lisbon.md", a_menu_image),
    "themes": ("Trip to Lisbon.md", a_theme),
    "tabs": ("Trip to Lisbon.md", a_tabs),
    "lightbox": ("Trip to Lisbon.md", a_lightbox),
    "typing": ("Long note.md", a_type),
    "scroll": ("Long note.md", a_scroll),
}

if __name__ == "__main__":
    names = sys.argv[1:] or list(SCENARIOS)
    disp = clips.start_display()
    try:
        # idle baseline
        s = clips.Session("Trip to Lisbon.md")
        time.sleep(3)
        print("idle after start:", mem(s.proc.pid), flush=True)
        time.sleep(20)
        print("idle after 20 s: ", mem(s.proc.pid), flush=True)
        s.close()
        for n in names:
            note, fn = SCENARIOS[n]
            s = clips.Session(note)
            try:
                if n in ("tabs-big", "pics"):
                    (make_big_notes if n == "tabs-big" else make_pic_notes)()
                    s.proc.terminate()
                    s.proc.wait()
                    s = clips.Session(note, keep_vault=True)
                series(s, n, fn)
            finally:
                s.close()
    finally:
        disp.terminate()
