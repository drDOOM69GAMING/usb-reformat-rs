// Minimal Win32 FFI layer. std + these declarations only, no external crates.
#![allow(dead_code)]

use std::ffi::c_void;

pub type BOOL = i32;
pub type DWORD = u32;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type HANDLE = *mut c_void;
pub type HMENU = *mut c_void;
pub type HICON = *mut c_void;
pub type HBRUSH = *mut c_void;
pub type HCURSOR = *mut c_void;
pub type HFONT = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type HWND = *mut c_void;
pub type HDC = *mut c_void;

pub const INVALID_HANDLE_VALUE: isize = -1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wparam: WPARAM,
    pub lparam: LPARAM,
    pub time: u32,
    pub pt: POINT,
}

pub type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cb_size: u32,
    pub style: u32,
    pub lpfn_wnd_proc: Option<WndProc>,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_instance: HINSTANCE,
    pub h_icon: HICON,
    pub h_cursor: HCURSOR,
    pub hbr_background: HBRUSH,
    pub lpsz_menu_name: *const u16,
    pub lpsz_class_name: *const u16,
    pub h_icon_sm: HICON,
}

#[repr(C)]
pub struct InitCommonControlsEx {
    pub dw_size: u32,
    pub dw_icc: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IoCounters {
    pub read_operation_count: u64,
    pub write_operation_count: u64,
    pub other_operation_count: u64,
    pub read_transfer_count: u64,
    pub write_transfer_count: u64,
    pub other_transfer_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct JobBasicLimitInformation {
    pub per_process_user_time_limit: i64,
    pub per_job_user_time_limit: i64,
    pub limit_flags: u32,
    pub minimum_working_set_size: usize,
    pub maximum_working_set_size: usize,
    pub active_process_limit: u32,
    pub affinity: usize,
    pub priority_class: u32,
    pub scheduling_class: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct JobExtendedLimitInformation {
    pub basic_limit_information: JobBasicLimitInformation,
    pub io_info: IoCounters,
    pub process_memory_limit: usize,
    pub job_memory_limit: usize,
    pub peak_process_memory_used: usize,
    pub peak_job_memory_used: usize,
}

// ---- window / control styles ----
pub const WS_OVERLAPPED: u32 = 0x0000_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
pub const WS_CLIPCHILDREN: u32 = 0x0200_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_VSCROLL: u32 = 0x0020_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_TABSTOP: u32 = 0x0001_0000;
pub const WS_GROUP: u32 = 0x0002_0000;

pub const BS_PUSHBUTTON: u32 = 0x0000_0000;
pub const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
pub const BS_CHECKBOX: u32 = 0x0000_0002;
pub const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
pub const BS_RADIOBUTTON: u32 = 0x0000_0004;
pub const BS_AUTORADIOBUTTON: u32 = 0x0000_0009;
pub const BS_GROUPBOX: u32 = 0x0000_0007;

pub const LBS_NOTIFY: u32 = 0x0000_0001;
pub const LBS_HASSTRINGS: u32 = 0x0000_0040;
pub const LBS_NOINTEGRALHEIGHT: u32 = 0x0000_0100;
pub const LBS_EXTENDEDSEL: u32 = 0x0000_0800;

pub const ES_MULTILINE: u32 = 0x0000_0004;
pub const ES_AUTOVSCROLL: u32 = 0x0000_0040;
pub const ES_READONLY: u32 = 0x0000_0800;

// ---- messages ----
pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_SETICON: u32 = 0x0080;
pub const WM_CTLCOLORBTN: u32 = 0x0135;
pub const WM_CTLCOLORDLG: u32 = 0x0136;
pub const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub const WM_CTLCOLOREDIT: u32 = 0x0133;
pub const WM_CTLCOLORLISTBOX: u32 = 0x0134;
pub const WM_GETTEXT: u32 = 0x000D;
pub const WM_GETTEXTLENGTH: u32 = 0x000E;

pub const BM_SETCHECK: u32 = 0x00F1;
pub const BM_GETCHECK: u32 = 0x00F0;
pub const BST_CHECKED: LRESULT = 1;
pub const BST_UNCHECKED: LRESULT = 0;

pub const EM_SETSEL: u32 = 0x00B1;
pub const EM_REPLACESEL: u32 = 0x00C2;
pub const EM_SCROLLCARET: u32 = 0x00B7;
pub const EM_SETREADONLY: u32 = 0x00CF;
pub const WM_CLEAR: u32 = 0x0303;

pub const LB_ADDSTRING: u32 = 0x0180;
pub const LB_RESETCONTENT: u32 = 0x0184;
pub const LB_GETSELCOUNT: u32 = 0x0190;
pub const LB_GETSELITEMS: u32 = 0x0191;
pub const LB_SETSEL: u32 = 0x0185;
pub const LBN_SELCHANGE: u32 = 1;

pub const BN_CLICKED: u32 = 0;

// ---- misc ----
pub const SW_SHOW: i32 = 5;
pub const SW_HIDE: i32 = 0;
pub const SM_CXSCREEN: i32 = 0;
pub const SM_CYSCREEN: i32 = 1;
pub const SM_CXICON: i32 = 11;
pub const SM_CYICON: i32 = 12;
pub const SM_CXSMICON: i32 = 49;
pub const SM_CYSMICON: i32 = 50;
pub const ICON_SMALL: usize = 0;
pub const ICON_BIG: usize = 1;

pub const COLOR_3DFACE: i32 = 15;
pub const COLOR_BTNTEXT: i32 = 18;
pub const COLOR_WINDOW: i32 = 5;
pub const COLOR_HIGHLIGHT: i32 = 13;
pub const TRANSPARENT: i32 = 1;
pub const OPAQUE: i32 = 2;
pub const WHITE_BRUSH: i32 = 0;

// ---- SetWindowPos / RedrawWindow flags ----
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const RDW_INVALIDATE: u32 = 0x0001;
pub const RDW_ERASE: u32 = 0x0004;
pub const RDW_ALLCHILDREN: u32 = 0x0080;
pub const RDW_FRAME: u32 = 0x0400;

// ---- DWM ----
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
pub const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;

pub const IDC_ARROW: usize = 32512;

pub const CS_HREDRAW: u32 = 0x0002;
pub const CS_VREDRAW: u32 = 0x0001;

pub const FW_NORMAL: i32 = 400;
pub const DEFAULT_CHARSET: u32 = 1;
pub const OUT_DEFAULT_PRECIS: u32 = 0;
pub const CLIP_DEFAULT_PRECIS: u32 = 0;
pub const CLEARTYPE_QUALITY: u32 = 5;
// Win98 drew UI text with the non-antialiased system font. Rendering with
// DEFAULT_QUALITY (instead of ClearType) keeps the chunky 1-bit pixel look.
pub const DEFAULT_QUALITY: u32 = 0;
pub const ANTIALIASED_QUALITY: u32 = 4;
pub const FIXED_PITCH: u32 = 1;
pub const DEFAULT_PITCH: u32 = 0;
pub const FF_DONTCARE: u32 = 0;

// ---- sunken / raised 3D edges and static styles (Win98 chrome) ----
pub const WS_DLGFRAME: u32 = 0x4000_0000;
pub const WS_THICKFRAME: u32 = 0x0004_0000;
pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
pub const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
pub const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
pub const WS_EX_STATICEDGE: u32 = 0x0000_4000;
pub const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
pub const SS_LEFTNOWORDWRAP: u32 = 0x0000_000C;
pub const SS_CENTERIMAGE: u32 = 0x0000_0200;
pub const SS_SUNKEN: u32 = 0x0000_1000;
pub const SS_ENDELLIPSIS: u32 = 0x0000_C000;
pub const PBS_SMOOTH: u32 = 0x0000_0001;
pub const BS_OWNERDRAW: u32 = 0x0000_000B;

// ---- embedded resources (RCDATA: the chiptunes) ----
pub const RT_RCDATA: u16 = 10;

// ---- owner draw ----
pub const WM_DRAWITEM: u32 = 0x02AB;
pub const ODA_DRAWENTIRE: u32 = 0x0001;
pub const ODA_SELECTED: u32 = 0x0002;
pub const ODA_FOCUS: u32 = 0x0004;

// ---- DrawText ----
pub const DT_LEFT: u32 = 0x0000;
pub const DT_CENTER: u32 = 0x0001;
pub const DT_VCENTER: u32 = 0x0004;
pub const DT_SINGLELINE: u32 = 0x0020;
pub const DT_END_ELLIPSIS: u32 = 0x8000;
pub const DT_NOPREFIX: u32 = 0x0080;

#[repr(C)]
pub struct DRAWITEMSTRUCT {
    pub hwnd: usize,
    pub w_id: u32,
    pub item_id: u32,
    pub item_action: u32,
    pub item_data: *mut c_void,
    pub rc_item: RECT,
}

pub const ICC_PROGRESS_CLASS: u32 = 0x0000_0020;

pub const MB_OK: u32 = 0x0000_0000;
pub const MB_YESNO: u32 = 0x0000_0004;
pub const MB_ICONERROR: u32 = 0x0000_0010;
pub const MB_ICONWARNING: u32 = 0x0000_0030;
pub const MB_ICONINFORMATION: u32 = 0x0000_0040;
pub const IDYES: i32 = 6;

pub const ERROR_CANCELLED: u32 = 1223;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(lp_module_name: *const u16) -> HINSTANCE;
    pub fn GetLastError() -> u32;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn GetExitCodeProcess(h_process: HANDLE, lp_exit_code: *mut u32) -> BOOL;
    pub fn GetLocalTime(lp_system_time: *mut crate::core::WinSystemTime);
    pub fn GetLogicalDrives() -> DWORD;
    pub fn GetDriveTypeW(lp_root_path_name: *const u16) -> u32;
    pub fn GetVolumeInformationW(
        lp_root_path_name: *const u16,
        lp_volume_name_buffer: *mut u16,
        n_volume_name_size: DWORD,
        lp_volume_serial_number: *mut DWORD,
        lp_maximum_component_length: *mut DWORD,
        lp_file_system_flags: *mut DWORD,
        lp_file_system_name_buffer: *mut u16,
        n_file_system_name_size: DWORD,
    ) -> BOOL;
    pub fn GetDiskFreeSpaceExW(
        lp_directory_name: *const u16,
        lp_free_bytes_available_to_caller: *mut u64,
        lp_total_number_of_bytes: *mut u64,
        lp_total_number_of_free_bytes: *mut u64,
    ) -> BOOL;
    pub fn CreateFileW(
        lp_file_name: *const u16,
        dw_desired_access: DWORD,
        dw_share_mode: DWORD,
        lp_security_attributes: *mut c_void,
        dw_creation_disposition: DWORD,
        dw_flags_and_attributes: DWORD,
        h_template_file: HANDLE,
    ) -> HANDLE;
    pub fn DeviceIoControl(
        h_device: HANDLE,
        dw_io_control_code: DWORD,
        lp_in_buffer: *mut c_void,
        n_in_buffer_size: DWORD,
        lp_out_buffer: *mut c_void,
        n_out_buffer_size: DWORD,
        lp_bytes_returned: *mut DWORD,
        lp_overlapped: *mut c_void,
    ) -> BOOL;
    pub fn CloseHandle(h_object: HANDLE) -> BOOL;
    pub fn WriteFile(
        h_file: HANDLE,
        lp_buffer: *const c_void,
        n_number_of_bytes_to_write: DWORD,
        lp_number_of_bytes_written: *mut DWORD,
        lp_overlapped: *mut c_void,
    ) -> BOOL;
    pub fn FlushFileBuffers(h_file: HANDLE) -> BOOL;
    pub fn FindFirstVolumeW(lpsz_volume_name: *mut u16, cch_buffer_length: DWORD) -> HANDLE;
    pub fn FindNextVolumeW(
        h_find_volume: HANDLE,
        lpsz_volume_name: *mut u16,
        cch_buffer_length: DWORD,
    ) -> BOOL;
    pub fn FindVolumeClose(h_find_volume: HANDLE) -> BOOL;
    pub fn WaitForSingleObject(h_handle: HANDLE, dw_milliseconds: DWORD) -> DWORD;
    pub fn TerminateProcess(h_process: HANDLE, u_exit_code: u32) -> BOOL;
    pub fn CreateJobObjectW(lp_job_attributes: *mut c_void, lp_name: *const u16) -> HANDLE;
    pub fn SetInformationJobObject(
        h_job: HANDLE,
        job_object_information_class: i32,
        lp_job_object_information: *mut c_void,
        cb_job_object_information_length: DWORD,
    ) -> BOOL;
    pub fn AssignProcessToJobObject(h_job: HANDLE, h_process: HANDLE) -> BOOL;
    pub fn GetConsoleWindow() -> HWND;
    pub fn AllocConsole() -> BOOL;
    pub fn AttachConsole(dw_process_id: DWORD) -> BOOL;
    pub fn GetStdHandle(n_std_handle: DWORD) -> HANDLE;
    pub fn SetConsoleOutputCP(w_code_page_id: u32) -> BOOL;
    pub fn SetConsoleTitleW(lp_console_title: *const u16) -> BOOL;
    pub fn LoadLibraryW(lp_file_name: *const u16) -> HINSTANCE;
    pub fn GetProcAddress(h_module: HINSTANCE, lp_proc_name: *const u8) -> *mut c_void;
    pub fn FindResourceW(h_module: HINSTANCE, lp_name: *const u16, lp_type: *const u16) -> HANDLE;
    pub fn EnumResourceNamesW(
        h_module: HINSTANCE,
        lp_type: *const u16,
        lp_name: *const u16,
        lp_enum_func: Option<
            unsafe extern "system" fn(HINSTANCE, *const u16, *const u16, LPARAM) -> BOOL,
        >,
        l_param: LPARAM,
    ) -> BOOL;
    pub fn LoadResource(h_module: HINSTANCE, h_res_info: HANDLE) -> HANDLE;
    pub fn LockResource(h_res_data: HANDLE) -> *mut c_void;
    pub fn SizeofResource(h_module: HINSTANCE, h_res_info: HANDLE) -> DWORD;
}

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(lp_wnd_class: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
        dw_ex_style: DWORD,
        lp_class_name: *const u16,
        lp_window_name: *const u16,
        dw_style: DWORD,
        x: i32,
        y: i32,
        n_width: i32,
        n_height: i32,
        h_wnd_parent: HWND,
        h_menu: HMENU,
        h_instance: HINSTANCE,
        lp_param: *mut c_void,
    ) -> HWND;
    pub fn DefWindowProcW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    pub fn GetMessageW(
        lp_msg: *mut MSG,
        h_wnd: HWND,
        w_msg_filter_min: u32,
        w_msg_filter_max: u32,
    ) -> BOOL;
    pub fn TranslateMessage(lp_msg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(lp_msg: *const MSG) -> LRESULT;
    pub fn PostQuitMessage(n_exit_code: i32);
    pub fn SendMessageW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> LRESULT;
    pub fn PostMessageW(h_wnd: HWND, msg: u32, w_param: WPARAM, l_param: LPARAM) -> BOOL;
    pub fn LoadCursorW(h_instance: HINSTANCE, lp_cursor_name: *const u16) -> HCURSOR;
    pub fn SetTimer(
        h_wnd: HWND,
        n_id_event: usize,
        u_elapse: u32,
        lp_timer_func: *mut c_void,
    ) -> usize;
    pub fn KillTimer(h_wnd: HWND, u_id_event: usize) -> BOOL;
    pub fn GetSystemMetrics(n_index: i32) -> i32;
    pub fn AdjustWindowRectEx(
        lp_rect: *mut RECT,
        dw_style: DWORD,
        b_menu: BOOL,
        dw_ex_style: DWORD,
    ) -> BOOL;
    pub fn ShowWindow(h_wnd: HWND, n_cmd_show: i32) -> BOOL;
    pub fn UpdateWindow(h_wnd: HWND) -> BOOL;
    pub fn GetDC(h_wnd: HWND) -> HDC;
    pub fn ReleaseDC(h_wnd: HWND, h_dc: HDC) -> i32;
    pub fn SetForegroundWindow(h_wnd: HWND) -> BOOL;
    pub fn DestroyWindow(h_wnd: HWND) -> BOOL;
    pub fn EnableWindow(h_wnd: HWND, b_enable: BOOL) -> BOOL;
    pub fn SetFocus(h_wnd: HWND) -> HWND;
    pub fn SetWindowTextW(h_wnd: HWND, lp_string: *const u16) -> BOOL;
    pub fn DrawTextW(
        hdc: HDC,
        lpch_text: *const u16,
        cch_text: i32,
        lprc: *mut RECT,
        format: u32,
    ) -> i32;
    pub fn GetWindowTextW(h_wnd: HWND, lp_string: *mut u16, n_max_count: i32) -> i32;
    pub fn GetWindowTextLengthW(h_wnd: HWND) -> i32;
    pub fn GetClassNameW(h_wnd: HWND, lp_class_name: *mut u16, n_max_count: i32) -> i32;
    pub fn GetClientRect(h_wnd: HWND, lp_rect: *mut RECT) -> BOOL;
    pub fn GetWindowRect(h_wnd: HWND, lp_rect: *mut RECT) -> BOOL;
    pub fn MoveWindow(
        h_wnd: HWND,
        x: i32,
        y: i32,
        n_width: i32,
        n_height: i32,
        b_repaint: BOOL,
    ) -> BOOL;
    pub fn GetSysColor(n_index: i32) -> u32;
    pub fn RedrawWindow(
        h_wnd: HWND,
        lprc_update: *const RECT,
        hrgn_update: *mut c_void,
        flags: u32,
    ) -> BOOL;
    pub fn SetWindowPos(
        h_wnd: HWND,
        h_wnd_insert_after: HWND,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> BOOL;
    pub fn CreateIconFromResourceEx(
        presbits: *const u8,
        dw_res_size: u32,
        f_icon: BOOL,
        dw_ver: u32,
        cx_desired: i32,
        cy_desired: i32,
        flags: u32,
    ) -> HICON;
    pub fn MessageBoxW(
        h_wnd: HWND,
        lp_text: *const u16,
        lp_caption: *const u16,
        u_type: u32,
    ) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateFontW(
        c_height: i32,
        c_width: i32,
        c_escapement: i32,
        c_orientation: i32,
        c_weight: i32,
        b_italic: DWORD,
        b_underline: DWORD,
        b_strike_out: DWORD,
        i_char_set: DWORD,
        i_out_precision: DWORD,
        i_clip_precision: DWORD,
        i_quality: DWORD,
        i_pitch_and_family: DWORD,
        psz_face_name: *const u16,
    ) -> HFONT;
    pub fn DeleteObject(h_object: HANDLE) -> BOOL;
    pub fn GetSysColorBrush(n_index: i32) -> HBRUSH;
    pub fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    pub fn SetTextColor(hdc: HDC, color: u32) -> u32;
    pub fn SetBkColor(hdc: HDC, color: u32) -> u32;
    pub fn GetStockObject(i: i32) -> HANDLE;
    pub fn CreateSolidBrush(color: u32) -> HBRUSH;
    pub fn FillRect(hdc: HDC, lprc: *const RECT, hbr: HBRUSH) -> i32;
    pub fn FrameRect(hdc: HDC, lprc: *const RECT, hbr: HBRUSH) -> i32;
    pub fn Rectangle(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> i32;
    pub fn MoveToEx(hdc: HDC, x: i32, y: i32, lp_point: *mut POINT) -> BOOL;
    pub fn LineTo(hdc: HDC, x: i32, y: i32) -> BOOL;
}

/// Flip the window into (or out of) dark mode for the DWM frame / title bar.
/// `dwmapi` has no import library in this MinGW toolchain, so it is resolved
/// at runtime instead of being linked. Missing OS support is not an error.
pub fn dwm_set_dark(h_wnd: HWND, dark: bool) {
    use std::sync::OnceLock;
    static F: OnceLock<Option<unsafe extern "system" fn(HWND, u32, *const c_void, u32) -> i32>> =
        OnceLock::new();
    let f = F.get_or_init(|| unsafe {
        let mut dll: Vec<u16> = "dwmapi.dll".encode_utf16().collect();
        dll.push(0);
        let h = LoadLibraryW(dll.as_ptr());
        if h.is_null() {
            return None;
        }
        let name = b"DwmSetWindowAttribute\0";
        let p = GetProcAddress(h, name.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute(p))
        }
    });
    if let Some(f) = f {
        let v: i32 = if dark { 1 } else { 0 };
        unsafe {
            f(
                h_wnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &v as *const i32 as *const c_void,
                4,
            );
        }
    }
}

#[link(name = "comctl32")]
extern "system" {
    pub fn InitCommonControlsEx(picce: *const InitCommonControlsEx) -> BOOL;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn OpenProcessToken(
        process_handle: HANDLE,
        desired_access: DWORD,
        token_handle: *mut HANDLE,
    ) -> BOOL;
    pub fn GetTokenInformation(
        token_handle: HANDLE,
        token_information_class: i32,
        token_information: *mut c_void,
        token_information_length: DWORD,
        return_length: *mut DWORD,
    ) -> BOOL;
}

#[link(name = "shell32")]
extern "system" {
    pub fn ShellExecuteExW(p_exec_info: *mut crate::core::ShellExecuteInfoW) -> BOOL;
    pub fn SHChangeNotify(
        w_event_id: i32,
        u_flags: u32,
        dw_item1: *mut c_void,
        dw_item2: *mut c_void,
    );
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoInitializeEx(pv_reserved: *const c_void, dw_co_init: DWORD) -> i32;
}

/// RCDATA resource holding the newline-separated names of the embedded
/// chiptunes. Written by seticon; a single FindResource call replaces
/// EnumResourceNamesW, whose callback marshalling is easy to get wrong.
pub const MUSIC_INDEX: &str = "_music_index";

/// List the chiptunes embedded in this exe. Empty when none were built in.
pub fn music_index() -> Vec<String> {
    let Some(bytes) = resource_bytes(MUSIC_INDEX) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Read an RCDATA resource straight out of this exe's own module. The
/// chiptunes are embedded at build time and played from memory, so nothing is
/// ever unpacked to %TEMP% or anywhere else on disk.
pub fn resource_bytes(name: &str) -> Option<Vec<u8>> {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        if hinst.is_null() {
            return None;
        }
        let n = to_wide(name);
        let ty = RT_RCDATA as usize as *const u16;
        let res = FindResourceW(hinst, n.as_ptr(), ty);
        if res.is_null() {
            return None;
        }
        let size = SizeofResource(hinst, res) as usize;
        if size == 0 {
            return None;
        }
        let hdata = LoadResource(hinst, res);
        if hdata.is_null() {
            return None;
        }
        let ptr = LockResource(hdata) as *const u8;
        if ptr.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr, size).to_vec())
    }
}

// ---------------- small string helpers ----------------

pub fn to_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn raw_valid(h: HANDLE) -> bool {
    !h.is_null() && h as isize != INVALID_HANDLE_VALUE
}

pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}
