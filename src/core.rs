// Domain logic: drive discovery, diskpart rebuild, logging, the elevated
// worker, and the legacy interactive console flow.
#![allow(dead_code)]

use std::ffi::c_void;
use std::fs;
use std::io::{self, BufRead, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::win32::*;

// ---------------- Win32 structs ----------------

#[repr(C)]
pub struct StorageDeviceNumber {
    pub device_type: DWORD,
    pub device_number: DWORD,
    pub partition_number: DWORD,
}

#[repr(C)]
struct GetLengthInformation {
    length: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DiskExtent {
    disk_number: DWORD,
    starting_offset: i64,
    extent_length: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VolumeDiskExtents {
    number_of_disk_extents: DWORD,
    extents: [DiskExtent; 16],
}

#[repr(C)]
struct StoragePropertyQuery {
    property_id: DWORD,
    query_type: DWORD,
    additional: [u8; 1],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DeviceSeekPenaltyDescriptor {
    version: DWORD,
    size: DWORD,
    incurs_seek_penalty: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WinSystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

#[repr(C)]
struct TokenElevation {
    token_is_elevated: DWORD,
}

#[repr(C)]
pub struct ShellExecuteInfoW {
    pub cb_size: u32,
    pub f_mask: u32,
    pub hwnd: HANDLE,
    pub lp_verb: *const u16,
    pub lp_file: *const u16,
    pub lp_parameters: *const u16,
    pub lp_directory: *const u16,
    pub n_show: i32,
    pub h_inst_app: HANDLE,
    pub lp_id_list: *mut c_void,
    pub lp_class: *const u16,
    pub hkey_class: HANDLE,
    pub dw_hot_key: u32,
    pub h_icon: HANDLE,
    pub h_process: HANDLE,
}

// ---------------- constants ----------------

pub const GENERIC_READ: DWORD = 0x8000_0000;
pub const GENERIC_WRITE: DWORD = 0x4000_0000;
pub const FILE_SHARE_READ: DWORD = 0x0000_0001;
pub const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
pub const OPEN_EXISTING: DWORD = 3;
pub const IOCTL_STORAGE_GET_DEVICE_NUMBER: DWORD = 0x002D_1080;
pub const IOCTL_DISK_GET_LENGTH_INFO: DWORD = 0x0007_405C;
pub const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: DWORD = 0x0056_0000;
pub const FSCTL_LOCK_VOLUME: DWORD = 0x0009_0018;
pub const FSCTL_DISMOUNT_VOLUME: DWORD = 0x0009_0020;
pub const FILE_FLAG_NO_BUFFERING: DWORD = 0x2000_0000;
pub const IOCTL_STORAGE_QUERY_PROPERTY: DWORD = 0x002D_1400;
pub const STORAGE_DEVICE_PROPERTY: DWORD = 0;
pub const STORAGE_DEVICE_SEEK_PENALTY_PROPERTY: DWORD = 7;
pub const PROPERTY_STANDARD_QUERY: DWORD = 0;
pub const BUS_TYPE_USB: DWORD = 7;

pub const STD_INPUT_HANDLE: DWORD = 0xFFFF_FFF6;
pub const STD_OUTPUT_HANDLE: DWORD = 0xFFFF_FFF5;
pub const ATTACH_PARENT_PROCESS: DWORD = 0xFFFF_FFFF;

pub const DRIVE_UNKNOWN: u32 = 0;
pub const DRIVE_NO_ROOT_DIR: u32 = 1;
pub const DRIVE_REMOVABLE: u32 = 2;
pub const DRIVE_FIXED: u32 = 3;
pub const DRIVE_REMOTE: u32 = 4;
pub const DRIVE_CDROM: u32 = 5;
pub const DRIVE_RAMDISK: u32 = 6;

pub const INVALID_PARTITION: DWORD = 0xFFFF_FFFF;

pub const TOKEN_QUERY: DWORD = 0x0008;
pub const TOKEN_ELEVATION_CLASS: i32 = 20;

pub const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
pub const SW_SHOWNORMAL: i32 = 1;
pub const INFINITE: DWORD = 0xFFFF_FFFF;

pub const SHCNE_MEDIAINSERTED: i32 = 0x0000_0020;
pub const SHCNE_DRIVEADD: i32 = 0x0000_0100;
pub const SHCNE_UPDATEDIR: i32 = 0x0000_1000;
pub const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
pub const SHCNF_PATHW: u32 = 0x0005;

pub const COINIT_APARTMENTTHREADED: DWORD = 0x2;

// ---------------- globals ----------------

static LOGGER: OnceLock<Mutex<Option<fs::File>>> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static CONSOLE: OnceLock<Mutex<Option<fs::File>>> = OnceLock::new();
static INPUT: OnceLock<Mutex<Option<io::BufReader<fs::File>>>> = OnceLock::new();

static WINDOWED: AtomicBool = AtomicBool::new(false);
static SPINNING: AtomicBool = AtomicBool::new(false);

// ---------------- time helpers ----------------

pub fn local_time() -> WinSystemTime {
    let mut st = WinSystemTime::default();
    unsafe {
        GetLocalTime(&mut st);
    }
    st
}

pub fn now_string() -> String {
    let st = local_time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.year, st.month, st.day, st.hour, st.minute, st.second, st.milliseconds
    )
}

pub fn file_stamp() -> String {
    let st = local_time();
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        st.year, st.month, st.day, st.hour, st.minute, st.second
    )
}

// ---------------- console layer ----------------

fn open_con(name: &str) -> Option<fs::File> {
    let wide = to_wide(name);
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if raw_valid(h) {
        Some(unsafe { fs::File::from_raw_handle(h as RawHandle) })
    } else {
        None
    }
}

pub fn init_io() {
    let inherited_console = unsafe { !GetConsoleWindow().is_null() };

    let std_out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let std_in = unsafe { GetStdHandle(STD_INPUT_HANDLE) };

    let have_out = raw_valid(std_out);
    let have_in = raw_valid(std_in);

    if !inherited_console && (!have_out || !have_in) {
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
        if attached == 0 {
            unsafe {
                if AllocConsole() != 0 {
                    WINDOWED.store(true, Ordering::SeqCst);
                    let _ = SetConsoleOutputCP(65001);
                }
            }
        }
    }

    let out_file = if have_out {
        Some(unsafe { fs::File::from_raw_handle(std_out as RawHandle) })
    } else {
        open_con("CONOUT$")
    };
    let _ = CONSOLE.set(Mutex::new(out_file));

    let in_file = if have_in {
        Some(unsafe { fs::File::from_raw_handle(std_in as RawHandle) })
    } else {
        open_con("CONIN$")
    };
    let _ = INPUT.set(Mutex::new(in_file.map(io::BufReader::new)));
}

pub fn say(msg: &str) {
    if let Some(m) = CONSOLE.get() {
        if let Ok(mut guard) = m.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.write_all(msg.as_bytes());
                let _ = f.flush();
            }
        }
    }
}

pub fn sayln(msg: &str) {
    say(msg);
    say("\r\n");
}

pub fn log(level: &str, msg: &str) {
    let line = format!("[{}][{}] {}", now_string(), level, msg);
    sayln(&line);
    if let Some(m) = LOGGER.get() {
        if let Ok(mut guard) = m.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = writeln!(f, "{}", line);
                let _ = f.flush();
            }
        }
    }
}

