// usb-reformat — wipe, repartition and format USB drives (exFAT/FAT32).
//
//   usb-reformat                      classic Windows 98 style GUI (default)
//   usb-reformat --run ...            headless elevated worker (used by the GUI)
//   usb-reformat --work               legacy interactive console flow
//   usb-reformat --list               list detected drives (read-only)
//   usb-reformat --refresh            nudge File Explorer (read-only)
//
// No external crates: std + a small Win32 FFI layer (see win32.rs).

#![windows_subsystem = "windows"]

mod core;
mod gui;
mod music;
mod win32;

use std::path::PathBuf;

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// Parse the worker command line:
///   --run --drives G,H --fs exfat|fat32 [--passes N | --full] --log "C:\path\log.txt"
fn parse_worker_args(args: &[String]) -> Option<(Vec<char>, String, u32, PathBuf)> {
    let mut drives: Vec<char> = Vec::new();
    let mut fs_kind = "exFAT".to_string();
    let mut passes: u32 = 0;
    let mut log: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--drives" => {
                if i + 1 < args.len() {
                    drives = core::parse_letters(&args[i + 1]);
                    i += 1;
                }
            }
            "--fs" => {
                if i + 1 < args.len() {
                    fs_kind = if args[i + 1].eq_ignore_ascii_case("fat32") {
                        "FAT32".to_string()
                    } else {
                        "exFAT".to_string()
                    };
                    i += 1;
                }
            }
            "--passes" => {
                if i + 1 < args.len() {
                    passes = args[i + 1].parse().unwrap_or(passes);
                    i += 1;
                }
            }
            "--full" => {
                if passes == 0 {
                    passes = 1;
                }
            }
            "--log" => {
                if i + 1 < args.len() {
                    log = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    log.map(|l| (drives, fs_kind, passes, l))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Headless elevated worker launched by the GUI. No console, no window.
    if has_flag(&args, "--run") {
        let code = match parse_worker_args(&args) {
            Some((drives, fs_kind, passes, log)) if !drives.is_empty() => {
                core::run_worker(&drives, &fs_kind, passes, &log)
            }
            _ => 2,
        };
        std::process::exit(code);
    }

    if has_flag(&args, "--work") {
        core::init_io();
        core::run_workflow();
        core::refresh_explorer();
        core::pause_console();
        return;
    }

    if has_flag(&args, "--refresh") {
        core::init_io();
        core::refresh_explorer();
        core::sayln("Explorer refreshed.");
        core::pause_console();
        return;
    }

    if has_flag(&args, "--music") {
        // Diagnostic: list the chiptunes embedded in this exe.
        let names = win32::music_index();
        println!("embedded chiptunes: {}", names.len());
        for n in names.iter().take(10) {
            let sz = win32::resource_bytes(n).map(|b| b.len()).unwrap_or(0);
            println!("  {} ({} bytes)", n, sz);
        }
        // Sanity: can we read one by name directly?
        let probe = "pk - Chip orgie.xm";
        match win32::resource_bytes(probe) {
            Some(b) => println!("direct lookup '{}' -> {} bytes", probe, b.len()),
            None => println!("direct lookup '{}' -> NOT FOUND", probe),
        }
        return;
    }

    if has_flag(&args, "--list") {
        core::init_io();
        core::list_drives();
        core::pause_console();
        return;
    }

    // Default: the GUI.
    gui::run();
}
