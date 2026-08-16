use std::collections::HashMap;
use std::ffi::CStr;
use winapi::um::handleapi::CloseHandle;
use winapi::um::memoryapi::{MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ};
use winapi::um::winbase::OpenFileMappingA;

const MAX_PATH: usize = 260;
const MAP_NAME: &[u8] = b"MAHMSharedMemory\0";

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MahmSharedMemoryHeader {
    pub dw_signature: u32,
    pub dw_version: u32,
    pub dw_header_size: u32,
    pub dw_num_entries: u32,
    pub dw_entry_size: u32,
    pub time: i32,
    pub dw_num_gpu_entries: u32,
    pub dw_gpu_entry_size: u32,
}

#[repr(C)]
pub struct MahmSharedMemoryEntry {
    pub sz_src_name: [u8; MAX_PATH],
    pub sz_src_units: [u8; MAX_PATH],
    pub sz_localized_src_name: [u8; MAX_PATH],
    pub sz_localized_src_units: [u8; MAX_PATH],
    pub sz_recommended_format: [u8; MAX_PATH],
    pub data: f32,
    pub min_limit: f32,
    pub max_limit: f32,
    pub dw_flags: u32,
    pub dw_gpu: u32,
    pub dw_src_id: u32,
}

#[derive(Debug, Default, Clone)]
pub struct SystemSensors {
    pub sensors: HashMap<String, f32>,
}

impl SystemSensors {
    pub fn fetch_tick() -> Option<Self> {
        let raw_data = unsafe { Self::read_raw_entries()? };
        let mut sensors = HashMap::new();

        for (name, val) in raw_data {
            sensors.insert(name, val);
        }

        Some(SystemSensors { sensors })
    }

    unsafe fn read_raw_entries() -> Option<Vec<(String, f32)>> {
        let h_map = OpenFileMappingA(FILE_MAP_READ, 0, MAP_NAME.as_ptr() as *const i8);
        if h_map.is_null() {
            return None;
        }

        let p_buf = MapViewOfFile(h_map, FILE_MAP_READ, 0, 0, 0);
        if p_buf.is_null() {
            CloseHandle(h_map);
            return None;
        }

        let header = *(p_buf as *const MahmSharedMemoryHeader);
        let base_address = p_buf as usize;
        let entries_offset = header.dw_header_size as usize;

        let mut results = Vec::new();
        for i in 0..header.dw_num_entries {
            let entry_offset = entries_offset + (i as usize * header.dw_entry_size as usize);
            let entry = &*((base_address + entry_offset) as *const MahmSharedMemoryEntry);

            if entry.data == f32::MAX {
                continue;
            }

            let name = CStr::from_ptr(entry.sz_src_name.as_ptr() as *const i8)
                .to_string_lossy()
                .into_owned();

            results.push((name, entry.data));
        }

        UnmapViewOfFile(p_buf);
        CloseHandle(h_map);
        Some(results)
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.sensors.get(name).copied()
    }

    pub fn cpu_usage(&self) -> f32 { self.get("CPU usage").unwrap_or(0.0) }
    pub fn cpu_power(&self) -> f32 { self.get("CPU power").unwrap_or(0.0) }
    pub fn cpu_temp(&self) -> f32 { self.get("CPU temperature").unwrap_or(0.0) }
    pub fn gpu_usage(&self) -> f32 { self.get("GPU usage").unwrap_or(0.0) }
    pub fn gpu_power(&self) -> f32 { self.get("Power").unwrap_or(0.0) }
    pub fn gpu_temp(&self) -> f32 { self.get("GPU temperature").unwrap_or(0.0) }
}

// 기존 AfterburnerSampler 호환용 구조체 및 메서드 제공
pub struct AfterburnerSampler;

impl AfterburnerSampler {
    pub fn new() -> Self {
        Self
    }

    // 반환 순서: (cpu_usage, cpu_power, gpu_usage, gpu_power)
    pub fn sample(&self) -> (f64, f64, f64, f64) {
        if let Some(sensors) = SystemSensors::fetch_tick() {
            (
                sensors.cpu_usage() as f64,
                sensors.cpu_power() as f64,
                sensors.gpu_usage() as f64,
                sensors.gpu_power() as f64,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0)
        }
    }
}