fn read_line() -> String {
    let mut s = String::new();
    if let Some(m) = INPUT.get() {
        if let Ok(mut guard) = m.lock() {
            if let Some(r) = guard.as_mut() {
                let _ = r.read_line(&mut s);
            }
        }
    }
    s.trim().to_string()
}

fn prompt(msg: &str) -> String {
    say(msg);
    read_line()
}

fn pause_if_windowed() {
    if WINDOWED.load(Ordering::SeqCst) {
        sayln("");
        say("Press Enter to close...");
        read_line();
    }
}

// ---------------- animated progress ----------------

fn spinner_start(label: &str) {
    if SPINNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let label = label.to_string();
    std::thread::spawn(move || {
        let frames = ["|", "/", "-", "\\"];
        let mut i = 0usize;
        while SPINNING.load(Ordering::SeqCst) {
            say(&format!("\r   {} {}...", frames[i % frames.len()], label));
            i += 1;
            std::thread::sleep(Duration::from_millis(110));
        }
        say("\r                                                        \r");
    });
}

fn spinner_stop() {
    SPINNING.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(160));
}

// ---------------- Win32 wrappers ----------------

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev = TokenElevation {
            token_is_elevated: 0,
        };
        let mut ret: DWORD = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION_CLASS,
            &mut elev as *mut _ as *mut c_void,
            std::mem::size_of::<TokenElevation>() as DWORD,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elev.token_is_elevated != 0
    }
}

pub fn logical_drives() -> Vec<char> {
    let mut v = Vec::new();
    unsafe {
        let mask = GetLogicalDrives();
        for i in 0..26u32 {
            if mask & (1 << i) != 0 {
                v.push((b'A' + i as u8) as char);
            }
        }
    }
    v
}

pub fn open_volume(letter: char) -> Option<HANDLE> {
    let path = format!("\\\\.\\{}:", letter.to_ascii_uppercase());
    let wide = to_wide(&path);
    unsafe {
        let mut h = CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if !raw_valid(h) {
            h = CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
        }
        if raw_valid(h) {
            Some(h)
        } else {
            None
        }
    }
}

pub fn device_number(letter: char) -> Option<(u32, u32)> {
    let h = open_volume(letter)?;
    unsafe {
        let mut info = StorageDeviceNumber {
            device_type: 0,
            device_number: 0,
            partition_number: 0,
        };
        let mut bytes: DWORD = 0;
        let ok = DeviceIoControl(
            h,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            std::ptr::null_mut(),
            0,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<StorageDeviceNumber>() as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        );
        CloseHandle(h);
        if ok != 0 && info.partition_number != INVALID_PARTITION {
            Some((info.device_number, info.partition_number))
        } else {
            None
        }
    }
}

pub fn drive_type(letter: char) -> u32 {
    let root = to_wide(&format!("{}:\\", letter.to_ascii_uppercase()));
    unsafe { GetDriveTypeW(root.as_ptr()) }
}

pub fn drive_type_name(t: u32) -> &'static str {
    match t {
        DRIVE_REMOVABLE => "removable",
        DRIVE_FIXED => "fixed",
        DRIVE_REMOTE => "network",
        DRIVE_CDROM => "cd-rom",
        DRIVE_RAMDISK => "ramdisk",
        DRIVE_UNKNOWN => "unknown",
        DRIVE_NO_ROOT_DIR => "none",
        _ => "none",
    }
}

// ---------------- media detection (USB / HDD / SSD) ----------------

/// What kind of physical media a drive is. Used to keep multi-pass overwrites
/// away from solid-state drives, which they do not securely erase and only wear
/// out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Usb,
    Hdd,
    Ssd,
    Unknown,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            MediaKind::Usb => "USB",
            MediaKind::Hdd => "HDD",
            MediaKind::Ssd => "SSD",
            MediaKind::Unknown => "disk",
        }
    }
    pub fn is_ssd(self) -> bool {
        self == MediaKind::Ssd
    }
}

fn query_bus_type(h: HANDLE) -> DWORD {
    let mut q = StoragePropertyQuery {
        property_id: STORAGE_DEVICE_PROPERTY,
        query_type: PROPERTY_STANDARD_QUERY,
        additional: [0],
    };
    let mut buf = [0u8; 1024];
    let mut bytes: DWORD = 0;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut q as *mut _ as *mut c_void,
            std::mem::size_of::<StoragePropertyQuery>() as DWORD,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        )
    };
    // STORAGE_DEVICE_DESCRIPTOR.BusType lives at byte offset 28.
    if ok == 0 || bytes < 32 {
        return 0;
    }
    DWORD::from_le_bytes([buf[28], buf[29], buf[30], buf[31]])
}

/// Read the device's marketing/model name (e.g. "Samsung Flash Drive FIT")
/// out of STORAGE_DEVICE_DESCRIPTOR. VendorId sits at byte offset 12 and
/// ProductId at 16; both are null-terminated ASCII inside the returned buffer.
/// This is how you tell two identical-looking sticks apart when neither has a
/// volume label.
fn query_device_name(h: HANDLE) -> Option<String> {
    let mut q = StoragePropertyQuery {
        property_id: STORAGE_DEVICE_PROPERTY,
        query_type: PROPERTY_STANDARD_QUERY,
        additional: [0],
    };
    let mut buf = [0u8; 1024];
    let mut bytes: DWORD = 0;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut q as *mut _ as *mut c_void,
            std::mem::size_of::<StoragePropertyQuery>() as DWORD,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        )
    };
    let len = bytes as usize;
    // Need at least through ProductIdOffset (16..20).
    if ok == 0 || len < 20 || len > buf.len() {
        return None;
    }
    let read_str = |off: usize| -> String {
        if off == 0 || off >= len {
            return String::new();
        }
        let mut s = String::new();
        let mut i = off;
        while i < len && buf[i] != 0 {
            // Some devices put control characters in VendorId; drop anything
            // that is not printable ASCII.
            if buf[i] >= 0x20 && buf[i] < 0x7F {
                s.push(buf[i] as char);
            }
            i += 1;
        }
        s.trim().to_string()
    };
    let off = |o: usize| -> usize {
        u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]) as usize
    };
    let name = format!("{} {}", read_str(off(12)), read_str(off(16)));
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Model name of the physical device behind a drive letter.
pub fn device_model(letter: char) -> Option<String> {
    let h = open_volume(letter)?;
    let name = query_device_name(h);
    unsafe {
        CloseHandle(h);
    }
    name
}

