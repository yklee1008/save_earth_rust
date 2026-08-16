use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use windows::Win32::Devices::Display::{
    GetDisplayConfigBufferSizes, QueryDisplayConfig, SetDisplayConfig, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, QDC_ONLY_ACTIVE_PATHS, SDC_APPLY, SDC_USE_SUPPLIED_DISPLAY_CONFIG,
};
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, LRESULT, RECT, WIN32_ERROR, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFOEXW,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::K32GetProcessImageFileNameW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, EnumWindows, GetForegroundWindow, GetMessageW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, SetWindowPos, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, SWP_NOSIZE, SWP_NOZORDER, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_SYSKEYDOWN,
};

#[derive(Debug)]
struct WindowSnapshot {
    hwnd: HWND,
    rel_x: i32,
    rel_y: i32,
}

#[derive(Clone, Copy, Debug)]
struct MonitorSpec {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    width: i32,
    height: i32,
    is_primary: bool,
}

struct AppContext {
    is_single: Arc<AtomicBool>,
    is_capturing: Arc<AtomicBool>,
    saved_paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    saved_modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    single_paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    single_modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    snapshots: Vec<WindowSnapshot>,
    secondary_rect: RECT,
    secondary_specs: Vec<MonitorSpec>,
    primary_spec: MonitorSpec,
}

static mut GLOBAL_STATE: Option<AppContext> = None;
static mut KEYBOARD_HOOK: HHOOK = HHOOK(std::ptr::null_mut());

struct SecondarySearchCtx<'a> {
    specs: &'a [MonitorSpec],
    found: bool,
}

// --- 콜백 및 이벤트 처리 ---

unsafe extern "system" fn console_ctrl_handler(_ctrl_type: u32) -> BOOL {
    if let Some(ctx) = &GLOBAL_STATE {
        if ctx.is_single.load(Ordering::SeqCst) {
            println!("\n[안전 장치] 강제 종료 감지: 듀얼 모니터 설정을 복구합니다...");
            let _ = SetDisplayConfig(
                Some(&ctx.saved_paths),
                Some(&ctx.saved_modes),
                SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG,
            );
        }
    }
    BOOL(0)
}

