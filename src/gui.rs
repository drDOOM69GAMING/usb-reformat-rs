// Classic Windows 98 style GUI.
//
// The GUI process runs NON-elevated (so Shell change notifications reach
// Explorer and the freshly formatted drive auto-appears). Clicking WIPE
// launches a hidden, elevated `--run` worker for the selected drives (one UAC
// prompt) and polls it, tailing the worker's log file into the log pane.
#![allow(dead_code)]

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::core;
use crate::win32::*;

// ---------------- control ids ----------------

const ID_LIST: i32 = 1001;
const ID_RADIO_EXFAT: i32 = 1002;
const ID_RADIO_FAT32: i32 = 1003;
const ID_CHECK: i32 = 1004;
const ID_CONFIRM: i32 = 1005;
const ID_RESCAN: i32 = 1006;
const ID_WIPE: i32 = 1007;
const ID_PROGRESS: i32 = 1008;
const ID_LOG: i32 = 1009;
const ID_STATUS: i32 = 1010;
const ID_HIDE_FIXED: i32 = 1011;
const ID_FULL: i32 = 1012;
const ID_PAUSE: i32 = 1013;
const ID_CANCEL: i32 = 1014;
const ID_EXIT: i32 = 1015;
const ID_LOG_TOGGLE: i32 = 1016;
const ID_MUSIC: i32 = 1018;

// Pass-selection dialog control ids (its own child namespace).
const PID_R1: i32 = 2001;
const PID_R3: i32 = 2002;
const PID_R7: i32 = 2003;
const PID_R35: i32 = 2004;
const PID_OK: i32 = 2010;
const PID_CANCEL: i32 = 2011;

const FULL_WIPE_OFF: &str = "Full wipe (slow) - click to choose passes.  Unticked = quick wipe.";

const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 250;

// ---------------- theme ----------------

const ID_THEME: i32 = 1017;

const CLR_BLACK: u32 = 0x0000_0000;
const CLR_WHITE: u32 = 0x00FF_FFFF;
// Real Win98 palette. Modern Windows reports COLOR_BTNFACE as #F0F0F0, which
// reads as "Windows 2000+"; 98's button face was the warmer #D4D0C8.
const CLR_98_FACE: u32 = 0x00C8_D0D4;
const CLR_98_SHADOW: u32 = 0x0080_8080;
const CLR_DARK_BG: u32 = 0x001F_1F1F;
const CLR_DARK_EDIT_BG: u32 = 0x002D_2D2D;
const CLR_DARK_TEXT: u32 = 0x00FF_FFFF;
const CLR_DARK_SEL: u32 = 0x0040_40A0;

// Theme state deliberately lives outside the App mutex. WM_CTLCOLOR* and
// WM_ERASEBKGND fire while the parent is painting, which can happen while a
// caller still holds the App lock (e.g. set_status_text repaints a label), so
// locking App there would deadlock on this single-threaded, non-reentrant
// mutex. A plain atomic flag plus GDI brushes avoid that entirely.
static DARK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// Stock brushes stay valid for the process lifetime, so plain statics are safe
// and need no OnceLock (which would require the raw pointer to be Sync).
static mut DARK_FACE_BRUSH: HBRUSH = std::ptr::null_mut();
static mut DARK_SURFACE_BRUSH: HBRUSH = std::ptr::null_mut();
static mut FACE_98_BRUSH: HBRUSH = std::ptr::null_mut();
static mut SHADOW_BRUSH: HBRUSH = std::ptr::null_mut();

fn dark_mode() -> bool {
    DARK_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

fn dark_face_brush() -> HBRUSH {
    unsafe {
        if DARK_FACE_BRUSH.is_null() {
            DARK_FACE_BRUSH = CreateSolidBrush(CLR_DARK_BG);
        }
        DARK_FACE_BRUSH
    }
}

fn dark_surface_brush() -> HBRUSH {
    unsafe {
        if DARK_SURFACE_BRUSH.is_null() {
            DARK_SURFACE_BRUSH = CreateSolidBrush(CLR_DARK_EDIT_BG);
        }
        DARK_SURFACE_BRUSH
    }
}

static mut GROUP_LINE_98: HBRUSH = std::ptr::null_mut();
static mut GROUP_LINE_DARK: HBRUSH = std::ptr::null_mut();

fn group_line_brush(dark: bool) -> HBRUSH {
    unsafe {
        if dark {
            if GROUP_LINE_DARK.is_null() {
                GROUP_LINE_DARK = CreateSolidBrush(0x0060_6060);
            }
            GROUP_LINE_DARK
        } else {
            if GROUP_LINE_98.is_null() {
                GROUP_LINE_98 = CreateSolidBrush(0x0080_8080);
            }
            GROUP_LINE_98
        }
    }
}

/// Draw one etched group frame with a gap for `cap` (its caption label).
fn paint_group(hdc: HDC, frame: &RECT, cap: &RECT, dark: bool) {
    unsafe {
        FrameRect(hdc, frame, group_line_brush(dark));
        // Punch the caption out of the top edge.
        FillRect(
            hdc,
            cap,
            if dark {
                dark_face_brush()
            } else {
                GetSysColorBrush(COLOR_3DFACE)
            },
        );
    }
}

/// Win98 dialog background (#D4D0C8). Modern COLOR_BTNFACE is #F0F0F0, which
/// looks too modern, so the face colour is painted explicitly instead.
fn face_98_brush() -> HBRUSH {
    unsafe {
        if FACE_98_BRUSH.is_null() {
            FACE_98_BRUSH = CreateSolidBrush(CLR_98_FACE);
        }
        FACE_98_BRUSH
    }
}

fn shadow_brush() -> HBRUSH {
    unsafe {
        if SHADOW_BRUSH.is_null() {
            SHADOW_BRUSH = CreateSolidBrush(CLR_98_SHADOW);
        }
        SHADOW_BRUSH
    }
}

const ES_AUTOHSCROLL: u32 = 0x0000_0080;
const PBM_SETRANGE: u32 = 0x0401;
const PBM_SETPOS: u32 = 0x0402;
const WAIT_OBJECT_0: u32 = 0;

const CLIENT_W: i32 = 500;
const CLIENT_H: i32 = 482;
// Vertical stack. Keeping these in one place stops the create-time layout and
// the log show/hide relayout from drifting apart.
const TOOLBAR_Y: i32 = 6;
const GROUP_DRIVES_Y: i32 = 34;
const GROUP_FORMAT_Y: i32 = 176;
const CONFIRM_ROW_Y: i32 = 247;
const PROGRESS_Y: i32 = 278;
const LOG_Y: i32 = 306;
const LOG_H: i32 = 110;
const STATUS_Y: i32 = 422;
const STATUS_H: i32 = 20;
const ACTION_Y: i32 = 446;
// Vertical space the log pane (110px tall) plus its gap occupy; reclaimed when
// the log is hidden so the window shrinks instead of leaving a hole.
const LOG_OFFSET: i32 = 116;

// Win98 "group box" frames. Native BS_GROUPBOX draws its etched border with
// the DC text colour, so in dark mode it came out bright white. The frames are
// painted here instead and the caption is a plain transparent STATIC sitting
// in a gap that is punched out of the top line.
const GROUP_X: i32 = 6;
const GROUP_W: i32 = 488;
const GROUP_DRIVES_BOTTOM: i32 = 172;
const GROUP_FORMAT_BOTTOM: i32 = 246;
// Win98 dialog chrome: fixed-size window, caption, system menu, minimise box.
// Note WS_DLGFRAME (0x40000000) shares a bit with WS_POPUP and must NOT be
// ORed into an overlapped window - it turns the window into a popup. The
// classic inset dialog border comes from WS_EX_DLGMODALFRAME instead, which
// is passed as the extended style at creation time.
const MAIN_STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
const MAIN_EXSTYLE: u32 = WS_EX_DLGMODALFRAME;

// ---------------- app state ----------------

struct DriveEntry {
    letter: char,
    fixed: bool,
    media: core::MediaKind,
    /// Row text without the tick prefix, cached so the list can be re-rendered
    /// (to draw the tick) without re-querying every volume.
    text: String,
}

struct App {
    hwnd: HWND,
    list: HWND,
    radio_exfat: HWND,
    radio_fat32: HWND,
    check: HWND,
    full_wipe: HWND,
    confirm: HWND,
    btn_hide: HWND,
    btn_rescan: HWND,
    btn_wipe: HWND,
    btn_pause: HWND,
    btn_cancel: HWND,
    btn_exit: HWND,
    btn_log: HWND,
    btn_theme: HWND,
    btn_music: HWND,
    log_visible: bool,
    progress: HWND,
    log: HWND,
    status: HWND,
    font: HFONT,
    drives: Vec<DriveEntry>,
    hide_fixed: bool,
    passes: u32,
    paused: bool,
    progress_pct: u32,
    saw_overwrite: bool,
    full_run: bool,
    log_path: Option<PathBuf>,
    log_offset: u64,
    worker: Option<HANDLE>,
    busy: bool,
    tick: u32,
}

impl App {
    fn new() -> App {
        App {
            hwnd: std::ptr::null_mut(),
            list: std::ptr::null_mut(),
            radio_exfat: std::ptr::null_mut(),
            radio_fat32: std::ptr::null_mut(),
            check: std::ptr::null_mut(),
            full_wipe: std::ptr::null_mut(),
            confirm: std::ptr::null_mut(),
            btn_hide: std::ptr::null_mut(),
            btn_rescan: std::ptr::null_mut(),
            btn_wipe: std::ptr::null_mut(),
            btn_pause: std::ptr::null_mut(),
            btn_cancel: std::ptr::null_mut(),
            btn_exit: std::ptr::null_mut(),
            btn_log: std::ptr::null_mut(),
            btn_theme: std::ptr::null_mut(),
            btn_music: std::ptr::null_mut(),
            log_visible: false,
            progress: std::ptr::null_mut(),
            log: std::ptr::null_mut(),
            status: std::ptr::null_mut(),
            font: std::ptr::null_mut(),
            drives: Vec::new(),
            hide_fixed: false,
            passes: 0,
            paused: false,
            progress_pct: 0,
            saw_overwrite: false,
            full_run: false,
            log_path: None,
            log_offset: 0,
            worker: None,
            busy: false,
            tick: 0,
        }
    }
}

static APP: OnceLock<Mutex<App>> = OnceLock::new();

// The App holds raw window handles. It is only ever touched from the single UI
// thread (the window procedure and the message loop run on one thread); the
// Mutex exists purely for interior mutability inside the callback.
unsafe impl Send for App {}

fn app() -> &'static Mutex<App> {
    APP.get().expect("app state not initialised")
}