fn query_seek_penalty(h: HANDLE) -> Option<bool> {
    let mut q = StoragePropertyQuery {
        property_id: STORAGE_DEVICE_SEEK_PENALTY_PROPERTY,
        query_type: PROPERTY_STANDARD_QUERY,
        additional: [0],
    };
    let mut d = DeviceSeekPenaltyDescriptor::default();
    let mut bytes: DWORD = 0;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &mut q as *mut _ as *mut c_void,
            std::mem::size_of::<StoragePropertyQuery>() as DWORD,
            &mut d as *mut _ as *mut c_void,
            std::mem::size_of::<DeviceSeekPenaltyDescriptor>() as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        None
    } else {
        Some(d.incurs_seek_penalty != 0)
    }
}

/// Classify a drive's physical media. A removable device or one attached over
/// USB is reported as `Usb` (so external/enclosure drives stay allowed), while
/// internal drives are split into `Hdd` (seek penalty) and `Ssd` (none).
pub fn media_kind(letter: char) -> MediaKind {
    let removable = drive_type(letter) == DRIVE_REMOVABLE;
    let h = match open_volume(letter) {
        Some(h) => h,
        None => {
            return if removable {
                MediaKind::Usb
            } else {
                MediaKind::Unknown
            }
        }
    };
    let bus = query_bus_type(h);
    let seek = query_seek_penalty(h);
    unsafe {
        CloseHandle(h);
    }
    if removable || bus == BUS_TYPE_USB {
        MediaKind::Usb
    } else {
        match seek {
            Some(false) => MediaKind::Ssd,
            Some(true) => MediaKind::Hdd,
            None => MediaKind::Unknown,
        }
    }
}

pub fn system_drive_letter() -> char {
    let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    sys.chars().next().unwrap_or('C').to_ascii_uppercase()
}

pub struct VolumeInfo {
    pub fs: String,
    pub label: String,
    pub total: u64,
    pub free: u64,
}

pub fn volume_info(letter: char) -> Option<VolumeInfo> {
    let root = format!("{}:\\", letter.to_ascii_uppercase());
    let wide = to_wide(&root);
    unsafe {
        let mut label = [0u16; 64];
        let mut fsname = [0u16; 32];
        let mut serial: DWORD = 0;
        let mut maxcomp: DWORD = 0;
        let mut flags: DWORD = 0;
        let ok = GetVolumeInformationW(
            wide.as_ptr(),
            label.as_mut_ptr(),
            label.len() as DWORD,
            &mut serial,
            &mut maxcomp,
            &mut flags,
            fsname.as_mut_ptr(),
            fsname.len() as DWORD,
        );
        if ok == 0 {
            return None;
        }
        let mut free_avail: u64 = 0;
        let mut total: u64 = 0;
        let mut total_free: u64 = 0;
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_avail, &mut total, &mut total_free);
        Some(VolumeInfo {
            fs: from_wide(&fsname),
            label: from_wide(&label),
            total,
            free: total_free,
        })
    }
}

/// Tell the shell something changed so File Explorer picks it up without F5.
/// This works from an elevated process too: SHChangeNotify is a shell
/// broadcast, and UIPI only blocks messages travelling *up* from a
/// lower-integrity process, not notifications sent *down* to the shell.
pub fn refresh_explorer() {
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        for ch in logical_drives() {
            let root = to_wide(&format!("{}:\\", ch));
            let ptr = root.as_ptr() as *mut c_void;
            SHChangeNotify(SHCNE_MEDIAINSERTED, SHCNF_PATHW, ptr, std::ptr::null_mut());
            SHChangeNotify(SHCNE_DRIVEADD, SHCNF_PATHW, ptr, std::ptr::null_mut());
            SHChangeNotify(SHCNE_UPDATEDIR, SHCNF_PATHW, ptr, std::ptr::null_mut());
        }
    }
}

// ---------------- logging setup ----------------

pub fn default_log_path() -> PathBuf {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    dir.join(format!("usb-reformat_{}.log", file_stamp()))
}

pub fn setup_log() -> PathBuf {
    let path = default_log_path();
    setup_log_at(&path);
    path
}

pub fn setup_log_at(path: &Path) {
    let _ = LOG_PATH.set(path.to_path_buf());
    let file = fs::File::create(path).ok();
    let _ = LOGGER.set(Mutex::new(file));
}

/// Path of the pause flag for a given worker log file. The GUI raises it to
/// hold a running full format and the elevated worker checks it, so it lives
/// in the system temp folder (writable by the GUI, readable by the worker)
/// and is derived from the log file name so both sides agree on it.
pub fn pause_flag_path_for(log_path: &Path) -> PathBuf {
    let name = log_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "usb-reformat".to_string());
    std::env::temp_dir().join(format!("{}.pause", name))
}

/// Block the wipe while the pause flag exists. Called from the overwrite loop.
fn wait_if_paused() {
    let Some(lp) = LOG_PATH.get() else {
        return;
    };
    let flag = pause_flag_path_for(lp);
    if !flag.exists() {
        return;
    }
    log(
        "INFO",
        "PAUSED - writing is on hold. Click Resume to continue.",
    );
    while flag.exists() {
        std::thread::sleep(Duration::from_millis(250));
    }
    log("INFO", "RESUMED - continuing overwrite.");
}

pub fn log_drives(label: &str) {
    log("INFO", &format!("----- DRIVE SNAPSHOT: {} -----", label));
    for ch in logical_drives() {
        match volume_info(ch) {
            Some(v) => log(
                "DATA",
                &format!(
                    "{}: fs={} label='{}' total={:.2}GB free={:.2}GB media={}",
                    ch,
                    v.fs,
                    v.label,
                    v.total as f64 / 1e9,
                    v.free as f64 / 1e9,
                    media_kind(ch).label()
                ),
            ),
            None => log("DATA", &format!("{}: (no media / unavailable)", ch)),
        }
    }
    log("INFO", &format!("----- END SNAPSHOT: {} -----", label));
}

// ---------------- core wipe ----------------

fn wipe_files(letter: char) {
    let root = format!("{}:\\", letter);
    match fs::read_dir(&root) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let res = if is_dir {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                if let Err(e) = res {
                    log(
                        "WARN",
                        &format!("could not remove {}: {}", path.display(), e),
                    );
                }
            }
        }
        Err(e) => log("WARN", &format!("file wipe issue: {}", e)),
    }
}

/// Hard cap on a single diskpart invocation. A wedged `diskpart` (for example a
/// USB stick that has stopped responding to I/O) must never hang the worker
/// forever: the GUI cannot clear its "busy" state while the worker runs, and a
/// stuck worker is what makes the window unclosable.
const DISKPART_TIMEOUT: Duration = Duration::from_secs(300);