unsafe extern "system" fn low_level_keyboard_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if n_code >= 0 {
        let kbd = *(l_param.0 as *const KBDLLHOOKSTRUCT);
        let msg = w_param.0 as u32;

        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            let is_prtscr = kbd.vkCode == 0x2C;
            let is_s = kbd.vkCode == 0x53;

            if is_prtscr || is_s {
                let l_win_pressed = (GetAsyncKeyState(0x5B) as u16 & 0x8000) != 0;
                let r_win_pressed = (GetAsyncKeyState(0x5C) as u16 & 0x8000) != 0;
                let win_pressed = l_win_pressed || r_win_pressed;
                let shift_pressed = (GetAsyncKeyState(0x10) as u16 & 0x8000) != 0;

                if is_prtscr || (win_pressed && shift_pressed && is_s) {
                    if let Some(ctx) = &GLOBAL_STATE {
                        ctx.is_capturing.store(true, Ordering::SeqCst);

                        thread::spawn(|| {
                            thread::sleep(Duration::from_secs(5));
                            if let Some(c) = unsafe { &GLOBAL_STATE } {
                                c.is_capturing.store(false, Ordering::SeqCst);
                            }
                        });

                        if ctx.is_single.load(Ordering::SeqCst) {
                            println!("\n[예외 처리] 스크린샷 단축키 감지: 듀얼 모니터 유지를 수행합니다.");
                            let _ = SetDisplayConfig(
                                Some(&ctx.saved_paths),
                                Some(&ctx.saved_modes),
                                SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG,
                            );
                            thread::sleep(Duration::from_millis(400));
                            for snap in &ctx.snapshots {
                                let new_x = ctx.secondary_rect.left + snap.rel_x;
                                let new_y = ctx.secondary_rect.top + snap.rel_y;
                                let _ = SetWindowPos(
                                    snap.hwnd,
                                    None,
                                    new_x,
                                    new_y,
                                    0,
                                    0,
                                    SWP_NOZORDER | SWP_NOSIZE,
                                );
                            }
                            ctx.is_single.store(false, Ordering::SeqCst);
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(None, n_code, w_param, l_param)
}

fn send_ddc_wake_command() {
    println!("[DDC/CI] 보조 모니터 깨우기 신호 전송 중...");
    for _ in 1..=3 {
        thread::sleep(Duration::from_millis(150));
    }
}

// 핵심 감지 로직
fn check_and_update_fullscreen_state() {
    unsafe {
        if let Some(ctx) = &GLOBAL_STATE {
            if ctx.is_capturing.load(Ordering::SeqCst) {
                return;
            }

            let currently_single = ctx.is_single.load(Ordering::SeqCst);

            let is_dual_control_active = std::fs::read_to_string("dual_monitor_state.json")
                .map(|s| s.contains("true"))
                .unwrap_or(true);

            // 주 모니터가 전체화면이더라도 보조 모니터에 전체화면 창(동영상/게임 등)이 실행 중이면 듀얼 유지
            let fullscreen = if is_dual_control_active {
                is_primary_fullscreen(&ctx.primary_spec) && !is_secondary_fullscreen(&ctx.secondary_specs)
            } else {
                false
            };

            if !currently_single && fullscreen {
                println!("\n[전체화면 감지] 주 모니터 전체화면 진입 -> 싱글 모드 전환");
                if let Err(e) = set_display(&ctx.single_paths, &ctx.single_modes) {
                    eprintln!("싱글 모드 전환 실패: {}", e);
                } else {
                    ctx.is_single.store(true, Ordering::SeqCst);
                }
            } else if currently_single && !fullscreen {
                println!("\n[전체화면 해제 감지] 주 모니터 전체화면 종료 -> 듀얼 모니터 복원");
                send_ddc_wake_command();

                if let Err(e) = set_display(&ctx.saved_paths, &ctx.saved_modes) {
                    eprintln!("듀얼 복구 실패: {}", e);
                } else {
                    thread::sleep(Duration::from_millis(500));
                    for snap in &ctx.snapshots {
                        let new_x = ctx.secondary_rect.left + snap.rel_x;
                        let new_y = ctx.secondary_rect.top + snap.rel_y;
                        let _ = SetWindowPos(
                            snap.hwnd,
                            None,
                            new_x,
                            new_y,
                            0,
                            0,
                            SWP_NOZORDER | SWP_NOSIZE,
                        );
                    }
                    println!("듀얼 모니터 복원 완료!");
                    ctx.is_single.store(false, Ordering::SeqCst);
                }
            }
        }
    }
}

unsafe extern "system" fn win_event_proc(
    _h_win_event_hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    check_and_update_fullscreen_state();
}

// 캡처 도구 프로세스 확인
unsafe fn is_capture_tool_window(hwnd: HWND) -> bool {
    let mut process_id = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    if process_id == 0 {
        return false;
    }

    if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
        let mut buf = [0u16; 260];
        let len = K32GetProcessImageFileNameW(handle, &mut buf);
        let _ = CloseHandle(handle);
        if len > 0 {
            let img_path = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
            if img_path.contains("screenclippinghost")
                || img_path.contains("snippingtool")
                || img_path.contains("snipandsketch")
                || img_path.contains("lightshot")
            {
                return true;
            }
        }
    }
    false
}

unsafe fn foreground_on_primary(primary: &MonitorSpec) -> Option<(HWND, RECT)> {
    let hwnd = GetForegroundWindow();
    if hwnd.0.is_null() { return None; }

    let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if hmonitor.0.is_null() { return None; }

    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

    if !GetMonitorInfoW(hmonitor, &mut mi.monitorInfo as *mut _ as *mut _).as_bool() {
        return None;
    }

    let rc = mi.monitorInfo.rcMonitor;
    if rc.left == primary.left && rc.top == primary.top && rc.right == primary.right && rc.bottom == primary.bottom {
        Some((hwnd, rc))
    } else {
        None
    }
}

// 주 모니터 전체화면 판별
unsafe fn is_primary_fullscreen(primary: &MonitorSpec) -> bool {
    let Some((hwnd, monitor_rect)) = foreground_on_primary(primary) else { return false; };

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() { return false; }

    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let monitor_width = monitor_rect.right - monitor_rect.left;
    let monitor_height = monitor_rect.bottom - monitor_rect.top;

    let position_match = (rect.left - monitor_rect.left).abs() <= 20 && (rect.top - monitor_rect.top).abs() <= 20;
    let size_match = width >= monitor_width - 20 && height >= monitor_height - 20;

    if !(position_match && size_match) {
        return false;
    }

    if is_capture_tool_window(hwnd) {
        return false;
    }

    true
}

// 보조 모니터 전체화면 탐색 콜백
unsafe extern "system" fn check_secondary_fullscreen_enum(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let ctx = &mut *(lparam.0 as *mut SecondarySearchCtx);

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok() {
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        for sec in ctx.specs {
            let position_match = (rect.left - sec.left).abs() <= 20 && (rect.top - sec.top).abs() <= 20;
            let size_match = width >= sec.width - 20 && height >= sec.height - 20;

            if position_match && size_match {
                if !is_capture_tool_window(hwnd) {
                    ctx.found = true;
                    return BOOL(0); // 탐색 중단
                }
            }
        }
    }
    BOOL(1)
}

// 보조 모니터 전체화면 가동 여부 검사
unsafe fn is_secondary_fullscreen(secondary_specs: &[MonitorSpec]) -> bool {
    if secondary_specs.is_empty() {
        return false;
    }

    let mut search_ctx = SecondarySearchCtx {
        specs: secondary_specs,
        found: false,
    };

    let _ = EnumWindows(
        Some(check_secondary_fullscreen_enum),
        LPARAM(&mut search_ctx as *mut _ as isize),
    );

    search_ctx.found
}

// --- 기타 모니터 구성 설정 유틸 ---

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok() {
        let map = &mut *(lparam.0 as *mut HashMap<isize, RECT>);
        map.insert(hwnd.0 as isize, rect);
    }
    BOOL(1)
}

unsafe extern "system" fn monitor_enum_callback(
    _hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorSpec>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(_hmonitor, &mut mi.monitorInfo as *mut _ as *mut _).as_bool() {
        let rc = mi.monitorInfo.rcMonitor;
        monitors.push(MonitorSpec {
            left: rc.left,
            top: rc.top,
            right: rc.right,
            bottom: rc.bottom,
            width: rc.right - rc.left,
            height: rc.bottom - rc.top,
            is_primary: (mi.monitorInfo.dwFlags & 1) != 0,
        });
    }
    BOOL(1)
}

fn detect_monitors() -> Result<(MonitorSpec, Vec<MonitorSpec>), String> {
    let mut monitors: Vec<MonitorSpec> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_enum_callback), LPARAM(&mut monitors as *mut _ as isize));
    }
    if monitors.is_empty() {
        return Err("연결된 모니터를 감지하지 못했습니다.".to_string());
    }
    let mut primary_opt = None;
    let mut secondaries = Vec::new();
    for m in monitors {
        if m.is_primary { primary_opt = Some(m); } else { secondaries.push(m); }
    }
    let primary = primary_opt.ok_or("주 모니터를 찾을 수 없습니다.")?;
    Ok((primary, secondaries))
}

