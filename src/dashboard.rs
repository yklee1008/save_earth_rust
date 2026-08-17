use crate::power::PresetConfig;
use eframe::egui;
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct MonitorData {
    pub cpu_usage: f64,
    pub cpu_power: f64,
    pub gpu_usage: f64,
    pub gpu_power: f64,
    pub gpu_temp: u32,
    pub balanced_count: u32,
    pub ultimate_count: u32,
    pub balanced_kwh: f64,
    pub ultimate_kwh: f64,
    pub daily_kwh: f64,
    pub monthly_kwh: f64,
    pub saved_kwh: f64,
    pub daily_cost: f64,
    pub monthly_cost: f64,
    pub balanced_duration: String,
    pub ultimate_duration: String,
}

pub struct DashboardApp {
    is_visible: Arc<AtomicBool>,

    preset: String,
    cpu_threshold: f64,
    gpu_threshold: f64,
    hold_time: u64,

    cpu_usage: f64,
    cpu_power: f64,
    gpu_usage: f64,
    gpu_power: f64,
    balanced_count: u32,
    ultimate_count: u32,
    balanced_kwh: f64,
    ultimate_kwh: f64,
    daily_kwh: f64,
    monthly_kwh: f64,
    saved_kwh: f64,
    daily_cost: f64,
    monthly_cost: f64,
    balanced_duration: String,
    ultimate_duration: String,

    rx: Receiver<MonitorData>,

    save_message: Option<String>,
    message_timer: Option<Instant>,

    last_visible_state: bool,
    
    dual_monitor_active: bool,
}