/// Create a job object that terminates every process in it when the last handle
/// closes. The worker puts its `diskpart` child in such a job, so if the worker
/// is force-terminated (user cancels) the OS takes diskpart down with it.
fn kill_on_close_job() -> Option<HANDLE> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JobExtendedLimitInformation = std::mem::zeroed();
        info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<JobExtendedLimitInformation>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            return None;
        }
        Some(job)
    }
}

/// Run a diskpart script through `diskpart /s`, streaming its output to the log
/// and enforcing a hard timeout.
fn run_diskpart(script: &str) -> bool {
    run_diskpart_timeout(script, DISKPART_TIMEOUT)
}

fn run_diskpart_timeout(script: &str, timeout: Duration) -> bool {
    let pid = std::process::id();
    let dir = std::env::temp_dir();
    let script_path = dir.join(format!("usb-reformat-dp-{}.txt", pid));
    let out_path = dir.join(format!("usb-reformat-dp-{}.out", pid));
    let err_path = dir.join(format!("usb-reformat-dp-{}.err", pid));

    if let Err(e) = fs::write(&script_path, script.as_bytes()) {
        log("ERROR", &format!("could not write diskpart script: {}", e));
        return false;
    }
    let out_file = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => {
            log(
                "ERROR",
                &format!("could not open diskpart output log: {}", e),
            );
            return false;
        }
    };
    let err_file = match fs::File::create(&err_path) {
        Ok(f) => f,
        Err(e) => {
            log(
                "ERROR",
                &format!("could not open diskpart error log: {}", e),
            );
            return false;
        }
    };

    log(
        "INFO",
        &format!(
            "running diskpart /s {} (timeout {}s)",
            script_path.display(),
            timeout.as_secs()
        ),
    );
    // CREATE_NO_WINDOW: diskpart is a console app and there is no console in
    // the GUI worker; without this flag Windows would flash a console window.
    let spawned = Command::new("diskpart")
        .arg("/s")
        .arg(&script_path)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn();

    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            log("ERROR", &format!("failed to run diskpart: {}", e));
            let _ = fs::remove_file(&script_path);
            return false;
        }
    };

    // Best effort: tie diskpart's lifetime to the worker so force-killing the
    // worker (cancel) cannot leave an orphaned, unstoppable diskpart behind.
    let job = kill_on_close_job();
    if let Some(j) = job {
        unsafe {
            AssignProcessToJobObject(j, child.as_raw_handle() as HANDLE);
        }
    }

    let start = Instant::now();
    let mut last_beat = 0u64;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    log(
                        "ERROR",
                        &format!(
                            "diskpart did not finish within {}s - terminating it. \
                             The drive may be left in an inconsistent state; if it stops \
                             responding you may need to unplug it.",
                            timeout.as_secs()
                        ),
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                let secs = elapsed.as_secs();
                if secs >= last_beat + 5 {
                    last_beat = secs;
                    log("INFO", &format!("diskpart still running ({}s)...", secs));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                log("ERROR", &format!("diskpart wait error: {}", e));
                let _ = child.kill();
                break None;
            }
        }
    };

    // Emit whatever diskpart wrote. Console programs buffer their output when
    // not attached to a console, so this often only arrives once it has exited.
    let mut combined = String::new();
    for p in [&out_path, &err_path] {
        if let Ok(text) = fs::read_to_string(p) {
            for line in text.lines() {
                if !line.trim().is_empty() {
                    log("DATA", line.trim_end());
                    combined.push_str(line.trim_end());
                    combined.push('\n');
                }
            }
        }
    }

    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(&out_path);
    let _ = fs::remove_file(&err_path);
    if let Some(j) = job {
        unsafe {
            CloseHandle(j);
        }
    }

    // diskpart exits 0 even when a command inside the script fails, so the
    // exit code on its own is not a reliable answer. Judge it by what diskpart
    // actually printed.
    let mut ok = match status {
        Some(s) if s.success() => true,
        Some(s) => {
            log("ERROR", &format!("diskpart exited with {:?}", s.code()));
            false
        }
        None => {
            if !timed_out {
                log("ERROR", "diskpart did not complete.");
            }
            false
        }
    };

    if ok {
        for line in combined.lines() {
            let low = line.to_ascii_lowercase();
            if low.contains("virtual disk service error")
                || low.contains("diskpart has encountered an error")
                || low.contains("the requested operation failed")
                || low.contains("access is denied")
                || low.contains("cannot find the file")
            {
                log("ERROR", &format!("diskpart reported: {}", line.trim()));
                if low.contains("access is denied") {
                    log(
                        "ERROR",
                        "Windows refused access to the drive. Something still had it open, such as \
                         an Explorer window on that drive.",
                    );
                }
                ok = false;
            }
        }
    }
    ok
}

/// Quick rebuild: clean -> create -> format -> assign. Old data is simply
/// discarded (no overwrite), so this is fast.
fn diskpart_rebuild(disk: u32, letter: char, fs_kind: &str) -> bool {
    let script = format!(
        "select disk {}\r\nclean\r\ncreate partition primary\r\nformat fs={} quick label=DATA\r\nassign letter={}\r\n",
        disk,
        fs_kind.to_ascii_lowercase(),
        letter
    );
    let ok = run_diskpart(&script);
    if ok {
        std::thread::sleep(Duration::from_secs(2));
    }
    ok
}

/// Remove all partitions so the raw disk can be overwritten by hand.
fn diskpart_clean(disk: u32) -> bool {
    run_diskpart(&format!("select disk {}\r\nclean\r\n", disk))
}

/// Create one primary partition, quick-format it and assign the drive letter.
/// `clean` runs again first: the raw overwrite wiped the partition table, and
/// diskpart refuses to create a partition on an uninitialized disk.
fn diskpart_format(disk: u32, letter: char, fs_kind: &str) -> bool {
    let script = format!(
        "select disk {}\r\nclean\r\ncreate partition primary\r\nformat fs={} quick label=DATA\r\nassign letter={}\r\n",
        disk,
        fs_kind.to_ascii_lowercase(),
        letter
    );
    if run_diskpart(&script) {
        std::thread::sleep(Duration::from_secs(3));
        return true;
    }
    log(
        "WARN",
        &format!("{}: first format attempt failed - retrying.", letter),
    );
    std::thread::sleep(Duration::from_secs(3));
    let ok = run_diskpart(&script);
    if ok {
        std::thread::sleep(Duration::from_secs(3));
    }
    ok
}

// ---------------- full wipe: native multi-pass overwrite ----------------

/// One overwrite pass: a repeating byte/sequence, or fresh random data.
#[derive(Clone)]
enum Pass {
    Repeat(Vec<u8>),
    Random,
}

