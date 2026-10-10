<img width="1603" height="917" alt="Screenshot 2026-10-10 084855" src="https://github.com/user-attachments/assets/fdd78c54-87de-4742-9b79-f59b69ff2dff" />


# RE-Formatter

A USB drive wiper and formatter with a Windows 98 look, written in Rust with no
GUI toolkit. The window is drawn by hand against raw Win32 calls, so it looks
and behaves like a 1998 dialog instead of a modern app in a costume.

[![Build](https://github.com/drDOOM69GAMING/usb-reformat-rs/actions/workflows/build.yml/badge.svg)](https://github.com/drDOOM69GAMING/usb-reformat-rs/actions/workflows/build.yml)
[![Release](https://img.shields.io/github/v/release/drDOOM69GAMING/usb-reformat-rs?label=Release&color=28A745)](https://github.com/drDOOM69GAMING/usb-reformat-rs/releases/latest)
[![Windows](https://img.shields.io/badge/platform-Windows-0078D4?logo=windows&logoColor=white)](https://learn.microsoft.com/en-us/windows/)
[![Rust](https://img.shields.io/badge/rust-2021-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Downloads](https://img.shields.io/github/downloads/drDOOM69GAMING/usb-reformat-rs/total?label=Downloads&color=0078D4)](https://github.com/drDOOM69GAMING/usb-reformat-rs/releases)

## What it does

- Lists removable drives once per **physical disk**, so a multi partition stick
  (Rufus, Ventoy, anything with a weird layout) shows up as a single entry
- Shows the real device model name read straight from the USB descriptors
- Wipes the **whole device**, not just one partition, then rebuilds a single
  partition across all of it
- Formats to FAT32 or exFAT and verifies the result afterwards
- Pause and cancel a long wipe without killing the write halfway
- Progress bar driven by the real overwrite, not a fake animation
- A chiptune player built into the exe, playing from memory with no temp files

Everything runs as administrator, because raw disk writes require it.

## Download

Grab `usb-reformat.exe` from the
[releases page](https://github.com/drDOOM69GAMING/usb-reformat-rs/releases).
It is a single file with the icon, the manifest and the music all embedded.

## Build from source

Needs the Rust stable toolchain for `x86_64-pc-windows-gnu` and
[build-tools](https://winlibs.com/) for the GNU linker.

```sh
git clone https://github.com/drDOOM69GAMING/usb-reformat-rs
cd usb-reformat-rs
.\build.ps1
```

The finished exe stays at `target\release\usb-reformat.exe` and is not copied
anywhere else.

### Do not use plain `cargo build`

`cargo build` produces a working exe but **without** the embedded icon, the
application manifest or the music. Plain cargo relinks the binary and drops all
of it. Always go through `build.ps1`, which runs the `seticon` tool afterwards to
patch the resources in.

## Music

The 100 chiptunes live in [`music/`](music/) in this repo, and `build.ps1`
embeds them into the exe as RCDATA resources. At runtime they are played
straight out of the executable's own resources, so nothing is ever unpacked to
`%TEMP%`. Drop any extra `.xm`, `.it` or `.mod` files in there and they get
embedded too.

Playback picks a random track at startup and moves on when one finishes. The
end of a track is detected by the player reporting the module as finished, not
by a timer, so long songs play all the way through. One track in the collection
(`4mat - blank page.xm`) has a pattern jump that never resolves and never goes
quiet, and a 15 minute guard exists purely to skip that one.

## How it works

Five stages, logged to `usb-reformat_<timestamp>.log` next to the exe:

1. `diskpart clean` removes every partition on the disk
2. The volume is locked and dismounted
3. The raw device is overwritten. One pass for a full wipe, or skipped entirely
   for a quick wipe. Up to 35 passes for a Gutmann wipe
4. A single partition is created and formatted
5. The result is verified and reported

Pause is a flag file polled by the worker between 1 MiB chunks, so a paused
write stops promptly without killing the process.

## Layout

| Path | What |
|---|---|
| `src/main.rs` | Entry point, worker spawn |
| `src/gui.rs` | The window. Painting, controls, messages |
| `src/core.rs` | Wipe pipeline, diskpart, raw device I/O, logging |
| `src/win32.rs` | Hand written FFI to user32, gdi32, comctl32, dwmapi |
| `src/music.rs` | Chiptune player and playlist |
| `src/bin/seticon.rs` | Build tool. Patches icon, manifest and music into the exe |

## Warning

This tool destroys data. It overwrites whole physical devices at the raw block
level and the writes cannot be undone. Double check the disk number and drive
letter before you tick the box. Fixed disks are blocked by a guard, but a USB
drive that reports itself as fixed will not be caught by it.
