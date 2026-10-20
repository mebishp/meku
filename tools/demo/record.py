import json, sys, time
sys.path.insert(0, '/tmp/claude-1000/demo')
import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib
import drive

OUT = '/tmp/claude-1000/demo/raw.mp4'
AREA = (430, 196, 1060, 720)

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
def shell(method, params):
    return bus.call_sync('org.gnome.Shell.Screencast', '/org/gnome/Shell/Screencast',
                         'org.gnome.Shell.Screencast', method, params, None,
                         Gio.DBusCallFlags.NONE, -1, None)

events = []
t0 = [0.0]
def now(): return time.time() - t0[0]

_move, _click = drive.moveto, drive.click
def moveto(x, y):
    _move(x, y); events.append({'t': round(now(), 3), 'x': round(x, 1), 'y': round(y, 1)})
def click(*a):
    events.append({'t': round(now(), 3), 'click': True, 'x': drive.pos[0], 'y': drive.pos[1]}); _click(*a)
drive.moveto, drive.click = moveto, click

drive.pos[:] = [1700, 950]; _move(1700, 950)
time.sleep(0.5)
ok, path = shell('ScreencastArea', GLib.Variant('(iiiisa{sv})', (*AREA, OUT, {
    'framerate': GLib.Variant('i', 30), 'draw-cursor': GLib.Variant('b', False)}))).unpack()
t0[0] = time.time()
print('recording', ok, path, flush=True)
try:
    t = drive.type_text
    drive.pause(1.2)
    events.append({'t': round(now(), 3), 'x': 1700, 'y': 950})
    drive.glide(900, 420, 0.8); drive.pause(0.15); drive.click(); drive.pause(0.5)
    t("# Meet Meku\n\n", speed=1.35)
    drive.pause(0.35)
    t("Write in **markdown** and watch it turn into *live* formatting.\n\n", speed=1.35)
    drive.pause(0.35)
    t("- Lists that keep going\n[ ] Checkboxes you can tick\n\n\n", speed=1.35)
    drive.pause(0.35)
    t("> [!tip] Callouts\nTables and code blocks too.", speed=1.35)
    drive.pause(0.9)
    drive.glide(788, 466, 0.9); drive.pause(0.25); drive.click(); drive.pause(1.1)
    drive.glide(1250, 700, 0.8); drive.pause(0.3)
    for hold in (1.7, 1.2, 1.2, 1.4, 1.2, 1.8):   # Dark, Solarized L/D, Nord, Gruvbox, Dracula
        drive.combo('ctrl', 'alt', 't'); events.append({'t': round(now(), 3), 'theme': True}); drive.pause(hold)
    drive.glide(1700, 950, 0.7); drive.pause(0.6)
finally:
    dur = now()
    print('stop', shell('StopScreencast', None).unpack(), round(dur, 2), flush=True)
json.dump({'duration': dur, 'area': AREA, 'events': events}, open('/tmp/claude-1000/demo/events.json', 'w'))
