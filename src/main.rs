#![windows_subsystem = "windows"]
mod admin;
mod dashboard;
mod eco;
mod monitor;
mod power;
mod startup;
mod tray;
mod moncon;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Windows 프로세스 물리 메모리(Working Set) 트림 함수
#[cfg(target_os = "windows")]
fn trim_working_set() {
    unsafe {
        use winapi::um::processthreadsapi::GetCurrentProcess;
        use winapi::um::winbase::SetProcessWorkingSetSize;
        // (SIZE_T)-1을 전달하여 사용하지 않는 메모리 페이지를 OS로 반환
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_dashboard_mode = args.iter().any(|arg| arg == "--dashboard");

    if !admin::is_admin() {
        admin::run_as_admin();
        std::process::exit(0);
    }

    startup::check_and_prompt_startup();

    if is_dashboard_mode {
        let is_visible = Arc::new(AtomicBool::new(true));
        dashboard::run_dashboard(Arc::new(std::sync::Mutex::new(())), is_visible);
    } else {
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        let _moncon_stop = Arc::clone(&stop_flag);
        thread::spawn(move || {
            if let Ok(manager) = moncon::MonconManager::init() {
                println!("Moncon 서비스 시작됨.");
                manager.run_message_loop();
            }
        });

        tray::start_tray_monitor_server(Arc::clone(&stop_flag));

        let stop_flag_for_stats = Arc::clone(&stop_flag);
        thread::spawn(move || {
            while !stop_flag_for_stats.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(1));
            }
            power::save_final_power_stats();
        });

        let is_visible = Arc::new(AtomicBool::new(false));
        let tray_system = tray::TraySystem::new(Arc::clone(&is_visible));
        let mut last_tooltip_update = Instant::now();
        let mut last_trim_time = Instant::now();

        // 초기화 과정(이미지 로딩, 모듈 할당) 후 힙 메모리 물리 반환
        #[cfg(target_os = "windows")]
        trim_working_set();

        loop {
            if last_tooltip_update.elapsed() >= Duration::from_secs(2) {
                tray::update_tray_tooltip(&tray_system.tray_icon);
                last_tooltip_update = Instant::now();
            }

            // 30초마다 잔여 메모리 트림 실행
            if last_trim_time.elapsed() >= Duration::from_secs(30) {
                #[cfg(target_os = "windows")]
                trim_working_set();
                last_trim_time = Instant::now();
            }

            let is_visible_clone = Arc::clone(&is_visible);
            tray_system.handle_events(
                move || { tray::spawn_dashboard_process(Arc::clone(&is_visible_clone)); },
                || { tray::kill_all_related_processes(); },
            );

            #[cfg(target_os = "windows")]
            unsafe {
                use winapi::um::winuser::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG};
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            
            thread::sleep(Duration::from_millis(10));
        }
    }
}