// ---------------- pass-selection dialog state ----------------

struct PassDlg {
    hwnd: HWND,
    font: HFONT,
    radios: [HWND; 4],
    done: bool,
    result: Option<u32>,
    selected: u32,
}

impl PassDlg {
    fn new() -> PassDlg {
        PassDlg {
            hwnd: std::ptr::null_mut(),
            font: std::ptr::null_mut(),
            radios: [std::ptr::null_mut(); 4],
            done: false,
            result: None,
            selected: 1,
        }
    }
}

static PASS_DLG: OnceLock<Mutex<PassDlg>> = OnceLock::new();

unsafe impl Send for PassDlg {}

fn pass_dlg() -> &'static Mutex<PassDlg> {
    PASS_DLG.get().expect("pass dialog state not initialised")
}

// ---------------- helpers ----------------

unsafe fn child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    ex: u32,
    id: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> HWND {
    let cls = to_wide(class);
    let t = to_wide(text);
    let hinst = GetModuleHandleW(std::ptr::null());
    CreateWindowExW(
        ex,
        cls.as_ptr(),
        t.as_ptr(),
        style,
        x,
        y,
        w,
        h,
        parent,
        id as isize as HMENU,
        hinst,
        std::ptr::null_mut(),
    )
}

unsafe fn set_font(h: HWND, font: HFONT) {
    SendMessageW(h, WM_SETFONT, font as usize, 1);
}

unsafe fn set_text(h: HWND, s: &str) {
    let w = to_wide(s);
    SetWindowTextW(h, w.as_ptr());
}

fn set_status_text(hwnd_status: HWND, s: &str) {
    unsafe {
        set_text(hwnd_status, s);
    }
}

fn message(hwnd: HWND, text: &str, caption: &str, flags: u32) {
    let t = to_wide(text);
    let c = to_wide(caption);
    unsafe {
        MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), flags);
    }
}

fn append_edit(edit: HWND, s: &str) {
    let len = unsafe { SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0) } as usize;
    let w = to_wide(s);
    unsafe {
        SendMessageW(edit, EM_SETSEL, len, len as isize);
        SendMessageW(edit, EM_REPLACESEL, 0, w.as_ptr() as isize);
        SendMessageW(edit, EM_SCROLLCARET, 0, 0);
    }
}

fn checked(h: HWND) -> bool {
    (unsafe { SendMessageW(h, BM_GETCHECK, 0, 0) }) == BST_CHECKED
}

// ---------------- music ----------------

/// The chiptune player, owned for the lifetime of the process.
static MUSIC: OnceLock<Mutex<crate::music::MusicPlayer>> = OnceLock::new();

fn music() -> &'static Mutex<crate::music::MusicPlayer> {
    MUSIC.get_or_init(|| Mutex::new(crate::music::MusicPlayer::new()))
}

/// Pause / resume the soundtrack from the toolbar button.
fn toggle_music() {
    let paused = music().lock().unwrap().toggle_pause();
    let (btn, status) = {
        let a = app().lock().unwrap();
        (a.btn_music, a.status)
    };
    unsafe {
        set_text(btn, if paused { "Music: off" } else { "Music: on" });
    }
    set_status_text(
        status,
        if paused {
            "Music paused."
        } else {
            "Music playing."
        },
    );
}

// ---------------- icon ----------------

