use std::fs;
use std::path::Path;
use std::sync::Mutex;
use chrono::{Datelike, Local};

#[link(name = "powrprof")]
extern "system" {
    fn PowerGetActiveScheme(
        user_root_power_key: *mut std::ffi::c_void,
        active_policy_guid: *mut *mut GUID,
    ) -> u32;

    fn PowerSetActiveScheme(
        user_root_power_key: *mut std::ffi::c_void,
        scheme_guid: *const GUID,
    ) -> u32;

    fn LocalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    pub fn to_string_lossy(&self) -> String {
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.data1, self.data2, self.data3,
            self.data4[0], self.data4[1], self.data4[2], self.data4[3],
            self.data4[4], self.data4[5], self.data4[6], self.data4[7]
        )
    }
}

fn parse_guid_string(uuid_str: &str) -> GUID {
    let clean = uuid_str.trim_matches(|c| c == '{' || c == '}');
    let parts: Vec<&str> = clean.split('-').collect();
    
    let data1 = u32::from_str_radix(parts.get(0).unwrap_or(&"0"), 16).unwrap_or(0);
    let data2 = u16::from_str_radix(parts.get(1).unwrap_or(&"0"), 16).unwrap_or(0);
    let data3 = u16::from_str_radix(parts.get(2).unwrap_or(&"0"), 16).unwrap_or(0);
    
    let mut data4 = [0u8; 8];
    if let Some(hex_str) = parts.get(3) {
        if hex_str.len() >= 4 {
            data4[0] = u8::from_str_radix(&hex_str[0..2], 16).unwrap_or(0);
            data4[1] = u8::from_str_radix(&hex_str[2..4], 16).unwrap_or(0);
        }
    }
    if let Some(hex_str) = parts.get(4) {
        if hex_str.len() >= 12 {
            for i in 0..6 {
                data4[2 + i] = u8::from_str_radix(&hex_str[i * 2..(i + 1) * 2], 16).unwrap_or(0);
            }
        }
    }

    GUID { data1, data2, data3, data4 }
}

const BALANCED_GUID: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
const ULTIMATE_GUID: &str = "2be6cd84-0ab5-43a4-97e6-47d2d39f51ad";

const CONFIG_FILE: &str = "config.json";
const ACTIVE_PRESET_FILE: &str = "active_preset.json";
const STATS_FILE: &str = "power_stats.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerMode {
    Balanced,
    Ultimate,
}

struct SessionPowerStats {
    balanced_duration_secs: f64,
    ultimate_duration_secs: f64,
    balanced_energy_kwh: f64,
    ultimate_energy_kwh: f64,
    balanced_count: u32,
    ultimate_count: u32,
    last_applied_mode: Option<PowerMode>,
    last_changed_time: std::time::Instant,
    pending_target_mode: Option<PowerMode>,
}

struct PersistentPowerStats {
    daily_energy_kwh: f64,
    monthly_energy_kwh: f64,
    monthly_saved_energy_kwh: f64,
    daily_cost_krw: f64,
    monthly_cost_krw: f64,
    start_month: u32,
    start_day: u32,
    last_day: u32,
    last_month: u32,
}

static SESSION_STATS: Mutex<Option<SessionPowerStats>> = Mutex::new(None);
static PERSISTENT_STATS: Mutex<Option<PersistentPowerStats>> = Mutex::new(None);

fn ensure_persistent_stats_loaded(persistent_lock: &mut std::sync::MutexGuard<Option<PersistentPowerStats>>) {
    if persistent_lock.is_none() {
        let local_now = Local::now();
        let mut p = PersistentPowerStats {
            daily_energy_kwh: 0.0,
            monthly_energy_kwh: 0.0,
            monthly_saved_energy_kwh: 0.0,
            daily_cost_krw: 0.0,
            monthly_cost_krw: 0.0,
            start_month: local_now.month(),
            start_day: 1,
            last_day: local_now.day(),
            last_month: local_now.month(),
        };
        load_persistent_stats_from_file(&mut p);
        **persistent_lock = Some(p);
    }
}

