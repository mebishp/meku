"""Record the Meku demo: real key presses and mouse via /dev/uinput, window recorded
with GNOME's screencast service. Writes raw.mp4 and events.json."""
import json, os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib
import drive

OUT = '/tmp/claude-1000/demo/raw'
AREA = (0, 32, 1920, 1048)   # the maximized window, without the GNOME top bar
PIPE = 'videoconvert ! queue ! x264enc pass=quant quantizer=10 speed-preset=ultrafast ! h264parse ! mp4mux'

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
def shell(method, params):
    return bus.call_sync('org.gnome.Shell.Screencast', '/org/gnome/Shell/Screencast',
                         'org.gnome.Shell.Screencast', method, params, None,
                         Gio.DBusCallFlags.NONE, -1, None)

events, marks = [], {}
t0 = [None]
def now(): return time.time() - t0[0]
def mark(name): marks[name] = round(now(), 3)

_move, _click = drive.moveto, drive.click
def moveto(x, y):
    _move(x, y)
    if t0[0] is not None: events.append({'t': round(now(), 3), 'x': round(x, 1), 'y': round(y, 1)})
def click(*a):
    if t0[0] is not None: events.append({'t': round(now(), 3), 'click': True, 'x': drive.pos[0], 'y': drive.pos[1]})
    _click(*a)
drive.moveto, drive.click = moveto, click

# --- not recorded: focus the window and maximize it ---
drive.pos[:] = [1700, 950]; _move(1700, 950); time.sleep(0.4)
drive.glide(900, 420, 0.5); drive.click(); time.sleep(0.3)
drive.combo('super', 'up'); time.sleep(1.6)
drive.glide(1700, 950, 0.5); time.sleep(0.6)

ok, path = shell('ScreencastArea', GLib.Variant('(iiiisa{sv})', (*AREA, OUT, {
    'framerate': GLib.Variant('i', 60), 'draw-cursor': GLib.Variant('b', False),
    'pipeline': GLib.Variant('s', PIPE)}))).unpack()
t0[0] = time.time()
print('recording', ok, path, flush=True)
try:
    t = drive.type_text; SP = 1.7
    drive.pause(1.0)
    events.append({'t': round(now(), 3), 'x': 1700, 'y': 950})
    drive.glide(1000, 500, 0.8); drive.pause(0.15); mark('doc_click'); drive.click(); drive.pause(0.5)
    mark('type_start')
    t("# Meet Meku\n\n", speed=SP); drive.pause(0.3)
    t("Write in **markdown** and watch it turn into *live* formatting.\n\n", speed=SP); drive.pause(0.3)
    t("- Lists that keep going\n[ ] Checkboxes you can tick\n\n\n", speed=SP); drive.pause(0.4)
    mark('type1_end')
    drive.glide(868, 381, 0.9); drive.pause(0.25); mark('cb_click'); drive.click(); drive.pause(0.9)
    drive.glide(1500, 700, 0.7); drive.pause(0.2)
    mark('callout_start')
    t("> [!tip] Callouts\nTables and code blocks too.", speed=SP)
    mark('callout_end'); drive.pause(1.0)
    mark('themes_start')
    for hold in (1.5, 1.0, 1.0, 1.2, 1.0, 1.5):   # Dark, Solarized L/D, Nord, Gruvbox, Dracula
        drive.combo('ctrl', 'alt', 't'); events.append({'t': round(now(), 3), 'theme': True}); drive.pause(hold)
    mark('themes_end')
    drive.glide(1700, 950, 0.7); drive.pause(0.6)
finally:
    dur = now()
    print('stop', shell('StopScreencast', None).unpack(), round(dur, 2), flush=True)
os.replace(path, OUT + '.mp4')
json.dump({'duration': dur, 'area': AREA, 'marks': marks, 'events': events}, open('/tmp/claude-1000/demo/events.json', 'w'))