/// The overwrite schedule for a chosen number of passes.
fn pass_schedule(passes: u32) -> Vec<Pass> {
    fn r() -> Pass {
        Pass::Random
    }
    fn b(x: u8) -> Pass {
        Pass::Repeat(vec![x])
    }
    fn s(x: &[u8]) -> Pass {
        Pass::Repeat(x.to_vec())
    }
    match passes {
        1 => vec![b(0x00)],
        3 => vec![b(0x00), b(0xFF), r()],
        7 => vec![b(0x00), b(0xFF), r(), b(0x96), b(0x69), r(), b(0x00)],
        35 => vec![
            r(),
            r(),
            r(),
            r(),
            b(0x55),
            b(0xAA),
            s(&[0x92, 0x49, 0x24]),
            s(&[0x49, 0x24, 0x92]),
            s(&[0x24, 0x92, 0x49]),
            b(0x00),
            b(0x11),
            b(0x22),
            b(0x33),
            b(0x44),
            b(0x55),
            b(0x66),
            b(0x77),
            b(0x88),
            b(0x99),
            b(0xAA),
            b(0xBB),
            b(0xCC),
            b(0xDD),
            b(0xEE),
            b(0xFF),
            s(&[0x92, 0x49, 0x24]),
            s(&[0x49, 0x24, 0x92]),
            s(&[0x24, 0x92, 0x49]),
            s(&[0x6D, 0xB6, 0xDB]),
            s(&[0xB6, 0xDB, 0x6D]),
            s(&[0xDB, 0x6D, 0xB6]),
            r(),
            r(),
            r(),
            r(),
        ],
        n => {
            // Unknown count: write zeros, then a random pass.
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(b(0x00));
            }
            v.push(r());
            v
        }
    }
}

// Win32 error codes worth calling out by name, because the fix for each is
// completely different and a bare number tells the user nothing.
const ERROR_WRITE_PROTECT: u32 = 19;
const ERROR_SECTOR_NOT_FOUND: u32 = 55;
const ERROR_DISK_FULL: u32 = 112;
const ERROR_DEVICE_NOT_CONNECTED: u32 = 433;
const ERROR_IO_DEVICE: u32 = 1117;

/// How many times a single chunk is retried before the wipe is abandoned.
/// The OS retries these itself (System log, event id 153) and usually wins, so
/// aborting on the first failed write threw away hours of work over a hiccup.
const WRITE_RETRIES: u32 = 8;

/// True for errors that mean "the drive hiccuped", not "the drive is dead".
/// Worth another attempt rather than failing the whole wipe.
fn transient_write_error(err: u32) -> bool {
    matches!(
        err,
        5     // ERROR_ACCESS_DENIED - device busy while the bus resets
        | 6    // ERROR_INVALID_HANDLE
        | 21   // ERROR_NOT_READY
        | 31   // ERROR_GEN_FAILURE
        | 1117 // ERROR_IO_DEVICE - reported an error under sustained load
        | 1219 // ERROR_REQUEST_ABORTED
        | 1611 // ERROR_BUS_RESET
        | 1617 // ERROR_DEVICE_REMOVED - USB re-enumerating
    )
}

/// Human-readable description of the wipe mode.
pub fn pass_label(passes: u32) -> String {
    match passes {
        0 => "quick wipe (no overwrite)".to_string(),
        1 => "full wipe, 1 pass (single zero overwrite)".to_string(),
        3 => "full wipe, 3 passes (DoD 5220.22-M)".to_string(),
        7 => "full wipe, 7 passes (DoD 5220.22-M ECE)".to_string(),
        35 => "full wipe, 35 passes (Gutmann method)".to_string(),
        n => format!("full wipe, {} passes", n),
    }
}

/// Short label for the GUI checkbox.
pub fn passes_short(passes: u32) -> String {
    match passes {
        1 => "1 (fast)".to_string(),
        3 => "3 (DoD)".to_string(),
        7 => "7 (DoD-ECE)".to_string(),
        35 => "35 (Gutmann)".to_string(),
        n => format!("{}", n),
    }
}

/// Small xorshift64 PRNG (no external crates).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn fill_buffer(buf: &mut [u8], pass: &Pass, rng: &mut Rng) {
    match pass {
        Pass::Random => {
            let mut i = 0;
            while i + 8 <= buf.len() {
                let r = rng.next_u64().to_le_bytes();
                buf[i..i + 8].copy_from_slice(&r);
                i += 8;
            }
            if i < buf.len() {
                let r = rng.next_u64().to_le_bytes();
                let n = buf.len() - i;
                buf[i..].copy_from_slice(&r[..n]);
            }
        }
        Pass::Repeat(seq) => {
            if seq.is_empty() {
                return;
            }
            for (i, b) in buf.iter_mut().enumerate() {
                *b = seq[i % seq.len()];
            }
        }
    }
}

/// Lock and dismount every volume that lives on `disk`. Without this, a
/// `WriteFile` to the physical disk **blocks forever** the moment it reaches
/// sectors inside a still-mounted volume (this is exactly what made the full
/// wipe hang at 0%). The returned handles stay locked for the duration of the
/// overwrite and must be closed afterwards.
fn lock_volumes_on_disk(disk: u32) -> Vec<HANDLE> {
    let mut locked: Vec<HANDLE> = Vec::new();
    let mut name = vec![0u16; 260];
    unsafe {
        let find = FindFirstVolumeW(name.as_mut_ptr(), name.len() as DWORD);
        if !raw_valid(find) {
            return locked;
        }
        loop {
            let vol = from_wide(&name);
            let wide = to_wide(vol.trim_end_matches('\0'));
            let h = CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if raw_valid(h) {
                let mut ex = VolumeDiskExtents {
                    number_of_disk_extents: 0,
                    extents: [DiskExtent::default(); 16],
                };
                let mut br: DWORD = 0;
                let ok = DeviceIoControl(
                    h,
                    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                    std::ptr::null_mut(),
                    0,
                    &mut ex as *mut _ as *mut c_void,
                    std::mem::size_of::<VolumeDiskExtents>() as DWORD,
                    &mut br,
                    std::ptr::null_mut(),
                );
                let n = ex.number_of_disk_extents.min(16) as usize;
                let on_disk =
                    ok != 0 && n > 0 && ex.extents[..n].iter().any(|e| e.disk_number == disk);
                if on_disk {
                    let mut junk: DWORD = 0;
                    let lk = DeviceIoControl(
                        h,
                        FSCTL_LOCK_VOLUME,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut junk,
                        std::ptr::null_mut(),
                    );
                    let dm = DeviceIoControl(
                        h,
                        FSCTL_DISMOUNT_VOLUME,
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut junk,
                        std::ptr::null_mut(),
                    );
                    if lk != 0 || dm != 0 {
                        // Keep the handle open (and locked) for the whole overwrite.
                        locked.push(h);
                    } else {
                        CloseHandle(h);
                    }
                } else {
                    CloseHandle(h);
                }
            }
            if FindNextVolumeW(find, name.as_mut_ptr(), name.len() as DWORD) == 0 {
                break;
            }
        }
        FindVolumeClose(find);
    }
    locked
}