impl DashboardApp {
    pub fn new(_config_state: Arc<Mutex<()>>, is_visible: Arc<AtomicBool>) -> Self {
        let active_preset_file = "active_preset.json";
        let config_file = "config.json";

        if !Path::new(active_preset_file).exists() {
            let initial_active_data =
                "{\n  \"active_preset\": \"Auto\",\n  \"cpu_threshold\": 25.0,\n  \"gpu_threshold\": 30.0,\n  \"hold_time\": 8\n}";
            let _ = fs::write(active_preset_file, initial_active_data);
        }

        if !Path::new(config_file).exists() {
            let initial_config_data = "{\"custom_cpu\": 60.0, \"custom_gpu\": 60.0}";
            let _ = fs::write(config_file, initial_config_data);
        }

        let (raw_preset, loaded_cpu, loaded_gpu, loaded_hold) = {
            let mut preset = "Auto".to_string();
            let mut cpu = 25.0;
            let mut gpu = 30.0;
            let mut hold = 8u64;

            if Path::new(active_preset_file).exists() {
                if let Ok(content) = fs::read_to_string(active_preset_file) {
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
                            if let Ok(v) = line
                                .split(':')
                                .nth(1)
                                .unwrap_or("25")
                                .trim()
                                .trim_end_matches(',')
                                .parse()
                            {
                                cpu = v;
                            }
                        }
                        if line.contains("gpu_threshold") {
                            if let Ok(v) = line
                                .split(':')
                                .nth(1)
                                .unwrap_or("30")
                                .trim()
                                .trim_end_matches(',')
                                .parse()
                            {
                                gpu = v;
                            }
                        }
                        if line.contains("hold_time") {
                            if let Ok(v) = line
                                .split(':')
                                .nth(1)
                                .unwrap_or("8")
                                .trim()
                                .trim_end_matches(',')
                                .parse()
                            {
                                hold = v;
                            }
                        }
                    }
                }
            }
            (preset, cpu, gpu, hold)
        };

        let preset = raw_preset
            .trim_matches(|c| c == '"' || c == '\'' || c == '}' || c == '{' || c == ' ')
            .to_string();
        let preset = if preset.is_empty() {
            "Auto".to_string()
        } else {
            preset
        };

        let preset_upper = preset.to_uppercase();
        let (cpu, gpu) = if preset_upper == "CUSTOM" {
            (loaded_cpu, loaded_gpu)
        } else {
            PresetConfig::get_thresholds(&preset_upper)
        };

        let (tx, rx): (Sender<MonitorData>, Receiver<MonitorData>) = channel();
        
        start_data_receiver_thread(tx);

        let loaded_dual_mon_active = std::fs::read_to_string("dual_monitor_state.json")
            .map(|s| s.contains("true"))
            .unwrap_or(true);

                Self {
            is_visible,
            preset,
            cpu_threshold: cpu,
            gpu_threshold: gpu,
            hold_time: loaded_hold,
            cpu_usage: 0.0,
            cpu_power: 0.0,
            gpu_usage: 0.0,
            gpu_power: 0.0,
            balanced_count: 0,
            ultimate_count: 0,
            balanced_kwh: 0.0,
            ultimate_kwh: 0.0,
            daily_kwh: 0.0,
            monthly_kwh: 0.0,
            saved_kwh: 0.0,
            daily_cost: 0.0,
            monthly_cost: 0.0,
            balanced_duration: "00h00m00s".to_string(),
            ultimate_duration: "00h00m00s".to_string(),
            rx,
            save_message: None,
            message_timer: None,
            last_visible_state: true,
            dual_monitor_active: loaded_dual_mon_active,
        }
    }

    fn save_settings(&mut self) {
        let clean_preset = self
            .preset
            .trim_matches(|c| c == '"' || c == '\'' || c == '}' || c == '{' || c == ' ')
            .to_string();
        
        crate::power::save_active_preset_and_config_with_hold(
            &clean_preset,
            self.cpu_threshold,
            self.gpu_threshold,
            self.hold_time,
        );
        crate::power::save_custom_config(self.cpu_threshold, self.gpu_threshold);

        self.save_message = Some("✨ 설정이 저장되었습니다!".to_string());
        self.message_timer = Some(Instant::now());
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. 대시보드 창 닫기 요청 시 즉시 프로세스를 완전 종료하여 RAM을 100% 반환
        if ctx.input(|i| i.viewport().close_requested()) {
            std::process::exit(0);
        }

        let current_visible = self.is_visible.load(Ordering::SeqCst);

        // 2. 비활성화 상태 신호 수신 시 프로세스 완전 종료
        if !current_visible {
            std::process::exit(0);
        }

                if current_visible != self.last_visible_state {
            self.last_visible_state = current_visible;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(current_visible));
            if current_visible {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.request_repaint();
        }

        while let Ok(data) = self.rx.try_recv() {
            self.cpu_usage = data.cpu_usage;
            self.cpu_power = data.cpu_power;
            self.gpu_usage = data.gpu_usage;
            self.gpu_power = data.gpu_power;
            self.balanced_count = data.balanced_count;
            self.ultimate_count = data.ultimate_count;
            self.balanced_kwh = data.balanced_kwh;
            self.ultimate_kwh = data.ultimate_kwh;
            self.daily_kwh = data.daily_kwh;
            self.monthly_kwh = data.monthly_kwh;
            self.saved_kwh = data.saved_kwh;
            self.daily_cost = data.daily_cost;
            self.monthly_cost = data.monthly_cost;
            if !data.balanced_duration.is_empty() {
                self.balanced_duration = data.balanced_duration;
            }
            if !data.ultimate_duration.is_empty() {
                self.ultimate_duration = data.ultimate_duration;
            }
        }

        if let Some(timer) = self.message_timer {
            if timer.elapsed().as_secs() >= 3 {
                self.save_message = None;
                self.message_timer = None;
            }
        }

        ctx.request_repaint_after(Duration::from_secs(1));

        egui::CentralPanel::default().show(ctx, |ui| {
            let data_source_text = if crate::monitor::is_afterburner_active() {
                "Save Earth - Afterburner 연동"
            } else {
                "Save Earth - Windows API 연동"
            };
            ui.heading(data_source_text);
            ui.separator();

            egui::Grid::new("monitoring_grid")
                .num_columns(2)
                .spacing([20.0, 6.0])
                .show(ui, |ui| {
                    let actual_power_mode = crate::power::get_current_power_mode();
                    let mode_str = match actual_power_mode {
                        crate::power::PowerMode::Balanced => "균형 (Balanced)",
                        crate::power::PowerMode::Ultimate => "성능 (Ultimate)",
                    };

                    ui.label("상태:");
                    ui.label(format!("[{}] [전원모드: {}]", self.preset, mode_str));
                    ui.end_row();

                    ui.label("CPU 사용율 / 전력:");
                    ui.label(format!(
                        "{:.1}%  |  사용전력: {:.1}[W]",
                        self.cpu_usage, self.cpu_power
                    ));
                    ui.end_row();

                    ui.label("GPU 사용율 / 전력:");
                    ui.label(format!(
                        "{:.1}%  |  사용전력: {:.1}[W]",
                        self.gpu_usage, self.gpu_power
                    ));
                    ui.end_row();

                    ui.label("균형모드 횟수 / 시간 / 전력량:");
                    ui.label(format!(
                        "{}회  |  {}  |  {:.4} [kWh]",
                        self.balanced_count, self.balanced_duration, self.balanced_kwh
                    ));
                    ui.end_row();

                    ui.label("성능모드 횟수 / 시간 / 전력량:");
                    ui.label(format!(
                        "{}회  |  {}  |  {:.4} [kWh]",
                        self.ultimate_count, self.ultimate_duration, self.ultimate_kwh
                    ));
                    ui.end_row();
                });

            ui.separator();

            let eco = crate::eco::get_eco_metrics();
            let period_text = crate::power::get_accumulation_period_string();
            
            ui.label(period_text);
            ui.add_space(2.0);

            egui::Grid::new("stats_grid")
                .num_columns(2)
                .spacing([20.0, 6.0])
                .show(ui, |ui| {
                    ui.label("일간 / 월간 사용전력량:");
                    ui.label(format!("{:.1} [Wh] / {:.1} [Wh]", self.daily_kwh * 1000.0, self.monthly_kwh * 1000.0));
                    ui.end_row();

                    let calc_daily_cost = self.daily_kwh * 187.9;
                    let calc_monthly_cost = self.monthly_kwh * 187.9;

                    ui.label("일간 / 월간 전력요금:");
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 200, 100),
                        format!("{:.0} [원] / {:.0} [원]", calc_daily_cost, calc_monthly_cost)
                    );
                    ui.end_row();

                    ui.label("추정 절감 전력량:");
                    ui.label(format!("{:.1} [Wh]", eco.saved_wh));
                    ui.end_row();

                    ui.label("탄소 감축량 / 소나무 식재효과:");
                    ui.label(format!(
                        "{:.2} [kg] / {:.2} 그루",
                        eco.carbon_saved_kg, eco.pine_trees
                    ));
                    ui.end_row();
                });

            ui.separator();
            ui.add_space(3.0);

            ui.heading("최적화 프로필 선택");
            let old_preset = self.preset.clone();
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.preset, "Auto".to_string(), "Auto");
                ui.radio_value(&mut self.preset, "Green".to_string(), "Green");
                ui.radio_value(&mut self.preset, "Performance".to_string(), "Performance");
                ui.radio_value(&mut self.preset, "Custom".to_string(), "Custom");
            });

            if old_preset != self.preset {
                let clean_preset = self
                    .preset
                    .trim_matches(|c| c == '"' || c == '\'' || c == '}' || c == '{' || c == ' ')
                    .to_string();
                if self.preset == "Custom" {
                    let (c, g) = PresetConfig::get_thresholds(&old_preset.to_uppercase());
                    self.cpu_threshold = c;
                    self.gpu_threshold = g;
                } else {
                    let (c, g) = PresetConfig::get_thresholds(&self.preset.to_uppercase());
                    self.cpu_threshold = c;
                    self.gpu_threshold = g;
                }
                crate::power::save_active_preset_and_config_with_hold(
                    &clean_preset,
                    self.cpu_threshold,
                    self.gpu_threshold,
                    self.hold_time,
                );
                crate::power::save_custom_config(self.cpu_threshold, self.gpu_threshold);
            }

            ui.add_space(3.0);
            ui.label("사용자 맞춤 임계값 및 홀드 타임 (Custom 모드 전용)");

            let available_width = ui.available_width();
            ui.spacing_mut().slider_width = (available_width - 120.0) * 0.9;

            let is_custom = self.preset == "Custom";
            ui.add_enabled_ui(is_custom, |ui| {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    let cpu_slider = ui.add(
                        egui::Slider::new(&mut self.cpu_threshold, 20.0..=100.0)
                            .text("CPU 임계값 (%)")
                            .step_by(1.0),
                    );
                    let gpu_slider = ui.add(
                        egui::Slider::new(&mut self.gpu_threshold, 20.0..=100.0)
                            .text("GPU 임계값 (%)")
                            .step_by(1.0),
                    );

                    if cpu_slider.changed() || gpu_slider.changed() {
                        crate::power::save_custom_config(
                            self.cpu_threshold,
                            self.gpu_threshold,
                        );
                    }
                });
            });

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                let hold_slider = ui.add(
                    egui::Slider::new(&mut self.hold_time, 4..=30)
                        .text("홀드 타임 (초)")
                        .step_by(1.0),
                );

                if hold_slider.changed() {
                    let clean_preset = self.preset.trim_matches(|c| c == '"' || c == '\'' || c == '}' || c == '{' || c == ' ').to_string();
                    crate::power::save_active_preset_and_config_with_hold(
                        &clean_preset,
                        self.cpu_threshold,
                        self.gpu_threshold,
                        self.hold_time,
                    );
                }
            });

            ui.add_space(2.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_startup = crate::startup::is_registered_in_startup();
                    let startup_btn_text = if is_startup {
                        "시작프로그램 해제"
                    } else {
                        "시작프로그램 등록"
                    };

                    if ui.button(startup_btn_text).clicked() {
                        if is_startup {
                            crate::startup::unregister_from_startup();
                            self.save_message = Some("🗑️ 시작프로그램에서 해제되었습니다.".to_string());
                        } else {
                            crate::startup::register_to_startup();
                            self.save_message = Some("🚀 시작프로그램에 등록되었습니다.".to_string());
                        }
                        self.message_timer = Some(Instant::now());
                    }

                    let dual_monitor_btn_text = if self.dual_monitor_active {
                        "듀얼모니터 제어 해제"
                    } else {
                        "듀얼모니터 제어 활성"
                    };

                    if ui.button(dual_monitor_btn_text).clicked() {
                        self.dual_monitor_active = !self.dual_monitor_active;
                        if self.dual_monitor_active {
                            self.save_message = Some("🖥️ 듀얼모니터 제어가 활성화되었습니다.".to_string());
                        } else {
                            self.save_message = Some("🖥️ 듀얼모니터 제어가 해제되었습니다.".to_string());
                        }
                        self.message_timer = Some(Instant::now());

                        let _ = std::fs::write(
                            "dual_monitor_state.json",
                            format!("{{\"active\": {}}}", self.dual_monitor_active),
                        );

                        if !self.dual_monitor_active {
                            crate::moncon::force_restore_dual_monitor();
                        }
                    }

                    if ui.button("설정 적용 (Apply)").clicked() {
                        self.save_settings();
                    }
                });
            });

            if let Some(msg) = &self.save_message {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(46, 204, 113), msg);
                });
            }   

            ui.add_space(4.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Powered by Gemini A.I with Human Collaboration")
                            .strong()
                            .size(11.0)
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                    ui.label(
                        egui::RichText::new(
                            "윈도우 전원모드를 실시간 제어합니다.\n\
                            개발 경험이 없는 사람이 A.I와 협업하여 만들었습니다.\n\
                            인간은 목적을 밝히고 A.I는 기능을 실현했습니다..\n\
                            기본이 탄탄한 A.I와 함께 살아움직이는 코드를 작성하세요.\n\
                            Human께서는 아이디어와 방향만 제시하시면 됩니다.",
                        )
                        .size(10.0)
                        .color(egui::Color32::GRAY),
                    );
                });
            });
        });
    }
}

