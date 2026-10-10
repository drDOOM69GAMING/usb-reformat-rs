// seticon — embed an .ico into a PE file as RT_ICON + RT_GROUP_ICON so that
// Explorer's file icon, the taskbar and the window all show it, embed an
// application manifest as RT_MANIFEST, and optionally embed a folder of files
// (the chiptunes) as RT_RCDATA resources named after each file.
//
// Using the documented BeginUpdateResource / UpdateResource Windows APIs, so no
// resource compiler (windres/rc/llvm-rc) is required.
//
// Usage: seticon <pe-file> <icon.ico> [manifest.xml] [music-dir]
//
// Build: cargo build --release --bin seticon

use std::ffi::c_void;
use std::process::exit;

type HANDLE = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn BeginUpdateResourceW(p_file_name: *const u16, b_delete_existing: i32) -> HANDLE;
    fn UpdateResourceW(
        h_update: HANDLE,
        lp_type: *const u16,
        lp_name: *const u16,
        w_language: u16,
        lp_data: *const c_void,
        cb: u32,
    ) -> i32;
    fn EndUpdateResourceW(h_update: HANDLE, f_discard: i32) -> i32;
    fn GetLastError() -> u32;
}

const RT_ICON: usize = 3;
const RT_GROUP_ICON: usize = 14;
const RT_MANIFEST: usize = 24;
const RT_RCDATA: usize = 10;
// CREATEPROCESS_MANIFEST_RESOURCE_ID: the id the loader looks for.
const MANIFEST_ID: usize = 1;
const LANG_NEUTRAL: u16 = 0x0000;

/// Tracker file extensions we will embed from the music folder.
const MUSIC_EXTS: [&str; 4] = ["xm", "it", "mod", "s3m"];

/// RCDATA resource that lists the names of every embedded track.
const MUSIC_INDEX_NAME: &str = "_music_index";

fn is_music(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => MUSIC_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)),
        None => false,
    }
}

fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

struct Img {
    width: u8,
    height: u8,
    color_count: u8,
    planes: u16,
    bit_count: u16,
    data: Vec<u8>,
}