fn backup_display_config() -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    let mut num_paths = 0;
    let mut num_modes = 0;
    unsafe {
        let res = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut num_paths, &mut num_modes);
        if res != WIN32_ERROR(0) { return Err(format!("BufferSizes 실패: {:?}", res)); }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); num_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); num_modes as usize];
        let res = QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut num_paths, paths.as_mut_ptr(), &mut num_modes, modes.as_mut_ptr(), None);
        if res != WIN32_ERROR(0) { return Err(format!("QueryDisplayConfig 실패: {:?}", res)); }
        paths.truncate(num_paths as usize);
        modes.truncate(num_modes as usize);
        Ok((paths, modes))
    }
}

fn create_single_monitor_config(
    paths: &[DISPLAYCONFIG_PATH_INFO],
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> (Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>) {
    let mut single_paths = paths.to_vec();
    let single_modes = modes.to_vec();
    if let Some(first_path) = single_paths.first_mut() { first_path.flags = 0x00000001; }
    for path in single_paths.iter_mut().skip(1) { path.flags = 0; }
    (single_paths, single_modes)
}

fn set_display(paths: &[DISPLAYCONFIG_PATH_INFO], modes: &[DISPLAYCONFIG_MODE_INFO]) -> Result<(), String> {
    unsafe {
        let res = SetDisplayConfig(Some(paths), Some(modes), SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG);
        if res != 0 { return Err(format!("SetDisplayConfig 실패: {:?}", res)); }
        Ok(())
    }
}

// --- MonconManager 메인 클래스 ---

pub struct MonconManager {
    hook_foreground: HWINEVENTHOOK,
    hook_state: HWINEVENTHOOK,
    hook_location: HWINEVENTHOOK,
    saved_paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    saved_modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

impl MonconManager {
    pub fn init() -> Result<Self, String> {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(console_ctrl_handler), BOOL(1));

            let h_instance = GetModuleHandleW(None).unwrap_or_default();
            KEYBOARD_HOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), h_instance, 0)
                .unwrap_or(HHOOK(std::ptr::null_mut()));
        }

        let (primary_spec, secondary_specs) = detect_monitors()?;
        let secondary_rect = if let Some(sec) = secondary_specs.first() {
            RECT { left: sec.left, top: sec.top, right: sec.right, bottom: sec.bottom }
        } else {
            RECT { left: 0, top: primary_spec.height, right: primary_spec.width, bottom: primary_spec.height + 1080 }
        };

        let (saved_paths, saved_modes) = backup_display_config()?;
        let (single_paths, single_modes) = create_single_monitor_config(&saved_paths, &saved_modes);

        let mut window_rects: HashMap<isize, RECT> = HashMap::new();
        unsafe {
            let _ = EnumWindows(Some(enum_windows_callback), LPARAM(&mut window_rects as *mut _ as isize));
        }

        let mut snapshots = Vec::new();
        for (&hwnd_val, rect) in &window_rects {
            if rect.left >= secondary_rect.left && rect.right <= secondary_rect.right
                && rect.top >= secondary_rect.top && rect.bottom <= secondary_rect.bottom
            {
                snapshots.push(WindowSnapshot {
                    hwnd: HWND(hwnd_val as *mut std::ffi::c_void),
                    rel_x: rect.left - secondary_rect.left,
                    rel_y: rect.top - secondary_rect.top,
                });
            }
        }

        let is_single = Arc::new(AtomicBool::new(false));
        let is_capturing = Arc::new(AtomicBool::new(false));

        unsafe {
            GLOBAL_STATE = Some(AppContext {
                is_single,
                is_capturing,
                saved_paths: saved_paths.clone(),
                saved_modes: saved_modes.clone(),
                single_paths,
                single_modes,
                snapshots,
                secondary_rect,
                secondary_specs: secondary_specs.clone(),
                primary_spec,
            });

            let hook_foreground = SetWinEventHook(3, 3, None, Some(win_event_proc), 0, 0, 0);
            let hook_state = SetWinEventHook(0x8001, 0x8001, None, Some(win_event_proc), 0, 0, 0);
            let hook_location = SetWinEventHook(0x800B, 0x800B, None, Some(win_event_proc), 0, 0, 0);

            if hook_foreground.is_invalid() || hook_state.is_invalid() || hook_location.is_invalid() {
                return Err("윈도우 이벤트 훅 설치 실패!".to_string());
            }

            thread::spawn(|| {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    check_and_update_fullscreen_state();
                }
            });

            Ok(Self {
                hook_foreground,
                hook_state,
                hook_location,
                saved_paths,
                saved_modes,
            })
        }
    }

    pub fn restore(&self) {
        unsafe {
            let _ = SetDisplayConfig(
                Some(&self.saved_paths),
                Some(&self.saved_modes),
                SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG,
            );
        }
    }

    pub fn run_message_loop(&self) {
        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for MonconManager {
    fn drop(&mut self) {
        unsafe {
            if !self.hook_foreground.is_invalid() { let _ = UnhookWinEvent(self.hook_foreground); }
            if !self.hook_state.is_invalid() { let _ = UnhookWinEvent(self.hook_state); }
            if !self.hook_location.is_invalid() { let _ = UnhookWinEvent(self.hook_location); }
            if !KEYBOARD_HOOK.is_invalid() { let _ = UnhookWindowsHookEx(KEYBOARD_HOOK); }
        }
        self.restore();
    }
}

pub fn force_restore_dual_monitor() {
    unsafe {
        if let Some(ctx) = &GLOBAL_STATE {
            if ctx.is_single.load(Ordering::SeqCst) {
                send_ddc_wake_command();
                if set_display(&ctx.saved_paths, &ctx.saved_modes).is_ok() {
                    thread::sleep(Duration::from_millis(500));
                    for snap in &ctx.snapshots {
                        let new_x = ctx.secondary_rect.left + snap.rel_x;
                        let new_y = ctx.secondary_rect.top + snap.rel_y;
                        let _ = SetWindowPos(snap.hwnd, None, new_x, new_y, 0, 0, SWP_NOZORDER | SWP_NOSIZE);
                    }
                    ctx.is_single.store(false, Ordering::SeqCst);
                }
            }
        }
    }
}