pub fn get_current_power_mode() -> PowerMode {
    unsafe {
        let mut guid_ptr: *mut GUID = std::ptr::null_mut();
        if PowerGetActiveScheme(std::ptr::null_mut(), &mut guid_ptr) == 0 && !guid_ptr.is_null() {
            let active_guid = (*guid_ptr).to_string_lossy();
            LocalFree(guid_ptr as *mut std::ffi::c_void);

            if active_guid.to_lowercase() == BALANCED_GUID {
                return PowerMode::Balanced;
            }
        }
    }
    PowerMode::Ultimate
}

pub struct PresetConfig;

impl PresetConfig {
    pub fn get_thresholds(preset_name: &str) -> (f64, f64) {
        let upper_name = preset_name.to_uppercase();
        if upper_name == "CUSTOM" {
            return load_custom_config();
        }

        match upper_name.as_str() {
            "AUTO" => (25.0, 30.0),
            "GREEN" => (40.0, 50.0),
            "PERFORMANCE" => (20.0, 25.0),
            _ => (25.0, 30.0),
        }
    }
}

pub fn save_custom_config(cpu: f64, gpu: f64) {
    let json_data = format!("{{\"custom_cpu\": {}, \"custom_gpu\": {}}}", cpu, gpu);
    let _ = fs::write(CONFIG_FILE, json_data);
}

pub fn load_custom_config() -> (f64, f64) {
    if Path::new(CONFIG_FILE).exists() {
        if let Ok(content) = fs::read_to_string(CONFIG_FILE) {
            let mut cpu = 25.0;
            let mut gpu = 30.0;
            for line in content.lines() {
                if line.contains("custom_cpu") {
                    if let Ok(val) = line.split(':').nth(1).unwrap_or("25").trim().trim_end_matches(',').parse::<f64>() {
                        cpu = val;
                    }
                }
                if line.contains("custom_gpu") {
                    if let Ok(val) = line.split(':').nth(1).unwrap_or("30").trim().trim_end_matches(',').parse::<f64>() {
                        gpu = val;
                    }
                }
            }
            return (cpu, gpu);
        }
    }
    (25.0, 30.0)
}

pub fn save_active_preset_and_config_with_hold(preset: &str, cpu: f64, gpu: f64, hold_time: u64) {
    let json_data = format!(
        "{{\n  \"active_preset\": \"{}\",\n  \"cpu_threshold\": {},\n  \"gpu_threshold\": {},\n  \"hold_time\": {}\n}}",
        preset, cpu, gpu, hold_time
    );
    let _ = fs::write(ACTIVE_PRESET_FILE, json_data);
}

pub struct ActivePresetSettings {
    pub preset: String,
    pub cpu_threshold: f64,
    pub gpu_threshold: f64,
    pub hold_time: u64,
}

pub fn load_active_preset_settings() -> ActivePresetSettings {
    let mut preset = "Auto".to_string();
    let mut hold_time = 8;
    let mut loaded_cpu = 25.0;
    let mut loaded_gpu = 30.0;

    if Path::new(ACTIVE_PRESET_FILE).exists() {
        if let Ok(content) = fs::read_to_string(ACTIVE_PRESET_FILE) {
            for line in content.lines() {
                if line.contains("active_preset") {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() > 1 {
                        preset = parts[1]
                            .trim()
                            .trim_matches('"')
                            .trim_end_matches(',')
                            .to_string();
                    }
                }
                if line.contains("cpu_threshold") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("25").trim().trim_end_matches(',').parse() {
                        loaded_cpu = v;
                    }
                }
                if line.contains("gpu_threshold") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("30").trim().trim_end_matches(',').parse() {
                        loaded_gpu = v;
                    }
                }
                if line.contains("hold_time") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("8").trim().trim_end_matches(',').parse() {
                        hold_time = v;
                    }
                }
            }
        }
    }

    let upper_preset = preset.to_uppercase();
    let (cpu_threshold, gpu_threshold) = if upper_preset == "CUSTOM" {
        load_custom_config()
    } else {
        PresetConfig::get_thresholds(&upper_preset)
    };

    ActivePresetSettings {
        preset,
        cpu_threshold,
        gpu_threshold,
        hold_time,
    }
}

pub fn decide_power_mode(cpu_usage: f64, gpu_usage: f64, _preset: &str) -> PowerMode {
    let settings = load_active_preset_settings();
    if cpu_usage >= settings.cpu_threshold || gpu_usage >= settings.gpu_threshold {
        PowerMode::Ultimate
    } else {
        PowerMode::Balanced
    }
}

