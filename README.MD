# Meku

A fast, simple markdown editor for your notes. Meku opens a folder of plain
`.md` files and lets you write in a clean live preview. It is a native app
(no browser inside), so it starts quickly and stays smooth, even with big notes.

Your notes stay ordinary markdown files on your disk. Meku never locks them
into a special format.

## Features

- **Live preview.** Write markdown and see it formatted as you type. The
  markup only shows on the line you are editing.
- **Three views.** Switch between Edit (plain markdown), Live and Read.
- **Folders of notes.** Open any folder (Meku calls it a *mekuto*). Browse
  notes in the sidebar, with an outline of the headings in the open note.
- **Tabs.** Keep several notes open. Meku remembers your last folder and tabs
  and brings them back next time.
- **Quick everything.** `Ctrl+P` finds a note, `Ctrl+Shift+P` runs any command,
  `Ctrl+F` finds text in a note, and `Ctrl+Shift+F` searches the whole folder.
- **Helpful writing.** Smart lists and checkboxes, auto-closing brackets,
  tables, callouts, images (paste or drop them in) and a `/` menu for quick
  inserts.
- **Themes.** Light and dark themes, follow your system, or make your own.
- **Settings.** Change how Meku behaves for every folder, or only for one.
  Press `Ctrl+,` to open them.
- **Safe by default.** Autosave, and Meku tells you if a note changes on disk
  while it is open.

Not in this first version: links between notes, plugins and sync.

## Install

Download the file for your system from the
[latest release](https://github.com/mebishp/meku/releases/latest):

| System | File |
|---|---|
| Ubuntu, Debian | `.deb` — run `sudo apt install ./meku_0.1.0_amd64.deb` |
| Arch Linux | `.pkg.tar.zst` — run `sudo pacman -U meku-0.1.0-1-x86_64.pkg.tar.zst` |
| Any Linux | `.AppImage` — make it executable and run it |
| Any Linux | `.tar.gz` — unpack it and run `meku` |

Meku needs a graphics driver with Vulkan support, which most computers
already have. It currently runs on Linux; macOS and Windows may come later.

## Use it

Start Meku and choose **Open folder** (`Ctrl+O`), or give it a folder:

```
meku ~/Notes
```

Press `Ctrl+T` for a new note and `Ctrl+E` to change view. A full list of
shortcuts is in the menu (the `⋯` button at the top of the editor).

## Build from source

You need [Rust](https://rustup.rs) (a recent version).

```
git clone https://github.com/mebishp/meku.git
cd meku
cargo run --release
```

To build install packages (deb, AppImage, Arch and tar.gz):

```
tools/package.sh
```

The files appear in `dist/`.

## License

Meku is released under the [Apache License 2.0](LICENSE).
