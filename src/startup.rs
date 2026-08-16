use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::um::winuser::{MessageBoxW, IDYES, MB_YESNO, MB_ICONQUESTION};
use winapi::um::winreg::HKEY_CURRENT_USER;
use winapi::um::winnt::{KEY_READ, KEY_WRITE, KEY_WOW64_64KEY, REG_SZ};

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        hKey: *mut std::ffi::c_void,
        lpSubKey: *const u16,
        ulOptions: u32,
        samDesired: u32,
        phkResult: *mut *mut std::ffi::c_void,
    ) -> i32;

    fn RegSetValueExW(
        hKey: *mut std::ffi::c_void,
        lpValueName: *const u16,
        Reserved: u32,
        dwType: u32,
        lpData: *const u8,
        cbData: u32,
    ) -> i32;

    fn RegQueryValueExW(
        hKey: *mut std::ffi::c_void,
        lpValueName: *const u16,
        lpReserved: *mut u32,
        lpType: *mut u32,
        lpData: *mut u8,
        lpcbData: *mut u32,
    ) -> i32;

    fn RegCloseKey(hKey: *mut std::ffi::c_void) -> i32;
}

const REG_RUN_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_REG_NAME: &str = "SaveEarth";
const APP_PROMPTED_FLAG: &str = "SaveEarthPrompted";

fn widestring(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// 이미 시작프로그램에 등록되어 있는지 확인
pub fn is_registered_in_startup() -> bool {
    unsafe {
        let sub_key = widestring(REG_RUN_PATH);
        let mut h_key = ptr::null_mut();

        if RegOpenKeyExW(HKEY_CURRENT_USER as _, sub_key.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut h_key) != 0 {
            return false;
        }

        let value_name = widestring(APP_REG_NAME);
        let mut buf = [0u8; 512];
        let mut buf_len = buf.len() as u32;
        let mut reg_type = REG_SZ;

        let query_result = RegQueryValueExW(h_key, value_name.as_ptr(), ptr::null_mut(), &mut reg_type, buf.as_mut_ptr(), &mut buf_len);
        RegCloseKey(h_key);
        query_result == 0
    }
}

/// 이미 사용자에게 팝업을 띄워 물어본 적이 있는지 확인
pub fn has_already_prompted() -> bool {
    unsafe {
        let sub_key = widestring(REG_RUN_PATH);
        let mut h_key = ptr::null_mut();

        if RegOpenKeyExW(HKEY_CURRENT_USER as _, sub_key.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut h_key) != 0 {
            return false;
        }

        let value_name = widestring(APP_PROMPTED_FLAG);
        let mut buf = [0u8; 32];
        let mut buf_len = buf.len() as u32;
        let mut reg_type = REG_SZ;

        let query_result = RegQueryValueExW(h_key, value_name.as_ptr(), ptr::null_mut(), &mut reg_type, buf.as_mut_ptr(), &mut buf_len);
        RegCloseKey(h_key);
        query_result == 0
    }
}

/// 팝업을 띄웠다는 기록을 레지스트리에 남김
pub fn mark_as_prompted() {
    let sub_key_w = widestring(REG_RUN_PATH);
    unsafe {
        let mut h_key = ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER as _, sub_key_w.as_ptr(), 0, KEY_WRITE | KEY_WOW64_64KEY, &mut h_key) == 0 {
            let value_name = widestring(APP_PROMPTED_FLAG);
            let data = widestring("1");
            let data_bytes = std::slice::from_raw_parts(
                data.as_ptr() as *const u8,
                data.len() * std::mem::size_of::<u16>(),
            );
            RegSetValueExW(h_key, value_name.as_ptr(), 0, REG_SZ, data_bytes.as_ptr(), data_bytes.len() as u32);
            RegCloseKey(h_key);
        }
    }
}

/// 시작프로그램 레지스트리 등록
pub fn register_to_startup() -> bool {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_str) = exe_path.to_str() {
            let formatted_path = format!("\"{}\"", exe_str);
            let sub_key = widestring(REG_RUN_PATH);
            let mut h_key = ptr::null_mut();

            unsafe {
                if RegOpenKeyExW(HKEY_CURRENT_USER as _, sub_key.as_ptr(), 0, KEY_WRITE | KEY_WOW64_64KEY, &mut h_key) != 0 {
                    return false;
                }

                let value_name = widestring(APP_REG_NAME);
                let data = widestring(&formatted_path);
                let data_bytes = std::slice::from_raw_parts(
                    data.as_ptr() as *const u8,
                    data.len() * std::mem::size_of::<u16>(),
                );

                let set_result = RegSetValueExW(h_key, value_name.as_ptr(), 0, REG_SZ, data_bytes.as_ptr(), data_bytes.len() as u32);
                RegCloseKey(h_key);
                return set_result == 0;
            }
        }
    }
    false
}

/// 시작프로그램 레지스트리 등록 해제 (삭제)
pub fn unregister_from_startup() -> bool {
    unsafe {
        let sub_key = widestring(REG_RUN_PATH);
        let mut h_key = ptr::null_mut();

        if RegOpenKeyExW(HKEY_CURRENT_USER as _, sub_key.as_ptr(), 0, KEY_WRITE | KEY_WOW64_64KEY, &mut h_key) != 0 {
            return false;
        }

        let value_name = widestring(APP_REG_NAME);
        
        #[link(name = "advapi32")]
        extern "system" {
            fn RegDeleteValueW(
                hKey: *mut std::ffi::c_void,
                lpValueName: *const u16,
            ) -> i32;
        }

        let delete_result = RegDeleteValueW(h_key, value_name.as_ptr());
        RegCloseKey(h_key);
        delete_result == 0
    }
}

/// 최초 실행 시 딱 한 번만 시작프로그램 등록 여부를 물어봄
pub fn check_and_prompt_startup() {
    if is_registered_in_startup() || has_already_prompted() {
        return;
    }

    mark_as_prompted();

    unsafe {
        let msg = widestring("Save Earth 프로그램을 윈도우 시작프로그램에 등록하시겠습니까?\n(등록하면 컴퓨터 부팅 시 자동으로 실행됩니다.)");
        let title = widestring("Save Earth - 시작프로그램 등록");

        let response = MessageBoxW(
            ptr::null_mut(),
            msg.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        );

        if response == IDYES {
            register_to_startup();
        }
    }
}