/// Overwrite every sector of the physical disk `passes` times. The disk must
/// have no mounted volumes (we lock/dismount them here) and we must be admin.
fn overwrite_disk(disk: u32, passes: u32) -> bool {
    let path = format!("\\\\.\\PhysicalDrive{}", disk);
    let wide = to_wide(&path);

    // Detach the file system first: raw writes to sectors inside a mounted
    // volume block forever otherwise.
    let locked = lock_volumes_on_disk(disk);
    log(
        "INFO",
        &format!(
            "{}: locked/dismounted {} volume(s) before raw writes",
            path,
            locked.len()
        ),
    );

    let mut h: HANDLE = std::ptr::null_mut();
    for attempt in 0..20 {
        h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                std::ptr::null_mut(),
            )
        };
        if raw_valid(h) {
            break;
        }
        if attempt == 19 {
            log(
                "ERROR",
                &format!("could not open {} for writing (err {}).", path, unsafe {
                    GetLastError()
                }),
            );
            unsafe {
                for lh in &locked {
                    CloseHandle(*lh);
                }
            }
            return false;
        }
        std::thread::sleep(Duration::from_millis(750));
    }

    let length: i64;
    unsafe {
        let mut info = GetLengthInformation { length: 0 };
        let mut bytes: DWORD = 0;
        let ok = DeviceIoControl(
            h,
            IOCTL_DISK_GET_LENGTH_INFO,
            std::ptr::null_mut(),
            0,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<GetLengthInformation>() as DWORD,
            &mut bytes,
            std::ptr::null_mut(),
        );
        if ok == 0 || info.length <= 0 {
            log(
                "ERROR",
                &format!("could not read size of {} (err {}).", path, GetLastError()),
            );
            CloseHandle(h);
            for lh in &locked {
                CloseHandle(*lh);
            }
            return false;
        }
        length = info.length;
    }

    // Only whole sectors. Round down to 4096 so the size is valid for both
    // 512- and 4096-byte sector devices under FILE_FLAG_NO_BUFFERING.
    let total = (length as u64) & !0xFFFu64;
    let schedule = pass_schedule(passes);
    const CHUNK: usize = 1 << 20; // 1 MiB

    // FILE_FLAG_NO_BUFFERING needs a sector-aligned buffer, so over-allocate
    // and align manually (covers both 512- and 4096-byte sector devices).
    let mut backing = vec![0u64; CHUNK / 8 + 1024];
    let base = backing.as_mut_ptr() as usize;
    let aligned = (base + 4095) & !4095usize;
    let buf: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(aligned as *mut u8, CHUNK) };

    // Seed from the clock so different runs write different random data.
    let seed = {
        let st = local_time();
        (st.milliseconds as u64)
            ^ ((st.second as u64) << 16)
            ^ ((st.minute as u64) << 24)
            ^ 0x9E37_79B9_7F4A_7C15
    };

    let mut ok_all = true;
    for (i, pass) in schedule.iter().enumerate() {
        log(
            "INFO",
            &format!(
                "overwrite pass {}/{} over {:.2} GB ...",
                i + 1,
                schedule.len(),
                total as f64 / 1e9
            ),
        );
        let mut rng = Rng::new(seed ^ (i as u64).wrapping_mul(0x1000_0000_1B3));
        let mut written: u64 = 0;
        let mut last_pct: u64 = u64::MAX;
        while written < total {
            // Honour a pause request between chunks (checked once per MiB).
            wait_if_paused();
            let n = std::cmp::min(CHUNK as u64, total - written) as usize;
            fill_buffer(&mut buf[..n], pass, &mut rng);

            // Retry the same chunk on transient errors. A drive under
            // sustained load often reports one bad write and then accepts the
            // next one, and Windows retries these internally anyway (System
            // log event 153). Giving up on the first failure killed a
            // multi-hour wipe over a single hiccup.
            let mut attempt: u32 = 0;
            let mut done: DWORD;
            loop {
                done = 0;
                let w = unsafe {
                    WriteFile(
                        h,
                        buf.as_ptr() as *const c_void,
                        n as DWORD,
                        &mut done,
                        std::ptr::null_mut(),
                    )
                };
                if w != 0 && done > 0 {
                    break;
                }
                let err = unsafe { GetLastError() };
                attempt += 1;
                if attempt >= WRITE_RETRIES || !transient_write_error(err) {
                    log(
                        "ERROR",
                        &format!("write failed at offset {} (err {}).", written, err),
                    );
                    // Say what actually went wrong - a dropped USB device is
                    // the single most common cause and needs completely
                    // different advice from a generic write error.
                    match err {
                        ERROR_DEVICE_NOT_CONNECTED => log(
                            "ERROR",
                            "The USB device disconnected mid-write. This is a hardware / \
                             connection problem - the port, cable or drive dropped out under \
                             load - not a fault in this tool. Reconnect the drive and try a \
                             different USB port.",
                        ),
                        ERROR_IO_DEVICE => log(
                            "ERROR",
                            "The drive reported an I/O error under sustained load. It managed a \
                             previous pass and then stalled, which usually means the device is \
                             overheating or worn out rather than a connection fault. Let it cool \
                             and try a single pass instead of several.",
                        ),
                        ERROR_DISK_FULL => log(
                            "ERROR",
                            "The drive ran out of addressable space - it may have failed or be a \
                             fake capacity device.",
                        ),
                        ERROR_WRITE_PROTECT => log(
                            "ERROR",
                            "The device is write-protected, so it could not be overwritten.",
                        ),
                        ERROR_SECTOR_NOT_FOUND => log(
                            "ERROR",
                            "The drive reported a bad sector and cannot be overwritten reliably. \
                             It has probably failed - do not trust it.",
                        ),
                        _ => {}
                    }
                    ok_all = false;
                    break;
                }
                // Back off so a struggling device gets a moment instead of
                // being hammered while it recovers.
                let wait_ms = 250u64 * attempt as u64;
                log(
                    "WARN",
                    &format!(
                        "write hiccup at offset {} (err {}), retry {}/{} in {} ms ...",
                        written, err, attempt, WRITE_RETRIES, wait_ms
                    ),
                );
                let give_up = Instant::now() + Duration::from_millis(wait_ms);
                while Instant::now() < give_up {
                    wait_if_paused();
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            if !ok_all {
                break;
            }
            written += done as u64;
            let pct = if total > 0 {
                written * 100 / total
            } else {
                100
            };
            if pct != last_pct {
                last_pct = pct;
                log(
                    "DATA",
                    &format!("  pass {}/{} : {}%", i + 1, schedule.len(), pct),
                );
            }
        }
        unsafe {
            FlushFileBuffers(h);
        }
        if !ok_all {
            break;
        }
    }

    unsafe {
        CloseHandle(h);
        for lh in &locked {
            CloseHandle(*lh);
        }
    }
    ok_all
}

pub fn parse_letters(input: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for tok in input.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let t = tok.trim().trim_end_matches(':');
        if t.is_empty() {
            continue;
        }
        if let Some(ch) = t.chars().next() {
            let up = ch.to_ascii_uppercase();
            if up.is_ascii_alphabetic() && !out.contains(&up) {
                out.push(up);
            }
        }
    }
    out
}

