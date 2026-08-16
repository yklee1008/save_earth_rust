use image::{ImageBuffer, Rgba};
use std::process::Command;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

#[cfg(target_os = "windows")]
use std::ffi::OsStr;

static LATEST_CPU_USAGE: AtomicU32 = AtomicU32::new(0);
static LATEST_GPU_USAGE: AtomicU32 = AtomicU32::new(0);

// 파일 읽기 최소화를 위한 캐시 구조체
static PRESET_CACHE: Mutex<Option<(SystemTime, String)>> = Mutex::new(None);

pub fn load_earth_tray_icon() -> Icon {
    let size = 128;
    let image_data = include_bytes!("myearth.png");

    let raw_img = match image::load_from_memory(image_data) {
        Ok(img) => img.to_rgba8(),
        Err(_) => create_fallback_image(size),
    };

    let img_resized = image::imageops::resize(
        &raw_img,
        size,
        size,
        image::imageops::FilterType::Lanczos3,
    );

    let mut circular_img = ImageBuffer::new(size, size);
    let center = size as f32 / 2.0;
    let radius = center - 2.0;

    for (x, y, pixel) in img_resized.enumerate_pixels() {
        let dx = x as f32 - center;
        let dy = y as f32 - center;
        if dx * dx + dy * dy <= radius * radius {
            circular_img.put_pixel(x, y, *pixel);
        } else {
            circular_img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
    }

    let dot_color = Rgba([46, 204, 113, 255]);
    let border_color = Rgba([255, 220, 0, 255]);

    let dot_center_x = 110.0;
    let dot_center_y = 30.0;
    let dot_radius = 32.0;
    let border_thickness = 3.0;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - dot_center_x;
            let dy = y as f32 - dot_center_y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= dot_radius * dot_radius {
                if dist_sq >= (dot_radius - border_thickness) * (dot_radius - border_thickness) {
                    circular_img.put_pixel(x, y, border_color);
                } else {
                    circular_img.put_pixel(x, y, dot_color);
                }
            }
        }
    }

    let final_resized = image::imageops::resize(
        &circular_img,
        32,
        32,
        image::imageops::FilterType::Lanczos3,
    );

    let (width, height) = final_resized.dimensions();
    let rgba_data = final_resized.into_raw();

    Icon::from_rgba(rgba_data, width, height).expect("Failed to create tray icon")
}

fn create_fallback_image(size: u32) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(size, size);
    for (x, _y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 - (size as f32 / 2.0);
        let dy = x as f32 - (size as f32 / 2.0);
        if dx * dx + dy * dy <= (size as f32 / 2.0 - 2.0).powi(2) {
            *pixel = Rgba([30, 144, 255, 255]);
        } else {
            *pixel = Rgba([0, 0, 0, 0]);
        }
    }
    img
}

pub struct TraySystem {
    pub tray_icon: TrayIcon,
    settings_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
    is_visible: Arc<AtomicBool>,
    last_click_time: std::cell::Cell<Instant>,
}

impl TraySystem {
    pub fn new(is_visible: Arc<AtomicBool>) -> Self {
        let icon = load_earth_tray_icon();

        let tray_menu = Menu::new();
        let settings_item = MenuItem::new("설정", true, None);
        let quit_item = MenuItem::new("종료", true, None);

        let _ = tray_menu.append(&settings_item);
        let _ = tray_menu.append(&quit_item);

        let settings_id = settings_item.id().clone();
        let quit_id = quit_item.id().clone();

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_icon(icon)
            .with_tooltip("SaveEarth 2.0")
            .build()
            .unwrap();

        Self {
            tray_icon,
            settings_id,
            quit_id,
            is_visible,
            last_click_time: std::cell::Cell::new(Instant::now() - Duration::from_secs(2)),
        }
    }

