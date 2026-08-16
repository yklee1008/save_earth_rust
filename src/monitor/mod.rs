pub mod afterburner;

use afterburner::AfterburnerSampler;
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use sysinfo::System;

#[derive(Debug, Clone, Copy, Default)]
pub struct MonitorData {
    pub cpu_usage: f64,
    pub cpu_power: f64,
    pub gpu_usage: f64,
    pub gpu_power: f64,
    pub gpu_temp: u32,
}

pub type GpuData = MonitorData;

pub fn is_afterburner_active() -> bool {
    let ab_sampler = AfterburnerSampler::new();
    let (ab_cpu_usage, _, ab_gpu_usage, _) = ab_sampler.sample();
    ab_gpu_usage > 0.0 || ab_cpu_usage > 0.0
}

pub fn start_monitoring<F>(stop_flag: Arc<AtomicBool>, on_update: F)
where
    F: Fn(MonitorData) + Send + 'static,
{
    thread::spawn(move || {
        let ab_sampler = AfterburnerSampler::new();
        let nvml = Nvml::init().ok();
        let mut sys = System::new_all();

        while !stop_flag.load(Ordering::Relaxed) {
            let cpu_usage;
            let cpu_power;
            let mut gpu_usage = 0.0;
            let mut gpu_power = 0.0;
            let mut gpu_temp = 0;

            let (ab_cpu_usage, ab_cpu_power, ab_gpu_usage, ab_gpu_power) = ab_sampler.sample();
            let active = ab_gpu_usage > 0.0 || ab_cpu_usage > 0.0;

            if active {
                cpu_usage = ab_cpu_usage;
                cpu_power = ab_cpu_power;
                gpu_usage = ab_gpu_usage;
                gpu_power = ab_gpu_power;

                if let Some(ref nv) = nvml {
                    if let Ok(device) = nv.device_by_index(0) {
                        gpu_temp = device.temperature(TemperatureSensor::Gpu).unwrap_or(0);
                    }
                }
            } else {
                sys.refresh_cpu();
                cpu_usage = sys.global_cpu_info().cpu_usage() as f64;
                cpu_power = 0.0;

                if let Some(ref nv) = nvml {
                    if let Ok(device) = nv.device_by_index(0) {
                        gpu_temp = device.temperature(TemperatureSensor::Gpu).unwrap_or(0);
                        gpu_usage = device
                            .utilization_rates()
                            .map(|u| u.gpu as f64)
                            .unwrap_or(0.0);
                        gpu_power = device
                            .power_usage()
                            .map(|p| (p as f64) / 1000.0)
                            .unwrap_or(0.0);
                    }
                }
            }

            let total_power_watts = cpu_power + gpu_power;
            crate::power::tick_accumulate_power_stats(total_power_watts, 1.0);

            // [추가] 동적 전원 모드 결정 및 적용 로직
            let (target_mode, hold_time) = crate::power::decide_power_mode_dynamic(cpu_usage, gpu_usage);
            crate::power::apply_power_mode(target_mode, total_power_watts, hold_time);

            on_update(MonitorData {
                cpu_usage,
                cpu_power,
                gpu_usage,
                gpu_power,
                gpu_temp,
            });

            thread::sleep(Duration::from_secs(1));
        }
    });
}