/// Returns true when the drive ended in the desired state (or was intentionally
/// protected/skipped), false on a real failure.
pub fn process_target(letter: char, fs_kind: &str, interactive: bool, passes: u32) -> bool {
    sayln("");
    sayln(&format!("##########  TARGET {}:  ##########", letter));
    log("INFO", &format!("########## TARGET {}: ##########", letter));

    if letter == system_drive_letter() {
        log(
            "SKIP",
            &format!("{}: is the Windows system drive - protected.", letter),
        );
        return true;
    }

    let dt = drive_type(letter);
    if dt != DRIVE_REMOVABLE && dt != DRIVE_FIXED {
        log(
            "SKIP",
            &format!(
                "{}: is a {} drive, not a local disk - refusing.",
                letter,
                drive_type_name(dt)
            ),
        );
        return true;
    }
    if dt == DRIVE_FIXED {
        if interactive {
            log(
                "WARN",
                &format!("{}: is a FIXED disk (not removable) - double-check this is really your USB drive.", letter),
            );
            let again = prompt(&format!(
                "Type the letter {} again to confirm wiping this fixed disk: ",
                letter
            ));
            if !again.eq_ignore_ascii_case(&letter.to_string()) {
                log(
                    "SKIP",
                    &format!("{}: fixed disk not confirmed - skipping.", letter),
                );
                return true;
            }
        } else {
            log(
                "WARN",
                &format!(
                    "{}: FIXED disk - proceeding (confirmed in the GUI).",
                    letter
                ),
            );
        }
    }

    let (disk, part) = match device_number(letter) {
        Some(x) => x,
        None => {
            log(
                "ERROR",
                &format!(
                    "{}: no partition found (CD/DVD, network, or not present).",
                    letter
                ),
            );
            return false;
        }
    };
    log(
        "INFO",
        &format!(
            "{}: disk={} part={} mode={}",
            letter,
            disk,
            part,
            pass_label(passes)
        ),
    );

    // Never multi-pass overwrite a solid-state drive: it does not reliably
    // erase flash cells and it wastes write cycles. External/USB SSDs are
    // reported as Usb, so only internal SSDs are blocked.
    let media = media_kind(letter);
    log(
        "INFO",
        &format!("{}: media detected as {}", letter, media.label()),
    );
    if passes >= 1 && media.is_ssd() {
        log(
            "ERROR",
            &format!(
                "{}: {} is a solid-state drive - full (multi-pass) wipe REFUSED. \
                 Overwriting an SSD does not securely erase it and only wears it out. \
                 Use quick wipe instead.",
                letter,
                media.label()
            ),
        );
        return false;
    }

    let total_steps = if passes >= 1 { 5 } else { 3 };

    say(&format!(
        "   [1/{}] wiping files ................ ",
        total_steps
    ));
    wipe_files(letter);
    sayln("done");
    log(
        "STEP",
        &format!("{}: [1/{}] files wiped", letter, total_steps),
    );

    if passes >= 1 {
        // FULL wipe: drop the partitions, overwrite every sector by hand,
        // then create + format a fresh partition.
        say(&format!(
            "   [2/{}] removing partitions ....... ",
            total_steps
        ));
        say("\r\n");
        spinner_start("cleaning");
        let cleaned = diskpart_clean(disk);
        spinner_stop();
        if !cleaned {
            log(
                "ERROR",
                &format!("{}: could not clean disk - aborting full wipe.", letter),
            );
            return false;
        }
        say(&format!(
            "   [2/{}] removing partitions ....... ",
            total_steps
        ));
        sayln("done");
        log(
            "STEP",
            &format!("{}: [2/{}] partitions removed", letter, total_steps),
        );

        // Give the OS a moment to release the volume before raw writes.
        std::thread::sleep(Duration::from_secs(2));

        say(&format!(
            "   [3/{}] overwriting disk ({}) ... ",
            total_steps,
            pass_label(passes)
        ));
        say("\r\n");
        spinner_start("overwriting");
        let overwritten = overwrite_disk(disk, passes);
        spinner_stop();
        if !overwritten {
            log(
                "ERROR",
                &format!("{}: overwrite FAILED - drive may be left blank.", letter),
            );
            return false;
        }
        say(&format!(
            "   [3/{}] overwriting disk ........... ",
            total_steps
        ));
        sayln("done");
        log(
            "STEP",
            &format!(
                "{}: [3/{}] disk overwritten ({})",
                letter,
                total_steps,
                pass_label(passes)
            ),
        );

        say(&format!(
            "   [4/{}] creating + formatting ..... ",
            total_steps
        ));
        say("\r\n");
        spinner_start("formatting");
        let formatted = diskpart_format(disk, letter, fs_kind);
        spinner_stop();
        if !formatted {
            log(
                "ERROR",
                &format!("{}: format FAILED - drive may be left blank.", letter),
            );
            return false;
        }
        say(&format!(
            "   [4/{}] creating + formatting ..... ",
            total_steps
        ));
        sayln("done");
        log(
            "STEP",
            &format!("{}: [4/{}] partition formatted", letter, total_steps),
        );
    } else {
        say(&format!(
            "   [2/{}] rebuilding partition ....... ",
            total_steps
        ));
        say("\r\n");
        spinner_start("working");
        // Dismount before rebuilding. Without this the volume is still mounted
        // and Windows refuses the clean with "Access is denied", which is what
        // made quick wipes fail whenever the drive was still open.
        let quick_locked = lock_volumes_on_disk(disk);
        std::thread::sleep(Duration::from_secs(2));
        let rebuilt = diskpart_rebuild(disk, letter, fs_kind);
        for h in &quick_locked {
            unsafe {
                CloseHandle(*h);
            }
        }
        spinner_stop();
        if !rebuilt {
            log(
                "ERROR",
                &format!("{}: rebuild FAILED - drive may be left empty.", letter),
            );
            return false;
        }
        say(&format!(
            "   [2/{}] rebuilding partition ....... ",
            total_steps
        ));
        sayln("done");
        log(
            "STEP",
            &format!("{}: [2/{}] partition rebuilt", letter, total_steps),
        );
    }

    say(&format!(
        "   [{}/{}] verifying .................. ",
        total_steps, total_steps
    ));
    match volume_info(letter) {
        Some(v) if v.fs.eq_ignore_ascii_case(fs_kind) => {
            sayln(&format!(
                "OK  ({}, label '{}', {:.2} GB free)",
                v.fs,
                v.label,
                v.free as f64 / 1e9
            ));
            log(
                "OK",
                &format!(
                    "{}/{} VERIFIED {} label='{}' {:.2} GB free",
                    total_steps,
                    total_steps,
                    v.fs,
                    v.label,
                    v.free as f64 / 1e9
                ),
            );
            true
        }
        Some(v) => {
            sayln(&format!("FAILED (got {})", v.fs));
            log(
                "ERROR",
                &format!(
                    "{}/{} filesystem is '{}', not {}",
                    total_steps, total_steps, v.fs, fs_kind
                ),
            );
            false
        }
        None => {
            sayln("FAILED (could not verify volume)");
            log(
                "ERROR",
                &format!("{}/{} could not verify volume", total_steps, total_steps),
            );
            false
        }
    }
}