    pub fn handle_events<FOpen, FQuit>(&self, mut on_open: FOpen, mut on_quit: FQuit)
    where
        FOpen: FnMut(),
        FQuit: FnMut(),
    {
        let mut try_trigger_open = || {
            let now = Instant::now();
            if now.duration_since(self.last_click_time.get()) < Duration::from_secs(1) {
                return;
            }
            self.last_click_time.set(now);
            on_open();
        };

        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.settings_id {
                try_trigger_open();
            } else if event.id == self.quit_id {
                on_quit();
            }
        }

        if let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick {
                    button: tray_icon::MouseButton::Left,
                    ..
                } => {
                    try_trigger_open();
                }
                _ => {}
            }
        }
    }
}

pub fn start_tray_monitor_server(stop_flag: Arc<AtomicBool>) {
    thread::spawn(move || {
        #[cfg(target_os = "windows")]
        {
            use std::ptr::null_mut;
            use winapi::um::fileapi::WriteFile;
            use winapi::um::handleapi::CloseHandle;
            use winapi::um::namedpipeapi::{ConnectNamedPipe, CreateNamedPipeW};
            use winapi::um::winbase::{
                PIPE_ACCESS_DUPLEX, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
            };
            use winapi::um::winnt::HANDLE;

            let pipe_name: Vec<u16> = OsStr::new(r"\\.\pipe\SaveEarth_Data_Pipe")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let (tx, rx) = std::sync::mpsc::channel();
            crate::monitor::start_monitoring(Arc::clone(&stop_flag), move |data| {
                LATEST_CPU_USAGE.store(data.cpu_usage as u32, Ordering::Relaxed);
                LATEST_GPU_USAGE.store(data.gpu_usage as u32, Ordering::Relaxed);

                let total_power_watts = if data.gpu_power > 0.0 { data.gpu_power } else { 15.0 };
                crate::power::tick_accumulate_power_stats(total_power_watts, 1.0);
                let _ = tx.send(data);
            });

            loop {
                unsafe {
                    let handle: HANDLE = CreateNamedPipeW(
                        pipe_name.as_ptr(),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        255,
                        1024,
                        1024,
                        0,
                        null_mut(),
                    );

                    if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                        let connected = ConnectNamedPipe(handle, null_mut()) != 0
                            || winapi::um::errhandlingapi::GetLastError() == 535 
                            || winapi::um::errhandlingapi::GetLastError() == 536;

                        if connected {
                            while !stop_flag.load(Ordering::Relaxed) {
                                if let Ok(data) = rx.recv_timeout(Duration::from_millis(500)) {
                                    let (balanced_count, ultimate_count) = crate::power::get_transition_counts();
                                    let (balanced_kwh, ultimate_kwh) = crate::power::get_mode_energy_kwh();
                                    let daily_kwh = crate::power::get_daily_energy_kwh();
                                    let monthly_kwh = crate::power::get_monthly_energy_kwh();
                                    let (balanced_duration_str, ultimate_duration_str) = crate::power::get_mode_durations();

                                    let json_bytes = format!(
                                        "{{\"cpu_usage\":{},\"cpu_power\":{},\"gpu_usage\":{},\"gpu_power\":{},\"gpu_temp\":{},\"balanced_count\":{},\"ultimate_count\":{},\"balanced_kwh\":{},\"ultimate_kwh\":{},\"daily_kwh\":{},\"monthly_kwh\":{},\"balanced_duration\":\"{}\",\"ultimate_duration\":\"{}\"}}\n",
                                        data.cpu_usage, data.cpu_power, data.gpu_usage, data.gpu_power, data.gpu_temp,
                                        balanced_count, ultimate_count, balanced_kwh, ultimate_kwh, daily_kwh, monthly_kwh,
                                        balanced_duration_str, ultimate_duration_str
                                    );
                                    let mut bytes_written = 0;
                                    let res = WriteFile(
                                        handle,
                                        json_bytes.as_ptr() as _,
                                        json_bytes.len() as u32,
                                        &mut bytes_written,
                                        null_mut(),
                                    );
                                    if res == 0 {
                                        break;
                                    }
                                }
                            }
                        }
                        CloseHandle(handle);
                    }
                }
            }
        }
    });
}