fn start_data_receiver_thread(tx: Sender<MonitorData>) {
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        {
            use std::fs::OpenOptions;
            use std::io::{BufRead, BufReader};

            loop {
                if let Ok(file) = OpenOptions::new().read(true).open(r"\\.\pipe\SaveEarth_Data_Pipe") {
                    let reader = BufReader::new(file);
                    for line in reader.lines() {
                        if let Ok(content) = line {
                            if let Some(data) = parse_monitor_json(&content) {
                                if tx.send(data).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    });
}

fn parse_monitor_json(content: &str) -> Option<MonitorData> {
    let mut data = MonitorData::default();
    let content = content.trim_matches(|c| c == '{' || c == '}' || c == ' ');
    for pair in content.split(',') {
        let kv: Vec<&str> = pair.splitn(2, ':').collect();
        if kv.len() == 2 {
            let key = kv[0].trim().trim_matches('"');
            let val = kv[1].trim();
            match key {
                "cpu_usage" => data.cpu_usage = val.parse().unwrap_or(0.0),
                "cpu_power" => data.cpu_power = val.parse().unwrap_or(0.0),
                "gpu_usage" => data.gpu_usage = val.parse().unwrap_or(0.0),
                "gpu_power" => data.gpu_power = val.parse().unwrap_or(0.0),
                "gpu_temp" => data.gpu_temp = val.parse().unwrap_or(0),
                "balanced_count" => data.balanced_count = val.parse().unwrap_or(0),
                "ultimate_count" => data.ultimate_count = val.parse().unwrap_or(0),
                "balanced_kwh" => data.balanced_kwh = val.parse().unwrap_or(0.0),
                "ultimate_kwh" => data.ultimate_kwh = val.parse().unwrap_or(0.0),
                "daily_kwh" => data.daily_kwh = val.parse().unwrap_or(0.0),
                "monthly_kwh" => data.monthly_kwh = val.parse().unwrap_or(0.0),
                "saved_kwh" => data.saved_kwh = val.parse().unwrap_or(0.0),
                "daily_cost" => data.daily_cost = val.parse().unwrap_or(0.0),
                "monthly_cost" => data.monthly_cost = val.parse().unwrap_or(0.0),
                "balanced_duration" => data.balanced_duration = val.trim_matches('"').to_string(),
                "ultimate_duration" => data.ultimate_duration = val.trim_matches('"').to_string(),
                _ => {}
            }
        }
    }
    Some(data)
}

fn start_named_pipe_server(is_visible: Arc<AtomicBool>, egui_ctx: egui::Context) {
    std::thread::spawn(move || {
        #[cfg(target_os = "windows")]
        {
            use std::ptr::null_mut;
            use winapi::um::fileapi::ReadFile;
            use winapi::um::handleapi::CloseHandle;
            use winapi::um::namedpipeapi::{ConnectNamedPipe, CreateNamedPipeW};
            use winapi::um::winbase::{
                PIPE_ACCESS_DUPLEX, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
            };
            use winapi::um::winnt::HANDLE;

            let pipe_name: Vec<u16> = OsStr::new(r"\\.\pipe\SaveEarth_Dashboard_Pipe")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            loop {
                unsafe {
                    let handle: HANDLE = CreateNamedPipeW(
                        pipe_name.as_ptr(),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        1,
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
                            let mut buffer = [0u8; 128];
                            let mut bytes_read = 0;
                            if ReadFile(
                                handle,
                                buffer.as_mut_ptr() as _,
                                buffer.len() as u32,
                                &mut bytes_read,
                                null_mut(),
                            ) != 0
                            {
                                let msg = String::from_utf8_lossy(&buffer[..bytes_read as usize]);
                                                                if msg.contains("SHOW") {
                                    let already_visible = is_visible.load(Ordering::SeqCst);
                                    is_visible.store(true, Ordering::SeqCst);
                                    
                                    if !already_visible {
                                        egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                                        #[cfg(target_os = "windows")]
                                        {
                                            if let Some(monitor) = egui_ctx.input(|i| i.viewport().monitor_size) {
                                                let window_width = 460.0;
                                                let window_height = 610.0;
                                                let center_x = (monitor.x - window_width) / 2.0;
                                                let center_y = (monitor.y - window_height) / 2.0;
                                                if center_x > 0.0 && center_y > 0.0 {
                                                    egui_ctx.send_viewport_cmd(
                                                        egui::ViewportCommand::OuterPosition(
                                                            egui::pos2(center_x, center_y),
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    
                                    egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                                    egui_ctx.request_repaint();
                                } else if msg.contains("QUIT") {
                                    CloseHandle(handle);
                                    std::process::exit(0);
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

pub fn run_dashboard(config_state: Arc<Mutex<()>>, is_visible: Arc<AtomicBool>) {
    let window_width = 460.0;
    let window_height = 610.0;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([window_width, window_height])
            .with_min_inner_size([window_width, window_height])
            .with_max_inner_size([window_width, window_height])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_title("SaveEarth - Dashboard")
            .with_visible(true)
            .with_decorations(true),
        ..Default::default()
    };

    let is_visible_clone = Arc::clone(&is_visible);

    let _ = eframe::run_native(
        "SaveEarth - Dashboard",
        native_options,
        Box::new(move |cc| {
            let ctx = &cc.egui_ctx;

            if let Ok(font_data) = std::fs::read(r"C:\Windows\Fonts\malgun.ttf") {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "MalgunGothic".to_owned(),
                    egui::FontData::from_owned(font_data),
                );
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, "MalgunGothic".to_owned());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .push("MalgunGothic".to_owned());
                ctx.set_fonts(fonts);
            }

            start_named_pipe_server(is_visible_clone, ctx.clone());

            Ok(Box::new(DashboardApp::new(config_state, is_visible)))
        }),
    );
}