pub fn decide_power_mode_dynamic(cpu_usage: f64, gpu_usage: f64) -> (PowerMode, u64) {
    let settings = load_active_preset_settings();
    
    let mode = if cpu_usage >= settings.cpu_threshold || gpu_usage >= settings.gpu_threshold {
        PowerMode::Ultimate
    } else {
        PowerMode::Balanced
    };

    (mode, settings.hold_time)
}

pub fn apply_power_mode(target_mode: PowerMode, _current_power_watts: f64, hold_secs: u64) {
    let mut session_lock = SESSION_STATS.lock().unwrap();
    let now = std::time::Instant::now();

    let session = session_lock.get_or_insert_with(|| {
        let initial_mode = get_current_power_mode();
        SessionPowerStats {
            balanced_duration_secs: 0.0,
            ultimate_duration_secs: 0.0,
            balanced_energy_kwh: 0.0,
            ultimate_energy_kwh: 0.0,
            balanced_count: if initial_mode == PowerMode::Balanced { 1 } else { 0 },
            ultimate_count: if initial_mode == PowerMode::Ultimate { 1 } else { 0 },
            last_applied_mode: Some(initial_mode),
            last_changed_time: now,
            pending_target_mode: Some(initial_mode),
        }
    });

    let effective_hold = if hold_secs < 2 { 2 } else { hold_secs };
    let current_actual_mode = get_current_power_mode();

    if session.pending_target_mode != Some(target_mode) {
        session.pending_target_mode = Some(target_mode);
        session.last_changed_time = now;
        return;
    }

    if current_actual_mode == target_mode {
        session.last_changed_time = now;
        return;
    }

    if session.last_changed_time.elapsed() < std::time::Duration::from_secs(effective_hold) {
        return;
    }

    let guid_str = match target_mode {
        PowerMode::Balanced => BALANCED_GUID,
        PowerMode::Ultimate => ULTIMATE_GUID,
    };
    let mut scheme_guid = parse_guid_string(guid_str);

    unsafe {
        let result = PowerSetActiveScheme(std::ptr::null_mut(), &mut scheme_guid);
        if result == 0 {
            match target_mode {
                PowerMode::Balanced => session.balanced_count += 1,
                PowerMode::Ultimate => session.ultimate_count += 1,
            }
            session.last_applied_mode = Some(target_mode);
            session.last_changed_time = now;
        }
    }
}

pub fn tick_accumulate_power_stats(current_power_watts: f64, elapsed_secs: f64) {
    let actual_mode = get_current_power_mode();
    let local_now = Local::now();

    {
        let mut session_lock = SESSION_STATS.lock().unwrap();
        let session = session_lock.get_or_insert_with(|| {
            SessionPowerStats {
                balanced_duration_secs: 0.0,
                ultimate_duration_secs: 0.0,
                balanced_energy_kwh: 0.0,
                ultimate_energy_kwh: 0.0,
                balanced_count: if actual_mode == PowerMode::Balanced { 1 } else { 0 },
                ultimate_count: if actual_mode == PowerMode::Ultimate { 1 } else { 0 },
                last_applied_mode: Some(actual_mode),
                last_changed_time: std::time::Instant::now(),
                pending_target_mode: Some(actual_mode),
            }
        });

        let energy_increment_kwh = (current_power_watts * elapsed_secs) / 3_600_000.0;

        match actual_mode {
            PowerMode::Balanced => {
                session.balanced_duration_secs += elapsed_secs;
                session.balanced_energy_kwh += energy_increment_kwh;
            }
            PowerMode::Ultimate => {
                session.ultimate_duration_secs += elapsed_secs;
                session.ultimate_energy_kwh += energy_increment_kwh;
            }
        }
        session.last_applied_mode = Some(actual_mode);
    }

    {
        let mut persistent_lock = PERSISTENT_STATS.lock().unwrap();
        ensure_persistent_stats_loaded(&mut persistent_lock);

        let persistent = persistent_lock.as_mut().unwrap();

        if persistent.last_day != local_now.day() {
            persistent.daily_energy_kwh = 0.0;
            persistent.daily_cost_krw = 0.0;
            persistent.last_day = local_now.day();
        }

        if persistent.last_month != local_now.month() {
            persistent.monthly_energy_kwh = 0.0;
            persistent.monthly_saved_energy_kwh = 0.0;
            persistent.monthly_cost_krw = 0.0;
            persistent.start_month = local_now.month();
            persistent.start_day = 1;
            persistent.last_month = local_now.month();
        }

        let energy_increment_kwh = (current_power_watts * elapsed_secs) / 3_600_000.0;
        persistent.daily_energy_kwh += energy_increment_kwh;
        persistent.monthly_energy_kwh += energy_increment_kwh;
        
        // 전력량에 비례하여 요금 즉시 반영 (단가: 187.9원 기준)
        let unit_price = 187.9;
        persistent.daily_cost_krw = persistent.daily_energy_kwh * unit_price;
        persistent.monthly_cost_krw = persistent.monthly_energy_kwh * unit_price;
        
        if actual_mode == PowerMode::Balanced {
            persistent.monthly_saved_energy_kwh += energy_increment_kwh * 0.12;
        }

        drop(persistent_lock);
        save_persistent_power_stats();
    }
}