// active_preset.json 파일을 수정 시에만 읽도록 개선한 함수
fn get_cached_preset_name() -> String {
    let path = std::path::Path::new("active_preset.json");
    if !path.exists() {
        return "Custom".to_string();
    }

    if let Ok(metadata) = path.metadata() {
        if let Ok(modified) = metadata.modified() {
            if let Ok(cache) = PRESET_CACHE.lock() {
                if let Some((cached_time, ref name)) = *cache {
                    if cached_time == modified {
                        return name.clone();
                    }
                }
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                for line in content.lines() {
                    if line.contains("active_preset") {
                        let parts: Vec<&str> = line.split(':').collect();
                        if parts.len() > 1 {
                            let preset_name = parts[1]
                                .trim()
                                .trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == ' ')
                                .to_string();

                            if let Ok(mut cache) = PRESET_CACHE.lock() {
                                *cache = Some((modified, preset_name.clone()));
                            }
                            return preset_name;
                        }
                    }
                }
            }
        }
    }

    "Custom".to_string()
}

pub fn update_tray_tooltip(tray_icon: &TrayIcon) {
    let preset_name = get_cached_preset_name();

    let cpu_usage = LATEST_CPU_USAGE.load(Ordering::Relaxed);
    let gpu_usage = LATEST_GPU_USAGE.load(Ordering::Relaxed);

    let current_power_mode = crate::power::get_current_power_mode();
    let power_mode_name = match current_power_mode {
        crate::power::PowerMode::Balanced => "Balanced",
        crate::power::PowerMode::Ultimate => "Ultimate",
    };

    let tooltip_text: String = format!(
        "SaveEarth 2.0\nPreset: {}\nMode: {}\nCPU: {}%, GPU: {}%",
        preset_name, power_mode_name, cpu_usage, gpu_usage
    );

    let _ = tray_icon.set_tooltip(Some(&tooltip_text));
}

pub fn spawn_dashboard_process(is_visible: Arc<AtomicBool>) {
    #[cfg(target_os = "windows")]
    {
        use std::fs::OpenOptions;
        use std::io::Write;

        if let Ok(mut pipe) = OpenOptions::new()
            .write(true)
            .open(r"\\.\pipe\SaveEarth_Dashboard_Pipe")
        {
            if pipe.write_all(b"SHOW").is_ok() && pipe.flush().is_ok() {
                unsafe {
                    use winapi::um::winuser::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
                    let window_name: Vec<u16> = OsStr::new("SaveEarth - Dashboard")
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect();
                    let hwnd = FindWindowW(std::ptr::null(), window_name.as_ptr());
                    if !hwnd.is_null() {
                        ShowWindow(hwnd, SW_RESTORE);
                        SetForegroundWindow(hwnd);
                    }
                }
                return;
            }
        }
    }

    is_visible.store(true, Ordering::SeqCst);

    if let Ok(current_exe) = std::env::current_exe() {
        let mut cmd = Command::new(&current_exe);
        cmd.arg("--dashboard");

        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(0x00000008 | 0x00000200);
        }

        match cmd.spawn() {
            Ok(_) => {}
            Err(e) => eprintln!("Failed to spawn dashboard process: {}", e),
        }
    }
}

pub fn kill_all_related_processes() {
    #[cfg(target_os = "windows")]
    {
        use std::fs::OpenOptions;
        use std::io::Write;

        if let Ok(mut pipe) = OpenOptions::new()
            .write(true)
            .open(r"\\.\pipe\SaveEarth_Dashboard_Pipe")
        {
            let _ = pipe.write_all(b"QUIT");
            let _ = pipe.flush();
        }

        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(file_name) = current_exe.file_name() {
                if let Some(exe_str) = file_name.to_str() {
                    let _ = Command::new("taskkill")
                        .args(&["/F", "/IM", exe_str])
                        .creation_flags(0x08000000)
                        .output();
                }
            }
        }
    }
    std::process::exit(0);
}