fn load_icons() -> (HICON, HICON) {
    let bytes: &[u8] = include_bytes!("../ICON1.ico");
    if bytes.len() < 22 {
        return (std::ptr::null_mut(), std::ptr::null_mut());
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;

    let mut best: Option<(u32, usize, usize)> = None; // area, offset, size
    let mut best_png: Option<(u32, usize, usize)> = None;

    for i in 0..count {
        let e = 6 + i * 16;
        if e + 16 > bytes.len() {
            break;
        }
        let mut w = bytes[e] as u32;
        let mut h = bytes[e + 1] as u32;
        if w == 0 {
            w = 256;
        }
        if h == 0 {
            h = 256;
        }
        let size =
            u32::from_le_bytes([bytes[e + 8], bytes[e + 9], bytes[e + 10], bytes[e + 11]]) as usize;
        let off = u32::from_le_bytes([bytes[e + 12], bytes[e + 13], bytes[e + 14], bytes[e + 15]])
            as usize;
        if off + size > bytes.len() || size == 0 {
            continue;
        }
        let png = size >= 8 && bytes[off] == 0x89 && bytes[off + 1] == 0x50;
        let area = w * h;
        let cand = (area, off, size);
        if png {
            if best_png.map_or(true, |b| area > b.0) {
                best_png = Some(cand);
            }
        } else if best.map_or(true, |b| area > b.0) {
            best = Some(cand);
        }
    }

    let Some((_, off, size)) = best.or(best_png) else {
        return (std::ptr::null_mut(), std::ptr::null_mut());
    };

    unsafe {
        let ptr = bytes.as_ptr().add(off);
        let big = CreateIconFromResourceEx(
            ptr,
            size as u32,
            1,
            0x0003_0000,
            GetSystemMetrics(SM_CXICON),
            GetSystemMetrics(SM_CYICON),
            0,
        );
        let small = CreateIconFromResourceEx(
            ptr,
            size as u32,
            1,
            0x0003_0000,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            0,
        );
        (big, small)
    }
}

// ---------------- control creation ----------------

// Win98 used the 8pt "MS Sans Serif" pixel font at a -11 height, drawn
// without antialiasing. ClearType is deliberately avoided here (it looks
// modern and blurry at this size).
const UI_FONT_HEIGHT: i32 = -11;
const UI_FONT_NAME: &str = "MS Sans Serif";

fn create_ui_font() -> HFONT {
    let face = to_wide(UI_FONT_NAME);
    unsafe {
        CreateFontW(
            UI_FONT_HEIGHT,
            0,
            0,
            0,
            FW_NORMAL,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            DEFAULT_PITCH | FF_DONTCARE,
            face.as_ptr(),
        )
    }
}

fn create_controls(hwnd: HWND) {
    let mut a = app().lock().unwrap();
    a.hwnd = hwnd;

    a.font = create_ui_font();

    let base = WS_CHILD | WS_VISIBLE;

    unsafe {
        // Win98 dialogs frame each block of controls in an etched box with the
        // caption on the top-left edge. The frame itself is painted by the parent
        // (see paint_group) because BS_GROUPBOX draws its border in the DC text
        // colour, which is unreadable in the dark theme. Here are just the captions.
        let cap_drives = child(
            hwnd,
            "STATIC",
            "Drives to erase - the WHOLE device is wiped (all its partitions)",
            base | SS_LEFTNOWORDWRAP,
            0,
            0,
            12,
            GROUP_DRIVES_Y - 7,
            340,
            16,
        );
        let list = child(
            hwnd,
            "LISTBOX",
            "",
            base | WS_VSCROLL
                | LBS_NOTIFY
                | LBS_HASSTRINGS
                | LBS_NOINTEGRALHEIGHT
                | LBS_EXTENDEDSEL,
            WS_EX_CLIENTEDGE,
            ID_LIST,
            14,
            GROUP_DRIVES_Y + 16,
            472,
            116,
        );
        let cap_format = child(
            hwnd,
            "STATIC",
            "New partition format",
            base | SS_LEFTNOWORDWRAP,
            0,
            0,
            12,
            GROUP_FORMAT_Y - 7,
            160,
            16,
        );
        let r1 = child(
            hwnd,
            "BUTTON",
            "exFAT  (recommended)",
            base | BS_AUTORADIOBUTTON | WS_TABSTOP | WS_GROUP,
            0,
            ID_RADIO_EXFAT,
            175,
            187,
            140,
            18,
        );
        let r2 = child(
            hwnd,
            "BUTTON",
            "FAT32",
            base | BS_AUTORADIOBUTTON | WS_TABSTOP,
            0,
            ID_RADIO_FAT32,
            325,
            187,
            80,
            18,
        );
        let check = child(
            hwnd,
            "BUTTON",
            "I understand this erases ALL data on the selected drive(s).",
            base | BS_AUTOCHECKBOX | WS_TABSTOP | WS_GROUP,
            0,
            ID_CHECK,
            14,
            209,
            472,
            17,
        );
        let full = child(
            hwnd,
            "BUTTON",
            FULL_WIPE_OFF,
            base | BS_AUTOCHECKBOX | WS_TABSTOP | WS_GROUP,
            0,
            ID_FULL,
            14,
            227,
            472,
            17,
        );
        let logbtn = child(
            hwnd,
            "BUTTON",
            "Hide log",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_LOG_TOGGLE,
            388,
            TOOLBAR_Y,
            102,
            22,
        );
        let themebtn = child(
            hwnd,
            "BUTTON",
            "Dark mode",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_THEME,
            285,
            TOOLBAR_Y,
            100,
            22,
        );
        // Music pause / resume, same top toolbar row.
        let musicbtn = child(
            hwnd,
            "BUTTON",
            "Music: on",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_MUSIC,
            205,
            TOOLBAR_Y,
            72,
            22,
        );
        let label3 = child(
            hwnd,
            "STATIC",
            "Type WIPE to confirm:",
            base,
            0,
            0,
            10,
            252,
            118,
            18,
        );
        let confirm = child(
            hwnd,
            "EDIT",
            "",
            base | WS_TABSTOP | ES_AUTOHSCROLL,
            WS_EX_CLIENTEDGE,
            ID_CONFIRM,
            130,
            249,
            60,
            22,
        );
        let hide = child(
            hwnd,
            "BUTTON",
            "Hide fixed drives",
            base | BS_PUSHBUTTON | WS_TABSTOP | WS_GROUP,
            0,
            ID_HIDE_FIXED,
            195,
            CONFIRM_ROW_Y,
            100,
            26,
        );
        let rescan = child(
            hwnd,
            "BUTTON",
            "Re-scan",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_RESCAN,
            300,
            CONFIRM_ROW_Y,
            80,
            26,
        );
        let exit = child(
            hwnd,
            "BUTTON",
            "Exit",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_EXIT,
            400,
            CONFIRM_ROW_Y,
            90,
            26,
        );
        // Action row: WIPE, Pause/Resume, Cancel.
        let wipe = child(
            hwnd,
            "BUTTON",
            "WIPE DRIVE(S)",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_WIPE,
            10,
            ACTION_Y,
            170,
            28,
        );
        let pause = child(
            hwnd,
            "BUTTON",
            "Pause",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_PAUSE,
            190,
            ACTION_Y,
            140,
            28,
        );
        let cancel = child(
            hwnd,
            "BUTTON",
            "Cancel",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            ID_CANCEL,
            340,
            ACTION_Y,
            150,
            28,
        );
        // Win98 progress bar: 21px tall, segmented (no PBS_SMOOTH), sunken edges.
        let progress = child(
            hwnd,
            "msctls_progress32",
            "",
            base,
            WS_EX_CLIENTEDGE,
            ID_PROGRESS,
            10,
            PROGRESS_Y,
            480,
            21,
        );
        let log = child(
            hwnd,
            "EDIT",
            "",
            base | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            WS_EX_CLIENTEDGE,
            ID_LOG,
            10,
            LOG_Y,
            480,
            LOG_H,
        );
        // Win98 status bar: a sunken panel with the text inset from the edges.
        let status = child(
            hwnd,
            "STATIC",
            "Ready.",
            base | SS_SUNKEN | SS_LEFTNOWORDWRAP,
            0,
            ID_STATUS,
            10,
            STATUS_Y,
            480,
            STATUS_H,
        );

        for h in [
            cap_drives, list, cap_format, r1, r2, check, full, label3, confirm, hide, rescan, exit,
            logbtn, themebtn, musicbtn, wipe, pause, cancel, progress, log, status,
        ] {
            set_font(h, a.font);
        }

        // exFAT selected by default
        SendMessageW(r1, BM_SETCHECK, BST_CHECKED as usize, 0);
        // progress range 0..100
        SendMessageW(progress, PBM_SETRANGE, 0, ((100 << 16) | 0) as isize);

        a.list = list;
        a.radio_exfat = r1;
        a.radio_fat32 = r2;
        a.check = check;
        a.full_wipe = full;
        a.confirm = confirm;
        a.btn_hide = hide;
        a.btn_rescan = rescan;
        a.btn_wipe = wipe;
        a.btn_pause = pause;
        a.btn_cancel = cancel;
        a.btn_exit = exit;
        a.btn_log = logbtn;
        a.btn_theme = themebtn;
        a.btn_music = musicbtn;
        a.progress = progress;
        set_text(a.btn_theme, "Dark mode");
        a.log = log;
        a.status = status;
        // Pause / Cancel only make sense while a wipe is running.
        apply_busy(&a, false);
    }
}

// ---------------- drive list ----------------

/// Trim a model name so the row still fits the list width.
fn trim_model(model: &str) -> String {
    const MAX: usize = 26;
    if model.chars().count() <= MAX {
        return model.to_string();
    }
    let head: String = model.chars().take(MAX - 2).collect();
    format!("{}..", head.trim_end())
}

fn populate_drives() {
    let mut a = app().lock().unwrap();
    let list = a.list;
    unsafe {
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
    }
    a.drives.clear();

    let hide_fixed = a.hide_fixed;
    let sys = core::system_drive_letter();
    // A USB that Rufus/Ventoy split into several partitions exposes one drive
    // letter per lettered volume, but every one of them resolves to the SAME
    // physical disk - and the wipe runs `clean` against \\.\PhysicalDriveN.
    // Listing each letter would therefore offer the same stick twice and wipe it
    // twice. Keep the first volume seen per physical disk so one entry == one
    // whole device.
    let mut seen_disks: Vec<u32> = Vec::new();
    for ch in core::logical_drives() {
        let dt = core::drive_type(ch);
        if dt != core::DRIVE_REMOVABLE && dt != core::DRIVE_FIXED {
            continue;
        }
        if ch == sys {
            continue;
        }
        if dt == core::DRIVE_FIXED && hide_fixed {
            continue;
        }
        let Some((disk, _part)) = core::device_number(ch) else {
            continue;
        };
        if seen_disks.contains(&disk) {
            continue;
        }
        seen_disks.push(disk);
        let media = core::media_kind(ch);
        // Real model name so two identical sticks can be told apart, and so a
        // stick with no volume label is still identifiable.
        let model = core::device_model(ch).unwrap_or_default();
        let model = trim_model(&model);
        let text = match core::volume_info(ch) {
            Some(v) => {
                let tag = if dt == core::DRIVE_FIXED {
                    "FIXED - risky"
                } else {
                    "removable"
                };
                if model.is_empty() {
                    format!(
                        "{}:   {}   '{}'   {:.1} GB free   [{} / {} / disk {}]",
                        ch,
                        v.fs,
                        v.label,
                        v.free as f64 / 1e9,
                        media.label(),
                        tag,
                        disk
                    )
                } else {
                    format!(
                        "{}:   {}   '{}'   {:.1} GB free   [{} / {} / disk {}]",
                        ch,
                        model,
                        v.label,
                        v.free as f64 / 1e9,
                        media.label(),
                        tag,
                        disk
                    )
                }
            }
            None => {
                if model.is_empty() {
                    format!(
                        "{}:   (no filesystem)   [{} / disk {}]",
                        ch,
                        media.label(),
                        disk
                    )
                } else {
                    format!(
                        "{}:   {}   (no filesystem)   [{} / disk {}]",
                        ch,
                        model,
                        media.label(),
                        disk
                    )
                }
            }
        };
        a.drives.push(DriveEntry {
            letter: ch,
            fixed: dt == core::DRIVE_FIXED,
            media,
            text: text.clone(),
        });
        let w = to_wide(&text);
        unsafe {
            SendMessageW(list, LB_ADDSTRING, 0, w.as_ptr() as isize);
        }
    }
}

fn selected_drives(a: &App) -> Vec<DriveEntry> {
    let n = unsafe { SendMessageW(a.list, LB_GETSELCOUNT, 0, 0) } as usize;
    if n == 0 {
        return Vec::new();
    }
    let mut idx = vec![0i32; n];
    let got =
        unsafe { SendMessageW(a.list, LB_GETSELITEMS, n, idx.as_mut_ptr() as isize) } as usize;
    idx.truncate(got);
    idx.iter()
        .filter_map(|&i| {
            a.drives.get(i as usize).map(|d| DriveEntry {
                letter: d.letter,
                fixed: d.fixed,
                media: d.media,
                text: d.text.clone(),
            })
        })
        .collect()
}

// ---------------- busy state ----------------

/// Cancel a running wipe: terminate the elevated worker (best effort). The
/// worker keeps its diskpart child in a kill-on-close job, so terminating the
/// worker also takes a stuck diskpart down with it.
fn stop_worker() {
    let mut a = app().lock().unwrap();
    if let Some(h) = a.worker.take() {
        unsafe {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
    // Clear any pause flag so nothing is left behind for the next run.
    if let Some(log) = a.log_path.clone() {
        let _ = fs::remove_file(core::pause_flag_path_for(&log));
    }
    a.paused = false;
    a.progress_pct = 0;
    unsafe {
        KillTimer(a.hwnd, TIMER_ID);
        SendMessageW(a.progress, PBM_SETPOS, 0, 0);
        set_text(a.btn_pause, "Pause");
    }
    a.busy = false;
    apply_busy(&a, false);
    set_status_text(a.status, "Wipe cancelled.");
}

fn apply_busy(a: &App, busy: bool) {
    let en = if busy { 0 } else { 1 };
    unsafe {
        // Only the action buttons are greyed out while a wipe runs. A
        // *disabled* control stops receiving WM_CTLCOLOR*, so Windows
        // repaints it with the system disabled grey (measured darkest pixel
        // 160 vs 0 for the themed text) which made the labels look washed
        // out / blurry for the whole run. The data-entry controls therefore
        // stay enabled so they keep their themed colours; the handlers that
        // would mutate state are guarded by `busy` in the WM_COMMAND arm
        // instead.
        for h in [a.btn_hide, a.btn_rescan, a.btn_wipe] {
            EnableWindow(h, en);
        }
        for h in [
            a.list,
            a.radio_exfat,
            a.radio_fat32,
            a.check,
            a.full_wipe,
            a.confirm,
        ] {
            EnableWindow(h, 1);
        }
        // Pause / Cancel only apply while a wipe runs; Exit always works.
        EnableWindow(a.btn_pause, if busy { 1 } else { 0 });
        EnableWindow(a.btn_cancel, if busy { 1 } else { 0 });
        EnableWindow(a.btn_exit, 1);
    }
}

/// Raise / lower the pause flag the worker watches between write chunks.
fn toggle_pause() {
    let mut a = app().lock().unwrap();
    if !a.busy {
        return;
    }
    let Some(log) = a.log_path.clone() else {
        return;
    };
    let flag = core::pause_flag_path_for(&log);
    if a.paused {
        let _ = fs::remove_file(&flag);
        a.paused = false;
        unsafe {
            set_text(a.btn_pause, "Pause");
        }
        set_status_text(a.status, "Resumed - writing continues.");
    } else {
        if fs::write(&flag, b"pause").is_err() {
            set_status_text(a.status, "Could not pause (flag file not writable).");
            return;
        }
        a.paused = true;
        unsafe {
            set_text(a.btn_pause, "Resume");
        }
        set_status_text(
            a.status,
            "Paused - writing is on hold. Click Resume to continue.",
        );
    }
}

/// Hide / reveal the log pane. The controls below it (the status strip and the
/// WIPE / Pause / Cancel action row) move up and the window shrinks, so there
/// is never an empty hole left in the box.
fn toggle_log() {
    let vis = {
        let a = app().lock().unwrap();
        !a.log_visible
    };
    set_log_visible(vis);
}

/// Show or hide the log pane (also moves the controls below it and resizes
/// the window). The app starts with the log hidden.
fn set_log_visible(vis: bool) {
    let mut a = app().lock().unwrap();
    a.log_visible = vis;
    let log = a.log;
    let status = a.status;
    let wipe = a.btn_wipe;
    let pause = a.btn_pause;
    let cancel = a.btn_cancel;
    let hwnd = a.hwnd;
    unsafe {
        ShowWindow(log, if vis { SW_SHOW } else { SW_HIDE });
        // With the log shown the status strip and action row sit below it; when
        // hidden they slide up to just under the progress bar.
        let (sy, wy) = if vis {
            (STATUS_Y, ACTION_Y)
        } else {
            (PROGRESS_Y + 28, PROGRESS_Y + 52)
        };
        MoveWindow(status, 10, sy, 480, STATUS_H, 1);
        MoveWindow(wipe, 10, wy, 170, 28, 1);
        MoveWindow(pause, 190, wy, 140, 28, 1);
        MoveWindow(cancel, 340, wy, 150, 28, 1);
        set_text(a.btn_log, if vis { "Hide log" } else { "Show log" });
        // Shrink / grow the window to match the new content height.
        let client_h = if vis { CLIENT_H } else { CLIENT_H - LOG_OFFSET };
        resize_client_height(hwnd, client_h);
    }
}

/// Resize the main window to a new client height, keeping its top-left corner.
unsafe fn resize_client_height(hwnd: HWND, client_h: i32) {
    let mut fr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    AdjustWindowRectEx(&mut fr, MAIN_STYLE, 0, MAIN_EXSTYLE);
    let (frame_w, frame_h) = (fr.right - fr.left, fr.bottom - fr.top);
    let mut cur = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    GetWindowRect(hwnd, &mut cur);
    MoveWindow(
        hwnd,
        cur.left,
        cur.top,
        CLIENT_W + frame_w,
        client_h + frame_h,
        1,
    );
}

/// Apply the light or dark theme. The window background, the edit / list box
/// surfaces and the label text all come from WM_CTLCOLOR* handlers, which read
/// `dark_mode`, so flipping the flag and forcing a repaint restyles the UI.
///
/// Takes the mutex guard by value and drops it *before* redrawing: the repaint
/// synchronously pumps WM_CTLCOLOR* / WM_ERASEBKGND back into this window, and
/// those handlers lock the same (non-reentrant) mutex.
fn apply_theme(dark: bool, btn: HWND, status: HWND, hwnd: HWND) {
    DARK_MODE.store(dark, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        set_text(btn, if dark { "Light mode" } else { "Dark mode" });
        // Ask DWM for a dark title bar / border to match the client area.
        dwm_set_dark(hwnd, dark);
        set_status_text(
            status,
            if dark {
                "Dark theme on."
            } else {
                "Light theme on."
            },
        );
        // Repaints synchronously pump WM_CTLCOLOR* / WM_ERASEBKGND back into
        // this window, so nothing may hold a lock across this call.
        RedrawWindow(
            hwnd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            RDW_ERASE | RDW_FRAME | RDW_INVALIDATE | RDW_ALLCHILDREN,
        );
    }
}

fn toggle_theme() {
    let (btn, status, hwnd) = {
        let a = app().lock().unwrap();
        (a.btn_theme, a.status, a.hwnd)
    };
    apply_theme(!dark_mode(), btn, status, hwnd);
}

// ---------------- real progress ----------------

/// `  pass 2/7 : 43%`  ->  (pass index, pass count, pass percent)
fn parse_overwrite_progress(line: &str) -> Option<(u32, u32, u32)> {
    let t = line.trim();
    let t = t.strip_prefix("pass ")?;
    let (frac, rest) = t.split_once(':')?;
    let (i, n) = frac.trim().split_once('/')?;
    let i: u32 = i.trim().parse().ok()?;
    let n: u32 = n.trim().parse().ok()?;
    let pct: u32 = rest.trim().trim_end_matches('%').trim().parse().ok()?;
    Some((i, n, pct))
}

/// `[4/5] partition formatted` -> (step finished, step count)
fn parse_step(line: &str) -> Option<(u32, u32)> {
    let t = line.trim();
    let open = t.find('[')?;
    let close = t.find(']')?;
    if close <= open + 1 {
        return None;
    }
    let inner = &t[open + 1..close];
    let (k, m) = inner.split_once('/')?;
    Some((k.trim().parse().ok()?, m.trim().parse().ok()?))
}

/// The worker writes lines as `[2026-10-10 01:33:16.187][DATA]   pass 1/1 : 0%`.
/// Strip the timestamp and level prefixes so the content can be matched.
fn log_content(line: &str) -> &str {
    let mut s = line.trim();
    if !s.starts_with('[') {
        return s;
    }
    for _ in 0..2 {
        if !s.starts_with('[') {
            break;
        }
        match s.find(']') {
            Some(end) => s = s[end + 1..].trim_start(),
            None => break,
        }
    }
    s
}

/// Fold newly read log text into `progress_pct` (monotonic, 0..99).
/// Full runs track the real overwrite across every pass; quick runs fall back
/// to the coarse step lines.
fn note_progress(a: &mut App, text: &str) {
    for raw in text.lines() {
        let line = log_content(raw);
        if let Some((i, n, pct)) = parse_overwrite_progress(line) {
            a.saw_overwrite = true;
            let overall = if n > 0 {
                (((i.max(1) - 1) as u64 * 100 + pct as u64) / n as u64).min(99)
            } else {
                (pct as u64).min(99)
            };
            if overall as u32 > a.progress_pct {
                a.progress_pct = overall as u32;
            }
        } else if !a.full_run {
            if let Some((k, m)) = parse_step(line) {
                if m > 0 {
                    let v = ((k.max(1) - 1) as u64 * 100 / m as u64).min(99) as u32;
                    if v > a.progress_pct {
                        a.progress_pct = v;
                    }
                }
            }
        }
    }
}

// ---------------- log tail ----------------

fn tail_log(a: &mut App) {
    let Some(path) = a.log_path.clone() else {
        return;
    };
    let Ok(mut f) = fs::File::open(&path) else {
        return;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len < a.log_offset {
        a.log_offset = 0;
    }
    if len == a.log_offset {
        return;
    }
    if f.seek(SeekFrom::Start(a.log_offset)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return;
    }
    a.log_offset = len;

    let text = String::from_utf8_lossy(&buf);
    // Update the progress bar from the real overwrite steps.
    note_progress(a, &text);
    let norm = text.replace("\r\n", "\n").replace('\n', "\r\n");
    // Log lines belong only in the log pane. Do NOT mirror them onto the
    // status strip (it sits just below the log box and made log text appear
    // to "spill out" underneath it).
    append_edit(a.log, &norm);
}

// ---------------- actions ----------------

fn toggle_hide_fixed() {
    let mut a = app().lock().unwrap();
    a.hide_fixed = !a.hide_fixed;
    let caption = if a.hide_fixed {
        "Show fixed drives"
    } else {
        "Hide fixed drives"
    };
    let hide = a.btn_hide;
    unsafe {
        set_text(hide, caption);
    }
    drop(a);
    populate_drives();
    let a = app().lock().unwrap();
    if a.hide_fixed {
        set_status_text(a.status, "Fixed (non-removable) drives are hidden.");
    } else {
        set_status_text(a.status, "Fixed (non-removable) drives are shown.");
    }
}

fn start_wipe(hwnd: HWND) {
    let a = app().lock().unwrap();
    if a.busy {
        return;
    }
    if !checked(a.check) {
        drop(a);
        message(
            hwnd,
            "Please tick the 'I understand' box before wiping.",
            "RE-Formatter",
            MB_OK | MB_ICONWARNING,
        );
        return;
    }
    let mut buf = [0u16; 64];
    unsafe {
        GetWindowTextW(a.confirm, buf.as_mut_ptr(), buf.len() as i32);
    }
    if from_wide(&buf).trim() != "WIPE" {
        drop(a);
        message(
            hwnd,
            "Type WIPE (capital letters) in the confirmation box.",
            "RE-Formatter",
            MB_OK | MB_ICONWARNING,
        );
        return;
    }
    let sel = selected_drives(&a);
    if sel.is_empty() {
        drop(a);
        message(
            hwnd,
            "Select at least one drive in the list.",
            "RE-Formatter",
            MB_OK | MB_ICONWARNING,
        );
        return;
    }
    let fs_kind = if checked(a.radio_fat32) {
        "fat32"
    } else {
        "exfat"
    };
    let passes = if checked(a.full_wipe) {
        a.passes.max(1)
    } else {
        0
    };
    drop(a);

    // Full (multi-pass) overwrite is forbidden on solid-state drives.
    let ssd_sel: Vec<char> = sel
        .iter()
        .filter(|d| passes >= 1 && d.media.is_ssd())
        .map(|d| d.letter)
        .collect();
    if !ssd_sel.is_empty() {
        let names = ssd_sel
            .iter()
            .map(|c| format!("{}:", c))
            .collect::<Vec<_>>()
            .join(", ");
        let verb = if ssd_sel.len() == 1 {
            "is an SSD"
        } else {
            "are SSDs"
        };
        message(
            hwnd,
            &format!(
                "Full (multi-pass) wipe is BLOCKED for solid-state drives.\n\n\
                 {} {}. Overwriting an SSD does not securely erase it and only wears it out.\n\n\
                 Use quick wipe for it, or pick USB / hard drives for a full wipe.",
                names, verb
            ),
            "RE-Formatter - SSD protected",
            MB_OK | MB_ICONWARNING,
        );
        return;
    }

    if sel.iter().any(|d| d.fixed) {
        let r = unsafe {
            let t = to_wide("One of the selected drives is a FIXED disk (not removable).\n\nContinue and erase it?");
            let c = to_wide("Warning - fixed disk");
            MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONWARNING)
        };
        if r != IDYES {
            return;
        }
    }

    let letters = sel
        .iter()
        .map(|d| d.letter.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let log_path = core::default_log_path();
    let _ = fs::remove_file(&log_path);
    let mut params = format!(
        "--run --drives {} --fs {} --log \"{}\"",
        letters,
        fs_kind,
        log_path.display()
    );
    if passes >= 1 {
        params.push_str(&format!(" --passes {}", passes));
    }

    match core::spawn_worker(&params) {
        Ok(h) => {
            let mut a = app().lock().unwrap();
            a.worker = Some(h);
            a.log_path = Some(log_path);
            a.log_offset = 0;
            a.busy = true;
            a.tick = 0;
            a.paused = false;
            a.progress_pct = 0;
            a.saw_overwrite = false;
            a.full_run = passes >= 1;
            apply_busy(&a, true);
            unsafe {
                set_text(a.log, "");
                set_text(a.btn_pause, "Pause");
                SendMessageW(a.progress, PBM_SETPOS, 0, 0);
                SetTimer(a.hwnd, TIMER_ID, TIMER_MS, std::ptr::null_mut());
            }
            let start = if passes >= 1 {
                format!(
                    "Elevated worker started... {} (this can take a long time).",
                    core::pass_label(passes)
                )
            } else {
                "Elevated worker started... wiping drive(s).".to_string()
            };
            append_edit(
                a.log,
                &format!(
                    "[RE-Formatter] Launching elevated worker for [{}] as {} - {}.\r\n\r\n",
                    letters,
                    fs_kind,
                    if passes >= 1 {
                        core::pass_label(passes)
                    } else {
                        "quick wipe".to_string()
                    }
                ),
            );
            set_status_text(a.status, &start);
        }
        Err(code) => {
            let text = if code == ERROR_CANCELLED {
                "Administrator rights are required to format a drive.\n\nUAC was cancelled - nothing was changed."
            } else {
                "Could not start the elevated worker."
            };
            message(hwnd, text, "RE-Formatter", MB_OK | MB_ICONERROR);
        }
    }
}

/// Clear the one-shot wipe state so the form is ready for the next run: the
/// confirmation box, the "I understand" tick, the full-wipe arming and the
/// progress bar all go back to their idle defaults. Called once a wipe has
/// finished (successfully or not) so a stale "WIPE" / ticked box can never be
/// reused by the next click.
///
/// The control handles are null-checked so this is safe on a bare App (tests).
fn reset_for_next_wipe(a: &mut App) {
    unsafe {
        if !a.confirm.is_null() {
            SendMessageW(a.confirm, WM_CLEAR, 0, 0);
        }
        if !a.check.is_null() {
            SendMessageW(a.check, BM_SETCHECK, BST_UNCHECKED as usize, 0);
        }
        if !a.full_wipe.is_null() {
            SendMessageW(a.full_wipe, BM_SETCHECK, BST_UNCHECKED as usize, 0);
            set_text(a.full_wipe, FULL_WIPE_OFF);
        }
        if !a.progress.is_null() {
            SendMessageW(a.progress, PBM_SETPOS, 0, 0);
        }
    }
    a.passes = 0;
    a.paused = false;
    a.busy = false;
    a.progress_pct = 0;
    a.saw_overwrite = false;
    a.full_run = false;
    a.log_path = None;
    a.log_offset = 0;
}

fn on_timer(hwnd: HWND) {
    let mut a = app().lock().unwrap();
    if !a.busy {
        return;
    }
    a.tick = a.tick.wrapping_add(1);
    tail_log(&mut a);
    // The bar now tracks real progress (no more fake looping).
    unsafe {
        SendMessageW(a.progress, PBM_SETPOS, a.progress_pct as usize, 0);
    }

    let worker = a.worker;
    if let Some(h) = worker {
        let r = unsafe { WaitForSingleObject(h, 0) };
        if r == WAIT_OBJECT_0 {
            let mut code: u32 = 0;
            unsafe {
                GetExitCodeProcess(h, &mut code);
                CloseHandle(h);
            }
            a.worker = None;
            tail_log(&mut a);
            a.busy = false;
            a.paused = false;
            // Leave a full run showing 100% when it succeeded.
            if code == 0 {
                a.progress_pct = 100;
            }
            unsafe {
                KillTimer(hwnd, TIMER_ID);
                set_text(a.btn_pause, "Pause");
                SendMessageW(a.progress, PBM_SETPOS, a.progress_pct as usize, 0);
            }
            if let Some(lp) = a.log_path.clone() {
                let _ = fs::remove_file(core::pause_flag_path_for(&lp));
            }
            apply_busy(&a, false);
            core::refresh_explorer();
            reset_for_next_wipe(&mut a);

            let (msg, flags) = if code == 0 {
                (
                    "Done - drive(s) reformatted successfully.",
                    MB_OK | MB_ICONINFORMATION,
                )
            } else {
                (
                    "Finished, but at least one drive failed. See the log.",
                    MB_OK | MB_ICONWARNING,
                )
            };
            append_edit(
                a.log,
                &format!(
                    "\r\n[RE-Formatter] Worker finished (exit code {}).\r\n",
                    code
                ),
            );
            set_status_text(a.status, msg);
            drop(a);
            // Re-read volumes only once the lock is free: the new partition may
            // have taken a different letter or size than the pre-wipe one.
            populate_drives();
            message(hwnd, msg, "RE-Formatter", flags);
        }
    }
}

// ---------------- pass-selection dialog ----------------

fn create_pass_controls(hwnd: HWND) {
    let (font, sel) = {
        let d = pass_dlg().lock().unwrap();
        (d.font, d.selected)
    };
    let base = WS_CHILD | WS_VISIBLE;
    let mut radios = [std::ptr::null_mut(); 4];
    unsafe {
        let lbl = child(
            hwnd,
            "STATIC",
            "Choose the overwrite method. More passes = more secure, but much slower and more wear on the drive:",
            base,
            0,
            0,
            14,
            12,
            344,
            30,
        );
        let opts = [
            (PID_R1, "1 pass   - quick full wipe (write zeros once)"),
            (PID_R3, "3 passes - DoD 5220.22-M  (US Dept. of Defense)"),
            (PID_R7, "7 passes - DoD 5220.22-M ECE"),
            (PID_R35, "35 passes - Gutmann method (slowest)"),
        ];
        for (i, (id, text)) in opts.iter().enumerate() {
            let extra = if i == 0 { WS_GROUP } else { 0 };
            radios[i] = child(
                hwnd,
                "BUTTON",
                text,
                base | BS_AUTORADIOBUTTON | WS_TABSTOP | extra,
                0,
                *id,
                20,
                52 + (i as i32) * 24,
                336,
                20,
            );
            set_font(radios[i], font);
        }
        let note = child(
            hwnd,
            "STATIC",
            "Every sector is overwritten before the drive is re-formatted.",
            base,
            0,
            0,
            14,
            154,
            344,
            16,
        );
        let ok = child(
            hwnd,
            "BUTTON",
            "OK",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            PID_OK,
            196,
            180,
            78,
            26,
        );
        let cancel = child(
            hwnd,
            "BUTTON",
            "Cancel",
            base | BS_PUSHBUTTON | WS_TABSTOP,
            0,
            PID_CANCEL,
            280,
            180,
            78,
            26,
        );
        set_font(lbl, font);
        set_font(note, font);
        set_font(ok, font);
        set_font(cancel, font);

        let idx = match sel {
            3 => 1,
            7 => 2,
            35 => 3,
            _ => 0,
        };
        SendMessageW(radios[idx], BM_SETCHECK, BST_CHECKED as usize, 0);
    }
    let mut d = pass_dlg().lock().unwrap();
    d.radios = radios;
    d.hwnd = hwnd;
}

fn read_selected_pass() -> u32 {
    let d = pass_dlg().lock().unwrap();
    for (i, h) in d.radios.iter().enumerate() {
        if checked(*h) {
            return [1u32, 3, 7, 35][i];
        }
    }
    1
}

fn close_pass_dialog(hwnd: HWND, result: Option<u32>) {
    {
        let mut d = pass_dlg().lock().unwrap();
        d.result = result;
        d.done = true;
    }
    unsafe {
        DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn pass_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            create_pass_controls(hwnd);
            0
        }
        WM_COMMAND => {
            let id = loword(wparam) as i32;
            let code = ((wparam >> 16) & 0xFFFF) as u32;
            if code == BN_CLICKED {
                match id {
                    PID_OK => close_pass_dialog(hwnd, Some(read_selected_pass())),
                    PID_CANCEL => close_pass_dialog(hwnd, None),
                    _ => {}
                }
            }
            0
        }
        WM_CLOSE => {
            close_pass_dialog(hwnd, None);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORDLG | WM_CTLCOLOREDIT
        | WM_CTLCOLORLISTBOX => {
            let dark = dark_mode();
            let hdc = wparam as HDC;
            if dark {
                let mut cls = [0u16; 32];
                unsafe { GetClassNameW(lparam as HWND, cls.as_mut_ptr(), cls.len() as i32) };
                let name = from_wide(&cls);
                let is_surface =
                    !name.eq_ignore_ascii_case("Edit") && !name.eq_ignore_ascii_case("ListBox");
                unsafe {
                    if is_surface {
                        SetBkMode(hdc, TRANSPARENT);
                    } else {
                        SetBkMode(hdc, OPAQUE);
                        SetBkColor(hdc, CLR_DARK_EDIT_BG);
                    }
                    SetTextColor(hdc, CLR_DARK_TEXT);
                }
                let brush = if is_surface {
                    dark_face_brush()
                } else {
                    dark_surface_brush()
                };
                brush as LRESULT
            } else {
                unsafe {
                    SetBkMode(hdc, TRANSPARENT);
                    GetSysColorBrush(COLOR_3DFACE) as LRESULT
                }
            }
        }
        WM_ERASEBKGND => {
            let hdc = wparam as HDC;
            let mut rc = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe {
                GetClientRect(hwnd, &mut rc);
            }
            if dark_mode() {
                FillRect(hdc, &rc, dark_face_brush());
            } else {
                FillRect(hdc, &rc, GetSysColorBrush(COLOR_3DFACE));
            }
            1
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Show the modal pass chooser. Returns the chosen pass count, or None if the
/// user cancelled.
fn show_pass_dialog(parent: HWND, current: u32) -> Option<u32> {
    let font = app().lock().unwrap().font;
    {
        let mut d = pass_dlg().lock().unwrap();
        d.done = false;
        d.result = None;
        d.selected = current;
        d.font = font;
    }

    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 372,
        bottom: 224,
    };
    unsafe {
        AdjustWindowRectEx(&mut rect, style, 0, 0);
    }
    let win_w = rect.right - rect.left;
    let win_h = rect.bottom - rect.top;

    let mut pr = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetWindowRect(parent, &mut pr);
    }
    let x = pr.left + ((pr.right - pr.left) - win_w) / 2;
    let y = pr.top + ((pr.bottom - pr.top) - win_h) / 2;

    let class = to_wide("PassDialogW98");
    let title = to_wide("Full wipe options");
    let hinst = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            win_w,
            win_h,
            parent,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        return None;
    }

    unsafe {
        EnableWindow(parent, 0);
        // Match the main window's theme (dark title bar when it is dark).
        dwm_set_dark(hwnd, dark_mode());
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        SetForegroundWindow(hwnd);
    }

    // Nested modal message loop: keeps dispatching until the dialog closes.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if r <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if pass_dlg().lock().map(|d| d.done).unwrap_or(true) {
            break;
        }
    }

    unsafe {
        EnableWindow(parent, 1);
        SetForegroundWindow(parent);
    }
    pass_dlg().lock().unwrap().result
}

/// Called when the "Full wipe" checkbox is clicked: pops the pass chooser, or
/// resets to quick mode when it is unticked.
fn on_toggle_full(hwnd: HWND) {
    let want = {
        let a = app().lock().unwrap();
        checked(a.full_wipe)
    };
    if want {
        // Immediate feedback: do not let full mode be armed while an SSD is selected.
        let ssd_sel: Vec<char> = {
            let a = app().lock().unwrap();
            selected_drives(&a)
                .iter()
                .filter(|d| d.media.is_ssd())
                .map(|d| d.letter)
                .collect()
        };
        if !ssd_sel.is_empty() {
            let names = ssd_sel
                .iter()
                .map(|c| format!("{}:", c))
                .collect::<Vec<_>>()
                .join(", ");
            {
                let mut a = app().lock().unwrap();
                a.passes = 0;
                unsafe {
                    SendMessageW(a.full_wipe, BM_SETCHECK, BST_UNCHECKED as usize, 0);
                    set_text(a.full_wipe, FULL_WIPE_OFF);
                }
                set_status_text(a.status, "Full wipe is not allowed on SSD drives.");
            }
            message(
                hwnd,
                &format!(
                    "Full (multi-pass) wipe is BLOCKED for solid-state drives.\n\n\
                     {} {}. Overwriting an SSD does not securely erase it and only wears it out.\n\n\
                     Use quick wipe for it, or pick USB / hard drives for a full wipe.",
                    names,
                    if ssd_sel.len() == 1 { "is an SSD" } else { "are SSDs" }
                ),
                "RE-Formatter - SSD protected",
                MB_OK | MB_ICONWARNING,
            );
            return;
        }
        let current = {
            let a = app().lock().unwrap();
            if a.passes >= 1 {
                a.passes
            } else {
                1
            }
        };
        match show_pass_dialog(hwnd, current) {
            Some(p) => {
                let mut a = app().lock().unwrap();
                a.passes = p;
                unsafe {
                    set_text(
                        a.full_wipe,
                        &format!(
                            "Full wipe ON: {} pass(es) [{}] - click to change.",
                            p,
                            core::passes_short(p)
                        ),
                    );
                }
                set_status_text(a.status, &format!("{} selected.", core::pass_label(p)));
            }
            None => {
                let mut a = app().lock().unwrap();
                a.passes = 0;
                unsafe {
                    SendMessageW(a.full_wipe, BM_SETCHECK, BST_UNCHECKED as usize, 0);
                    set_text(a.full_wipe, FULL_WIPE_OFF);
                }
                set_status_text(a.status, "Full wipe cancelled; quick wipe selected.");
            }
        }
    } else {
        let mut a = app().lock().unwrap();
        a.passes = 0;
        unsafe {
            set_text(a.full_wipe, FULL_WIPE_OFF);
        }
        set_status_text(a.status, "Quick wipe selected (no overwrite).");
    }
}

// ---------------- window proc ----------------

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            create_controls(hwnd);
            // Start with the log pane hidden (compact window).
            set_log_visible(false);
            0
        }
        WM_COMMAND => {
            let id = loword(wparam) as i32;
            let code = ((wparam >> 16) & 0xFFFF) as u32;
            if code == BN_CLICKED {
                // While a wipe is running the data-entry controls stay enabled
                // (see apply_busy) so their text keeps the themed colours, so
                // the actions that would change or re-trigger a wipe are
                // blocked here instead.
                let running = app().lock().unwrap().busy;
                match id {
                    ID_RESCAN => {
                        if running {
                            return 0;
                        }
                        populate_drives();
                        set_status_text(app().lock().unwrap().status, "Drive list refreshed.");
                    }
                    ID_HIDE_FIXED => {
                        if running {
                            return 0;
                        }
                        toggle_hide_fixed()
                    }
                    ID_FULL => {
                        if running {
                            return 0;
                        }
                        on_toggle_full(hwnd)
                    }
                    ID_WIPE => {
                        if running {
                            return 0;
                        }
                        start_wipe(hwnd)
                    }
                    ID_PAUSE => toggle_pause(),
                    ID_CANCEL => {
                        let running = app().lock().unwrap().busy;
                        if running {
                            let r = unsafe {
                                let t = to_wide(
                                    "Stop the wipe?\n\nThe drive will be left blank / partially overwritten.",
                                );
                                let c = to_wide("RE-Formatter");
                                MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONWARNING)
                            };
                            if r == IDYES {
                                stop_worker();
                            }
                        }
                    }
                    ID_EXIT => unsafe {
                        SendMessageW(hwnd, WM_CLOSE, 0, 0);
                    },
                    ID_LOG_TOGGLE => toggle_log(),
                    ID_THEME => toggle_theme(),
                    ID_MUSIC => toggle_music(),
                    _ => {}
                }
            }
            0
        }
        WM_PAINT => {
            // Paint the background first, then overlay the group frames.
            let dp = DefWindowProcW(hwnd, msg, wparam, lparam);
            let dark = dark_mode();
            let hdc = GetDC(hwnd);
            paint_group(
                hdc,
                &RECT {
                    left: GROUP_X,
                    top: GROUP_DRIVES_Y,
                    right: GROUP_X + GROUP_W,
                    bottom: GROUP_DRIVES_BOTTOM,
                },
                &RECT {
                    left: 11,
                    top: GROUP_DRIVES_Y - 1,
                    right: 354,
                    bottom: GROUP_DRIVES_Y + 15,
                },
                dark,
            );
            paint_group(
                hdc,
                &RECT {
                    left: GROUP_X,
                    top: GROUP_FORMAT_Y,
                    right: GROUP_X + GROUP_W,
                    bottom: GROUP_FORMAT_BOTTOM,
                },
                &RECT {
                    left: 11,
                    top: GROUP_FORMAT_Y - 1,
                    right: 174,
                    bottom: GROUP_FORMAT_Y + 15,
                },
                dark,
            );
            ReleaseDC(hwnd, hdc);
            dp
        }
        WM_TIMER => {
            on_timer(hwnd);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN
        | WM_CTLCOLORDLG => {
            let dark = dark_mode();
            let hdc = wparam as HDC;
            let child = lparam as HWND;
            // A read-only edit control reports as WM_CTLCOLORSTATIC (not
            // WM_CTLCOLOREDIT). Any edit / list control must get a real,
            // opaque background, otherwise it shows the unpainted (black)
            // parent area when it repaints on scroll. Static labels stay
            // transparent so they blend into the 3D-face window.
            let mut cls = [0u16; 32];
            unsafe { GetClassNameW(child, cls.as_mut_ptr(), cls.len() as i32) };
            let name = from_wide(&cls);
            let is_surface =
                !name.eq_ignore_ascii_case("Edit") && !name.eq_ignore_ascii_case("ListBox");
            let transparent = (msg == WM_CTLCOLORSTATIC || msg == WM_CTLCOLORDLG) && is_surface;
            unsafe {
                if dark {
                    if transparent {
                        SetBkMode(hdc, TRANSPARENT);
                        if dark {
                            SetTextColor(hdc, CLR_DARK_TEXT);
                            dark_face_brush() as LRESULT
                        } else {
                            GetSysColorBrush(COLOR_3DFACE) as LRESULT
                        }
                    } else {
                        SetBkMode(hdc, OPAQUE);
                        SetTextColor(hdc, CLR_DARK_TEXT);
                        SetBkColor(hdc, CLR_DARK_EDIT_BG);
                        dark_surface_brush() as LRESULT
                    }
                } else {
                    if transparent {
                        SetBkMode(hdc, TRANSPARENT);
                        GetSysColorBrush(COLOR_3DFACE) as LRESULT
                    } else {
                        SetBkMode(hdc, OPAQUE);
                        SetTextColor(hdc, 0x0000_0000);
                        SetBkColor(hdc, 0x00FF_FFFF);
                        GetStockObject(WHITE_BRUSH) as LRESULT
                    }
                }
            }
        }
        WM_ERASEBKGND => {
            let dark = dark_mode();
            let hdc = wparam as HDC;
            let mut rc = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe {
                GetClientRect(hwnd, &mut rc);
            }
            if dark {
                FillRect(hdc, &rc, dark_face_brush());
                1
            } else {
                // Paint the true Win98 face colour instead of the class brush.
                FillRect(hdc, &rc, GetSysColorBrush(COLOR_3DFACE));
                1
            }
        }
        WM_CLOSE => {
            let busy = app().lock().unwrap().busy;
            if busy {
                let r = unsafe {
                    let t = to_wide("A wipe is still running.\n\nStop it and close the window?");
                    let c = to_wide("RE-Formatter");
                    MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONWARNING)
                };
                if r != IDYES {
                    return 0;
                }
                stop_worker();
            }
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------- entry ----------------

// Force classic (un-themed) common controls for the Win98 look. uxtheme is
// loaded dynamically so we do not need to link against it at build time.
fn disable_themes() {
    let dll = to_wide("uxtheme.dll");
    unsafe {
        let h = LoadLibraryW(dll.as_ptr());
        if h.is_null() {
            return;
        }
        let name = b"SetThemeAppProperties\0";
        let p = GetProcAddress(h, name.as_ptr());
        if !p.is_null() {
            let f: unsafe extern "system" fn(u32) = std::mem::transmute(p);
            f(0);
        }
    }
}

pub fn run() {
    unsafe {
        CoInitializeEx(std::ptr::null(), core::COINIT_APARTMENTTHREADED);
        disable_themes();
        let icc = InitCommonControlsEx {
            dw_size: std::mem::size_of::<InitCommonControlsEx>() as u32,
            dw_icc: ICC_PROGRESS_CLASS,
        };
        InitCommonControlsEx(&icc);
    }

    let (big_icon, small_icon) = load_icons();
    let hinst = unsafe { GetModuleHandleW(std::ptr::null()) };
    let cursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16) };

    let class_name = to_wide("UsbReformatW98");
    let wc = WNDCLASSEXW {
        cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfn_wnd_proc: Some(wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: hinst,
        h_icon: big_icon,
        h_cursor: cursor,
        hbr_background: (COLOR_3DFACE + 1) as HBRUSH,
        lpsz_menu_name: std::ptr::null(),
        lpsz_class_name: class_name.as_ptr(),
        h_icon_sm: small_icon,
    };
    unsafe {
        RegisterClassExW(&wc);
    }

    // The pass chooser is a second window class with its own message handler.
    let dlg_class_name = to_wide("PassDialogW98");
    let dwc = WNDCLASSEXW {
        cb_size: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfn_wnd_proc: Some(pass_wnd_proc),
        cb_cls_extra: 0,
        cb_wnd_extra: 0,
        h_instance: hinst,
        h_icon: big_icon,
        h_cursor: cursor,
        hbr_background: (COLOR_3DFACE + 1) as HBRUSH,
        lpsz_menu_name: std::ptr::null(),
        lpsz_class_name: dlg_class_name.as_ptr(),
        h_icon_sm: small_icon,
    };
    unsafe {
        RegisterClassExW(&dwc);
    }

    let _ = APP.set(Mutex::new(App::new()));
    let _ = PASS_DLG.set(Mutex::new(PassDlg::new()));

    let style = MAIN_STYLE;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: CLIENT_W,
        bottom: CLIENT_H,
    };
    unsafe {
        AdjustWindowRectEx(&mut rect, style, 0, MAIN_EXSTYLE);
    }
    let win_w = rect.right - rect.left;
    let win_h = rect.bottom - rect.top;
    let x = (unsafe { GetSystemMetrics(SM_CXSCREEN) } - win_w) / 2;
    let y = (unsafe { GetSystemMetrics(SM_CYSCREEN) } - win_h) / 2;

    let title = to_wide("RE-Formatter");
    let hwnd = unsafe {
        CreateWindowExW(
            MAIN_EXSTYLE,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            win_w,
            win_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        message(
            std::ptr::null_mut(),
            "Could not create the window.",
            "Error",
            MB_OK | MB_ICONERROR,
        );
        return;
    }

    unsafe {
        if !big_icon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, big_icon as isize);
        }
        if !small_icon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, small_icon as isize);
        }
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        SetForegroundWindow(hwnd);
    }

    // Start the chiptunes now so music is playing the moment the UI appears,
    // rather than waiting for the Pause button to be pressed.
    let _ = music();

    populate_drives();
    {
        let a = app().lock().unwrap();
        set_status_text(
            a.status,
            "Ready. Select a drive, tick the box, type WIPE, then click WIPE DRIVE(S).",
        );
        append_edit(
            a.log,
            "[RE-Formatter] Ready.\r\nSelect a drive, tick 'I understand', type WIPE, then click WIPE DRIVE(S).\r\n\r\n",
        );
    }

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if r <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---------------- tests ----------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_worker_log_prefix() {
        assert_eq!(
            log_content("[2026-10-10 01:33:16.187][DATA]   pass 1/1 : 0%"),
            "pass 1/1 : 0%"
        );
        assert_eq!(
            log_content("[2026-10-10 01:33:10.718][STEP] G: [1/5] files wiped"),
            "G: [1/5] files wiped"
        );
    }

    #[test]
    fn parses_overwrite_progress_lines() {
        assert_eq!(parse_overwrite_progress("pass 1/1 : 0%"), Some((1, 1, 0)));
        assert_eq!(parse_overwrite_progress("pass 3/7 : 43%"), Some((3, 7, 43)));
        assert_eq!(
            parse_overwrite_progress("pass 7/35 : 100%"),
            Some((7, 35, 100))
        );
        // Not a progress line.
        assert_eq!(
            parse_overwrite_progress("overwrite pass 1/1 over 64.16 GB ..."),
            None
        );
    }

    #[test]
    fn parses_step_lines() {
        assert_eq!(parse_step("G: [1/5] files wiped"), Some((1, 5)));
        assert_eq!(
            parse_step("G: [3/5] disk overwritten (full wipe)"),
            Some((3, 5))
        );
        assert_eq!(parse_step("pass 1/1 : 0%"), None);
    }

    #[test]
    fn full_run_follows_overwrite_progress_and_never_goes_backwards() {
        let mut a = App::new();
        a.full_run = true;

        note_progress(
            &mut a,
            "[2026-10-10 01:33:16.187][DATA]   pass 1/7 : 0%\r\n",
        );
        assert_eq!(a.progress_pct, 0);

        note_progress(
            &mut a,
            "[2026-10-10 01:40:00.000][DATA]   pass 4/7 : 50%\r\n",
        );
        assert_eq!(a.progress_pct, 50); // ((3*100) + 50) / 7 = 50

        // A later but "earlier" reading must not rewind the bar.
        note_progress(
            &mut a,
            "[2026-10-10 01:41:00.000][DATA]   pass 3/7 : 40%\r\n",
        );
        assert_eq!(a.progress_pct, 50);

        // Step lines are ignored during a full run (they would rewind it).
        note_progress(
            &mut a,
            "[2026-10-10 01:42:00.000][STEP] G: [1/5] files wiped\r\n",
        );
        assert_eq!(a.progress_pct, 50);

        // Final pass tops out just under 100; the finish handler sets 100.
        note_progress(
            &mut a,
            "[2026-10-10 02:00:00.000][DATA]   pass 7/7 : 100%\r\n",
        );
        assert_eq!(a.progress_pct, 99);
    }

    #[test]
    fn reset_clears_all_one_shot_wipe_state() {
        // A finished wipe must leave nothing armed for the next click.
        let mut a = App::new();
        a.passes = 35;
        a.paused = true;
        a.progress_pct = 100;
        a.saw_overwrite = true;
        a.full_run = true;
        a.log_path = Some(PathBuf::from("C:/tmp/usb-reformat_20261010_013309.log"));
        a.log_offset = 4096;
        a.busy = true;

        reset_for_next_wipe(&mut a);

        assert_eq!(a.passes, 0);
        assert!(!a.paused);
        assert_eq!(a.progress_pct, 0);
        assert!(!a.saw_overwrite);
        assert!(!a.full_run);
        assert!(a.log_path.is_none());
        assert_eq!(a.log_offset, 0);
        // The wipe is over: busy must be cleared too.
        assert!(!a.busy);
    }

    #[test]
    fn quick_run_falls_back_to_step_lines() {
        let mut a = App::new();
        a.full_run = false;
        note_progress(
            &mut a,
            "[2026-10-10 01:33:10.718][STEP] G: [1/3] files wiped\r\n",
        );
        assert_eq!(a.progress_pct, 0);
        note_progress(
            &mut a,
            "[2026-10-10 01:33:13.000][STEP] G: [2/3] partition rebuilt\r\n",
        );
        assert_eq!(a.progress_pct, 33);
        note_progress(
            &mut a,
            "[2026-10-10 01:33:16.000][STEP] G: [3/3] verifying\r\n",
        );
        assert_eq!(a.progress_pct, 66);
    }
}