fn format_duration_to_hms(total_secs: f64) -> String {
    let total_secs = total_secs as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{:02}h{:02}m{:02}s", hours, minutes, seconds)
}

pub fn get_mode_durations() -> (String, String) {
    let stats = SESSION_STATS.lock().unwrap();
    if let Some(s) = &*stats {
        (
            format_duration_to_hms(s.balanced_duration_secs),
            format_duration_to_hms(s.ultimate_duration_secs),
        )
    } else {
        ("00h00m00s".to_string(), "00h00m00s".to_string())
    }
}

pub fn get_transition_counts() -> (u32, u32) {
    let stats = SESSION_STATS.lock().unwrap();
    if let Some(s) = &*stats { (s.balanced_count, s.ultimate_count) } else { (0, 0) }
}

pub fn get_mode_energy_kwh() -> (f64, f64) {
    let stats = SESSION_STATS.lock().unwrap();
    if let Some(s) = &*stats { (s.balanced_energy_kwh, s.ultimate_energy_kwh) } else { (0.0, 0.0) }
}

pub fn get_daily_energy_kwh() -> f64 {
    let mut stats = PERSISTENT_STATS.lock().unwrap();
    ensure_persistent_stats_loaded(&mut stats);
    if let Some(s) = &*stats { s.daily_energy_kwh } else { 0.0 }
}

pub fn get_monthly_energy_kwh() -> f64 {
    let mut stats = PERSISTENT_STATS.lock().unwrap();
    ensure_persistent_stats_loaded(&mut stats);
    if let Some(s) = &*stats { s.monthly_energy_kwh } else { 0.0 }
}

pub fn get_daily_cost_krw() -> f64 {
    let mut stats = PERSISTENT_STATS.lock().unwrap();
    ensure_persistent_stats_loaded(&mut stats);
    if let Some(s) = &*stats { s.daily_cost_krw } else { 0.0 }
}

pub fn get_monthly_cost_krw() -> f64 {
    let mut stats = PERSISTENT_STATS.lock().unwrap();
    ensure_persistent_stats_loaded(&mut stats);
    if let Some(s) = &*stats { s.monthly_cost_krw } else { 0.0 }
}

pub fn get_saved_energy_kwh() -> f64 {
    let mut stats = PERSISTENT_STATS.lock().unwrap();
    ensure_persistent_stats_loaded(&mut stats);
    if let Some(s) = &*stats { s.monthly_saved_energy_kwh } else { 0.0 }
}

pub fn get_accumulation_period_string() -> String {
    let local_now = chrono::Local::now();
    let mut stats_lock = PERSISTENT_STATS.lock().unwrap();
    ensure_persistent_stats_loaded(&mut stats_lock);
    let (start_m, start_d) = if let Some(s) = &*stats_lock {
        (s.start_month, s.start_day)
    } else {
        (local_now.month(), 1)
    };

    let current_m = local_now.month();
    let current_d = local_now.day();

    format!("누적기간: {:02}월 {:02}일 ~ {:02}월 {:02}일 (오늘)", start_m, start_d, current_m, current_d)
}

