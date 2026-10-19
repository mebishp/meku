#!/usr/bin/env python3
"""Screenshot MCP server - captures the full desktop via the GNOME xdg-desktop-portal
Screenshot API (non-interactive; works on GNOME Wayland where org.gnome.Shell.Screenshot
D-Bus is extension-gated and maim/scrot/grim are unavailable).

Run with: uv run --with mcp python3 screenshot_server.py
"""
import time

from mcp.server.mcpserver import MCPServer

import gi

gi.require_version("GLib", "2.0")
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

mcp = MCPServer("screenshot")

REQUEST_TIMEOUT_S = 15


def portal_screenshot() -> str:
    loop = GLib.MainLoop()
    result: dict = {}

    def on_response(conn, sender, path, iface, signal, params):
        result["code"] = params[0]
        if len(params) > 1 and isinstance(params[1], dict):
            result["uri"] = params[1].get("uri")
        loop.quit()

    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    sub = conn.signal_subscribe(
        None, "org.freedesktop.portal.Request", "Response",
        None, None, Gio.DBusSignalFlags.NONE, on_response,
    )
    try:
        variants = GLib.Variant("a{sv}", {
            "interactive": GLib.Variant("b", False),
            "handle_token": GLib.Variant("s", f"mcp{int(time.time() * 10)}"),
        })
        conn.call_sync(
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Screenshot", "Screenshot",
            GLib.Variant.new_tuple(GLib.Variant.new_string(""), variants),
            None, Gio.DBusCallFlags.NONE, -1, None,
        )
    except GLib.Error as e:
        raise RuntimeError(f"screenshot portal call failed: {e}") from e

    GLib.timeout_add_seconds(REQUEST_TIMEOUT_S, loop.quit)
    loop.run()
    conn.signal_unsubscribe(sub)
    loop = None

    code = result.get("code")
    uri = result.get("uri")
    if code != 0 or not uri or not uri.startswith("file://"):
        raise RuntimeError(
            f"screenshot portal returned code {code} (uri={uri!r}); "
            f"session may need approval or the desktop is not GNOME"
        )
    return uri[len("file://"):]


@mcp.tool()
def take_screenshot() -> str:
    """Capture a full-screen screenshot of the current desktop via the GNOME
    screenshot portal (non-interactive). Returns the absolute path to the
    saved PNG View it with the Read tool to verify UI changes visually."""
    return portal_screenshot()


if __name__ == "__main__":
    mcp.run()