// ---------------- elevated worker ----------------

/// Headless elevated worker: processes each drive, writing progress to the log
/// file. Exit code 0 = all ok, 1 = at least one failure.
pub fn run_worker(drives: &[char], fs_kind: &str, passes: u32, log_path: &Path) -> i32 {
    setup_log_at(log_path);

    log("INFO", "================ WORKER START ================");
    log("INFO", &format!("Drives   : {:?}", drives));
    log("INFO", &format!("Filesys  : {}", fs_kind));
    log("INFO", &format!("Wipe mode: {}", pass_label(passes)));
    log(
        "INFO",
        &format!(
            "Computer : {}",
            std::env::var("COMPUTERNAME").unwrap_or_default()
        ),
    );
    log(
        "INFO",
        &format!(
            "User     : {}",
            std::env::var("USERNAME").unwrap_or_default()
        ),
    );
    log(
        "INFO",
        &format!(
            "SystemDrv: {}",
            std::env::var("SystemDrive").unwrap_or_default()
        ),
    );
    log_drives("BEFORE");

    let mut fails = 0;
    for letter in drives {
        if !process_target(*letter, fs_kind, false, passes) {
            fails += 1;
        }
    }

    log_drives("AFTER");
    if fails == 0 {
        log("INFO", "RESULT: SUCCESS");
        log("INFO", "================ WORKER END =================");
        0
    } else {
        log("ERROR", &format!("RESULT: {} drive(s) failed", fails));
        log("INFO", "================ WORKER END =================");
        1
    }
}

/// Relaunch this exe elevated (UAC), hidden, to run the worker with the given
/// argument list. Returns the worker process handle, or an error code.
pub fn spawn_worker(params: &str) -> Result<HANDLE, u32> {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return Err(0),
    };
    let verb = to_wide("runas");
    let file = to_wide(&exe.to_string_lossy());
    let par = to_wide(params);

    let mut sei: ShellExecuteInfoW = unsafe { std::mem::zeroed() };
    sei.cb_size = std::mem::size_of::<ShellExecuteInfoW>() as u32;
    sei.f_mask = SEE_MASK_NOCLOSEPROCESS;
    sei.lp_verb = verb.as_ptr();
    sei.lp_file = file.as_ptr();
    sei.lp_parameters = par.as_ptr();
    sei.n_show = SW_HIDE;

    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok == 0 {
        return Err(unsafe { GetLastError() });
    }
    Ok(sei.h_process)
}

// ---------------- legacy interactive console flow ----------------

pub fn run_workflow() {
    let logfile = setup_log();

    log("INFO", "================ SESSION START ================");
    log("INFO", &format!("Log file : {}", logfile.display()));
    log(
        "INFO",
        &format!(
            "Computer : {}",
            std::env::var("COMPUTERNAME").unwrap_or_default()
        ),
    );
    log(
        "INFO",
        &format!(
            "User     : {}",
            std::env::var("USERNAME").unwrap_or_default()
        ),
    );
    log(
        "INFO",
        &format!(
            "SystemDrv: {}",
            std::env::var("SystemDrive").unwrap_or_default()
        ),
    );
    log_drives("BEFORE");

    sayln("");
    sayln("==============================================");
    sayln("  USB wipe + repartition + format");
    sayln(&format!("  Logging to: {}", logfile.display()));
    sayln("==============================================");
    sayln("Type the drive letter(s) and press Enter (e.g. H, or H P).");

    let letters = parse_letters(&prompt("Drives: "));
    log("INFO", &format!("Parsed letters: {:?}", letters));
    if letters.is_empty() {
        log("WARN", "No valid drive letters entered. Nothing to do.");
        return;
    }

    sayln("");
    sayln("File system for the new partition:");
    sayln("  1 = exFAT   (recommended - files over 4 GB OK)");
    sayln("  2 = FAT32   (4 GB per-file limit)");
    let fs_input = prompt("Choose 1 or 2 (Enter = 1): ");
    let fs_kind = if fs_input == "2" || fs_input.eq_ignore_ascii_case("fat32") {
        "FAT32"
    } else {
        "exFAT"
    };
    log(
        "INFO",
        &format!("Filesystem choice: '{}' -> {}", fs_input, fs_kind),
    );
    if fs_kind == "FAT32" {
        log(
            "WARN",
            "FAT32 selected: 4 GB per-file limit, and Windows may refuse FAT32 on drives over 32 GB.",
        );
    }

    let confirm = prompt("Type WIPE to continue: ");
    log("INFO", &format!("Confirmation: '{}'", confirm));
    if confirm != "WIPE" {
        log("WARN", "Not confirmed. Nothing changed.");
        return;
    }

    let full_input = prompt("Full wipe? 0=quick, 1=fast, 3=DoD, 7=DoD-ECE, 35=Gutmann [0]: ");
    let passes: u32 = match full_input.trim() {
        "" | "0" => 0,
        "1" => 1,
        "3" => 3,
        "7" => 7,
        "35" => 35,
        other => other.parse().unwrap_or(0),
    };
    log("INFO", &format!("Wipe mode: {}", pass_label(passes)));

    sayln("");
    sayln("Wiping...");
    for letter in &letters {
        process_target(*letter, fs_kind, true, passes);
    }

    refresh_explorer();
    log_drives("AFTER");
    log("INFO", "================ SESSION END ================");
    sayln("");
    sayln(&format!("Finished. Log: {}", logfile.display()));
}

pub fn list_drives() {
    sayln("Detected drives:");
    for ch in logical_drives() {
        let loc = match device_number(ch) {
            Some((disk, part)) => format!("disk {} part {}", disk, part),
            None => "no partition".to_string(),
        };
        let dt = drive_type_name(drive_type(ch));
        let mk = media_kind(ch).label();
        let model = device_model(ch).unwrap_or_else(|| "unknown".to_string());
        match volume_info(ch) {
            Some(v) => sayln(&format!(
                "  {}:  {}  fs={}  label='{}'  {:.2} GB (free {:.2} GB)  [{} | {} | {}]",
                ch,
                model,
                v.fs,
                v.label,
                v.total as f64 / 1e9,
                v.free as f64 / 1e9,
                mk,
                dt,
                loc
            )),
            None => sayln(&format!(
                "  {}:  {}  (unavailable)  [{} | {} | {}]",
                ch, model, mk, dt, loc
            )),
        }
    }
}

pub fn pause_console() {
    pause_if_windowed();
}