fn save_persistent_power_stats() {
    let lock = PERSISTENT_STATS.lock().unwrap();
    let local_now = Local::now();
    if let Some(stats) = &*lock {
        let json_data = format!(
            "{{\n  \"daily_energy_kwh\": {},\n  \"monthly_energy_kwh\": {},\n  \"monthly_saved_energy_kwh\": {},\n  \"daily_cost_krw\": {},\n  \"monthly_cost_krw\": {},\n  \"start_month\": {},\n  \"start_day\": {},\n  \"last_month\": {}\n}}",
            stats.daily_energy_kwh,
            stats.monthly_energy_kwh,
            stats.monthly_saved_energy_kwh,
            stats.daily_cost_krw,
            stats.monthly_cost_krw,
            stats.start_month,
            stats.start_day,
            local_now.month()
        );
        let _ = fs::write(STATS_FILE, json_data);
    }
}

fn load_persistent_stats_from_file(stats: &mut PersistentPowerStats) {
    let local_now = Local::now();
    if Path::new(STATS_FILE).exists() {
        if let Ok(content) = fs::read_to_string(STATS_FILE) {
            let mut file_month = local_now.month();
            for line in content.lines() {
                if line.contains("daily_energy_kwh") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("0").trim().trim_end_matches(',').parse() { stats.daily_energy_kwh = v; }
                }
                if line.contains("monthly_energy_kwh") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("0").trim().trim_end_matches(',').parse() { stats.monthly_energy_kwh = v; }
                }
                if line.contains("monthly_saved_energy_kwh") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("0").trim().trim_end_matches(',').parse() { stats.monthly_saved_energy_kwh = v; }
                }
                if line.contains("daily_cost_krw") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("0").trim().trim_end_matches(',').parse() { stats.daily_cost_krw = v; }
                }
                if line.contains("monthly_cost_krw") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("0").trim().trim_end_matches(',').parse() { stats.monthly_cost_krw = v; }
                }
                if line.contains("start_month") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("1").trim().trim_end_matches(',').parse() { stats.start_month = v; }
                }
                if line.contains("start_day") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("1").trim().trim_end_matches(',').parse() { stats.start_day = v; }
                }
                if line.contains("last_month") {
                    if let Ok(v) = line.split(':').nth(1).unwrap_or("1").trim().trim_end_matches(',').parse() { file_month = v; }
                }
            }

            if file_month != local_now.month() {
                stats.monthly_energy_kwh = 0.0;
                stats.monthly_saved_energy_kwh = 0.0;
                stats.monthly_cost_krw = 0.0;
                stats.start_month = local_now.month();
                stats.start_day = 1;
            }
        }
    }
    stats.last_month = local_now.month();
    stats.last_day = local_now.day();
}

pub fn reset_power_stats() {
    let local_now = Local::now();

    {
        let mut session_lock = SESSION_STATS.lock().unwrap();
        *session_lock = Some(SessionPowerStats {
            balanced_duration_secs: 0.0,
            ultimate_duration_secs: 0.0,
            balanced_energy_kwh: 0.0,
            ultimate_energy_kwh: 0.0,
            balanced_count: 0,
            ultimate_count: 0,
            last_applied_mode: Some(get_current_power_mode()),
            last_changed_time: std::time::Instant::now(),
            pending_target_mode: Some(get_current_power_mode()),
        });
    }

    let json_data = format!(
        "{{\n  \"daily_energy_kwh\": 0.0,\n  \"monthly_energy_kwh\": 0.0,\n  \"monthly_saved_energy_kwh\": 0.0,\n  \"daily_cost_krw\": 0.0,\n  \"monthly_cost_krw\": 0.0,\n  \"start_month\": {},\n  \"start_day\": {},\n  \"last_month\": {},\n  \"last_day\": {}\n}}",
        local_now.month(),
        local_now.day(),
        local_now.month(),
        local_now.day()
    );
    let _ = fs::write(STATS_FILE, json_data);

    let mut lock = PERSISTENT_STATS.lock().unwrap();
    *lock = Some(PersistentPowerStats {
        daily_energy_kwh: 0.0,
        monthly_energy_kwh: 0.0,
        monthly_saved_energy_kwh: 0.0,
        daily_cost_krw: 0.0,
        monthly_cost_krw: 0.0,
        start_month: local_now.month(),
        start_day: local_now.day(),
        last_day: local_now.day(),
        last_month: local_now.month(),
    });
}

pub fn save_final_power_stats() {
    save_persistent_power_stats();
}