fn parse_ico(bytes: &[u8]) -> Option<(Vec<Img>, Vec<u8>)> {
    if bytes.len() < 6 || le16(bytes, 2) != 1 {
        return None;
    }
    let count = le16(bytes, 4) as usize;
    if count == 0 {
        return None;
    }
    let mut imgs: Vec<Img> = Vec::new();
    for i in 0..count {
        let e = 6 + i * 16;
        if e + 16 > bytes.len() {
            break;
        }
        let size = le32(bytes, e + 8) as usize;
        let off = le32(bytes, e + 12) as usize;
        if off + size > bytes.len() || size == 0 {
            continue;
        }
        imgs.push(Img {
            width: bytes[e],
            height: bytes[e + 1],
            color_count: bytes[e + 2],
            planes: le16(bytes, e + 4),
            bit_count: le16(bytes, e + 6),
            data: bytes[off..off + size].to_vec(),
        });
    }
    if imgs.is_empty() {
        return None;
    }

    // GRPICONDIR + GRPICONDIRENTRY[n]
    let n = imgs.len();
    let mut group = Vec::with_capacity(6 + n * 14);
    group.extend_from_slice(&0u16.to_le_bytes()); // idReserved
    group.extend_from_slice(&1u16.to_le_bytes()); // idType = icon
    group.extend_from_slice(&(n as u16).to_le_bytes()); // idCount
    for (i, img) in imgs.iter().enumerate() {
        group.push(img.width);
        group.push(img.height);
        group.push(img.color_count);
        group.push(0); // reserved
        group.extend_from_slice(&img.planes.to_le_bytes());
        group.extend_from_slice(&img.bit_count.to_le_bytes());
        group.extend_from_slice(&(img.data.len() as u32).to_le_bytes());
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes()); // nID -> RT_ICON id
    }
    Some((imgs, group))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: seticon <pe-file> <icon.ico> [manifest.xml] [music-dir]");
        exit(2);
    }
    let pe_path = &args[1];
    let ico_path = &args[2];
    let manifest_path = args.get(3).map(|s| s.as_str());
    let music_dir = args.get(4).map(|s| s.as_str());

    // Collect the tracker files to embed, sorted so builds are reproducible.
    let mut tracks: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(dir) = music_dir {
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("cannot read music dir {}: {}", dir, e);
                exit(2);
            }
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if !path.is_file() {
                continue;
            }
            let Some(fname) = path.file_name() else {
                continue;
            };
            let fname = fname.to_string_lossy().into_owned();
            if !is_music(&fname) {
                continue;
            }
            match std::fs::read(&path) {
                Ok(d) => tracks.push((fname, d)),
                Err(e) => eprintln!("skip {}: {}", fname, e),
            }
        }
        tracks.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let tracks_bytes: usize = tracks.iter().map(|(_, d)| d.len()).sum();

    let manifest = manifest_path.and_then(|p| match std::fs::read(p) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("cannot read {}: {}", p, e);
            exit(2);
        }
    });

    let bytes = std::fs::read(ico_path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", ico_path, e);
        exit(2);
    });
    let (imgs, group) = parse_ico(&bytes).unwrap_or_else(|| {
        eprintln!("{} is not a valid .ico file", ico_path);
        exit(2);
    });

    let path = wide(pe_path);
    unsafe {
        let h = BeginUpdateResourceW(path.as_ptr(), 0);
        if h.is_null() {
            eprintln!(
                "BeginUpdateResource failed on {} (err {})",
                pe_path,
                GetLastError()
            );
            exit(1);
        }

        for (i, img) in imgs.iter().enumerate() {
            let id = (i + 1) as usize as *const u16;
            let ok = UpdateResourceW(
                h,
                RT_ICON as usize as *const u16,
                id,
                LANG_NEUTRAL,
                img.data.as_ptr() as *const c_void,
                img.data.len() as u32,
            );
            if ok == 0 {
                eprintln!(
                    "UpdateResource(RT_ICON {}) failed (err {})",
                    i + 1,
                    GetLastError()
                );
                EndUpdateResourceW(h, 1);
                exit(1);
            }
        }

        let ok = UpdateResourceW(
            h,
            RT_GROUP_ICON as usize as *const u16,
            1usize as *const u16,
            LANG_NEUTRAL,
            group.as_ptr() as *const c_void,
            group.len() as u32,
        );
        if ok == 0 {
            eprintln!(
                "UpdateResource(RT_GROUP_ICON) failed (err {})",
                GetLastError()
            );
            EndUpdateResourceW(h, 1);
            exit(1);
        }

        if let Some(m) = manifest.as_ref() {
            let ok = UpdateResourceW(
                h,
                RT_MANIFEST as usize as *const u16,
                MANIFEST_ID as usize as *const u16,
                LANG_NEUTRAL,
                m.as_ptr() as *const c_void,
                m.len() as u32,
            );
            if ok == 0 {
                eprintln!(
                    "UpdateResource(RT_MANIFEST) failed (err {})",
                    GetLastError()
                );
                EndUpdateResourceW(h, 1);
                exit(1);
            }
        }

        // Chiptunes as RT_RCDATA, named after the file. They are read straight
        // out of the exe at runtime, so nothing is ever written to disk.
        for (fname, data) in tracks.iter() {
            let rname = wide(fname);
            let ok = UpdateResourceW(
                h,
                RT_RCDATA as usize as *const u16,
                rname.as_ptr(),
                LANG_NEUTRAL,
                data.as_ptr() as *const c_void,
                data.len() as u32,
            );
            if ok == 0 {
                eprintln!(
                    "UpdateResource(RT_RCDATA '{}') failed (err {})",
                    fname,
                    GetLastError()
                );
                EndUpdateResourceW(h, 1);
                exit(1);
            }
        }

        // A plain index resource listing every embedded track, so the app can discover
        // its music with one FindResource call instead of EnumResourceNamesW
        // (whose callback marshalling is easy to get subtly wrong).
        let mut index_bytes = Vec::new();
        for (fname, _) in tracks.iter() {
            index_bytes.extend_from_slice(fname.as_bytes());
            index_bytes.push(b'\n');
        }
        if !index_bytes.is_empty() {
            let iname = wide(MUSIC_INDEX_NAME);
            let ok = UpdateResourceW(
                h,
                RT_RCDATA as usize as *const u16,
                iname.as_ptr(),
                LANG_NEUTRAL,
                index_bytes.as_ptr() as *const c_void,
                index_bytes.len() as u32,
            );
            if ok == 0 {
                eprintln!(
                    "UpdateResource(RT_RCDATA index) failed (err {})",
                    GetLastError()
                );
                EndUpdateResourceW(h, 1);
                exit(1);
            }
        }

        if EndUpdateResourceW(h, 0) == 0 {
            eprintln!("EndUpdateResource failed (err {})", GetLastError());
            exit(1);
        }
    }

    println!("embedded {} icon image(s) into {}", imgs.len(), pe_path);
    if manifest.is_some() {
        println!("embedded application manifest into {}", pe_path);
    }
    if !tracks.is_empty() {
        println!(
            "embedded {} chiptune(s), {:.2} MB, into {}",
            tracks.len(),
            tracks_bytes as f64 / 1e6,
            pe_path
        );
    }
}
