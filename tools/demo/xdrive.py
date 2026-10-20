"""Keyboard and pointer for an isolated X display (Xwayland), through XTEST.

Nothing here touches the real desktop: events go only to the X server named by
$DISPLAY, which is the throwaway one the clips are recorded on.
"""
import random, time
from Xlib import X, XK, display as xdisplay
from Xlib.ext import xtest

d = xdisplay.Display()
root = d.screen().root
pos = [0, 0]

SHIFT = XK.string_to_keysym("Shift_L")
MODS = {"ctrl": "Control_L", "alt": "Alt_L", "shift": "Shift_L", "super": "Super_L"}
NAMED = {"enter": "Return", "esc": "Escape", "tab": "Tab", "backspace": "BackSpace",
         "up": "Up", "down": "Down", "left": "Left", "right": "Right",
         "home": "Home", "end": "End", "pgdn": "Next", "pgup": "Prior", "space": "space"}


def _code(keysym):
    return d.keysym_to_keycode(keysym)


def _key(code, down):
    xtest.fake_input(d, X.KeyPress if down else X.KeyRelease, code)


def _sym(name):
    name = NAMED.get(name, name)
    sym = XK.string_to_keysym(name)
    if not sym:
        raise KeyError(name)
    return sym


def tap(sym, shift=False, mods=()):
    held = [_code(_sym(MODS[m])) for m in mods] + ([_code(SHIFT)] if shift else [])
    for c in held:
        _key(c, True)
    _key(_code(sym), True)
    d.sync()
    time.sleep(0.012)
    _key(_code(sym), False)
    for c in reversed(held):
        _key(c, False)
    d.sync()


def char(c):
    names = {" ": "space", "\n": "Return"}
    sym = XK.string_to_keysym(names.get(c, c)) or ord(c)
    shift = d.keycode_to_keysym(_code(sym), 0) != sym
    tap(sym, shift=shift)


def type_text(s, speed=1.0, jitter=0.35):
    for c in s:
        char(c)
        t = 0.075 / speed * random.uniform(1 - jitter, 1 + jitter)
        if c == " ":
            t *= 1.1
        if c in ".,:\n":
            t *= 2.2
        time.sleep(t)


def combo(*names):
    *mods, last = names
    tap(_sym(last), mods=mods)


def pause(t):
    time.sleep(t)


def moveto(x, y):
    pos[0], pos[1] = x, y
    xtest.fake_input(d, X.MotionNotify, False, X.CurrentTime, root, int(x), int(y))
    d.sync()


def glide(x, y, dur=0.7, fps=90):
    x0, y0 = pos
    steps = max(6, int(dur * fps))
    for i in range(1, steps + 1):
        t = i / steps
        t = t * t * (3 - 2 * t)
        moveto(x0 + (x - x0) * t, y0 + (y - y0) * t)
        time.sleep(dur / steps)


def _button(n, is_down):
    xtest.fake_input(d, X.ButtonPress if is_down else X.ButtonRelease, n)
    d.sync()


def click(button=1, times=1):
    for _ in range(times):
        _button(button, True)
        time.sleep(0.05)
        _button(button, False)
        time.sleep(0.06)


def right_click():
    click(3)


def down(button=1):
    _button(button, True)


def up(button=1):
    _button(button, False)


def wheel(notches, delay=0.03):
    """Positive scrolls down."""
    b = 5 if notches > 0 else 4
    for _ in range(abs(notches)):
        click(b)
        time.sleep(delay)
