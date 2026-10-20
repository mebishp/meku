import random, time
from evdev import UInput, ecodes as e

K = e
LOW = {c: getattr(K, f"KEY_{c.upper()}") for c in "abcdefghijklmnopqrstuvwxyz"}
DIG = {d: getattr(K, f"KEY_{d}") for d in "0123456789"}
PUNCT = {
    ' ': (K.KEY_SPACE, False), '\n': (K.KEY_ENTER, False),
    '-': (K.KEY_MINUS, False), '_': (K.KEY_MINUS, True),
    '=': (K.KEY_EQUAL, False), '+': (K.KEY_EQUAL, True),
    '[': (K.KEY_LEFTBRACE, False), '{': (K.KEY_LEFTBRACE, True),
    ']': (K.KEY_RIGHTBRACE, False), '}': (K.KEY_RIGHTBRACE, True),
    ';': (K.KEY_SEMICOLON, False), ':': (K.KEY_SEMICOLON, True),
    "'": (K.KEY_APOSTROPHE, False), '"': (K.KEY_APOSTROPHE, True),
    ',': (K.KEY_COMMA, False), '<': (K.KEY_COMMA, True),
    '.': (K.KEY_DOT, False), '>': (K.KEY_DOT, True),
    '/': (K.KEY_SLASH, False), '?': (K.KEY_SLASH, True),
    '`': (K.KEY_GRAVE, False), '~': (K.KEY_GRAVE, True),
    '\\': (K.KEY_BACKSLASH, False), '|': (K.KEY_BACKSLASH, True),
    '!': (K.KEY_1, True), '@': (K.KEY_2, True), '#': (K.KEY_3, True),
    '$': (K.KEY_4, True), '%': (K.KEY_5, True), '^': (K.KEY_6, True),
    '&': (K.KEY_7, True), '*': (K.KEY_8, True), '(': (K.KEY_9, True), ')': (K.KEY_0, True),
}
CAPS = [K.KEY_A + 0]  # placeholder

ui = UInput({e.EV_KEY: list(range(1, 249))}, name="meku-demo-kbd")
time.sleep(1.0)  # let the compositor pick the device up

def _tap(code, shift=False, mods=()):
    held = list(mods) + ([K.KEY_LEFTSHIFT] if shift else [])
    for m in held:
        ui.write(e.EV_KEY, m, 1)
    ui.write(e.EV_KEY, code, 1); ui.syn()
    time.sleep(0.012)
    ui.write(e.EV_KEY, code, 0)
    for m in reversed(held):
        ui.write(e.EV_KEY, m, 0)
    ui.syn()

def char(c):
    if c in LOW: return _tap(LOW[c])
    if c.isalpha() and c.lower() in LOW: return _tap(LOW[c.lower()], shift=True)
    if c in DIG: return _tap(DIG[c])
    code, shift = PUNCT[c]
    _tap(code, shift)

def type_text(s, speed=1.0, jitter=0.35):
    for c in s:
        char(c)
        base = 0.075 / speed
        d = base * random.uniform(1 - jitter, 1 + jitter)
        if c in ' ': d *= 1.1
        if c in '.,:\n': d *= 2.2
        time.sleep(d)

def combo(*names):
    """combo('ctrl','alt','t')"""
    mods = {'ctrl': K.KEY_LEFTCTRL, 'alt': K.KEY_LEFTALT, 'shift': K.KEY_LEFTSHIFT, 'super': K.KEY_LEFTMETA}
    *ms, last = names
    m = [mods[x] for x in ms]
    key = {'enter': K.KEY_ENTER, 'esc': K.KEY_ESC, 'down': K.KEY_DOWN, 'up': K.KEY_UP,
           'left': K.KEY_LEFT, 'right': K.KEY_RIGHT, 'backspace': K.KEY_BACKSPACE,
           'home': K.KEY_HOME, 'end': K.KEY_END, 'tab': K.KEY_TAB}.get(last) or LOW.get(last) or DIG.get(last)
    for x in m: ui.write(e.EV_KEY, x, 1)
    ui.syn(); time.sleep(0.03)
    ui.write(e.EV_KEY, key, 1); ui.syn(); time.sleep(0.03)
    ui.write(e.EV_KEY, key, 0); ui.syn()
    for x in reversed(m): ui.write(e.EV_KEY, x, 0)
    ui.syn()

def pause(t): time.sleep(t)

# ---- pointer (absolute, like a VM tablet) ----
from evdev import AbsInfo
SW, SH = 1920, 1080
mouse = UInput(
    {e.EV_KEY: [e.BTN_LEFT, e.BTN_RIGHT, e.BTN_MIDDLE],
     e.EV_ABS: [(e.ABS_X, AbsInfo(0, 0, 32767, 0, 0, 0)), (e.ABS_Y, AbsInfo(0, 0, 32767, 0, 0, 0))]},
    name="meku-demo-pointer")
time.sleep(1.0)
pos = [0, 0]

def moveto(x, y):
    pos[0], pos[1] = x, y
    mouse.write(e.EV_ABS, e.ABS_X, int(x * 32767 / (SW - 1)))
    mouse.write(e.EV_ABS, e.ABS_Y, int(y * 32767 / (SH - 1)))
    mouse.syn()

def glide(x, y, dur=0.8, steps=None):
    x0, y0 = pos
    steps = steps or max(8, int(dur * 120))
    for i in range(1, steps + 1):
        t = i / steps
        t = t * t * (3 - 2 * t)
        moveto(x0 + (x - x0) * t, y0 + (y - y0) * t)
        time.sleep(dur / steps)

def click(button=e.BTN_LEFT):
    mouse.write(e.EV_KEY, button, 1); mouse.syn(); time.sleep(0.06)
    mouse.write(e.EV_KEY, button, 0); mouse.syn()
