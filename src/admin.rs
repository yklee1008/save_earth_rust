use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::um::shellapi::ShellExecuteW;
use winapi::um::winuser::SW_SHOWNORMAL;
use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
use winapi::um::securitybaseapi::GetTokenInformation;
use winapi::um::winnt::{TokenElevation, TOKEN_QUERY, TOKEN_ELEVATION};
use winapi::um::handleapi::CloseHandle;

/// 현재 프로세스가 관리자 권한인지 확인
pub fn is_admin() -> bool {
    unsafe {
        let mut token_handle = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) == 0 {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len = 0;
        let success = GetTokenInformation(
            token_handle,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );

        CloseHandle(token_handle);

        success != 0 && elevation.TokenIsElevated != 0
    }
}

/// 관리자 권한으로 프로그램 다시 실행
// admin.rs

/// 관리자 권한으로 프로그램 다시 실행 (기존 인자 유지)
// src/admin.rs

/// 관리자 권한으로 프로그램 다시 실행 (인자 보존)
pub fn run_as_admin() {
    let exe_path = std::env::current_exe().expect("현재 실행 파일 경로 확인 실패");
    let exe_osstr: &OsStr = exe_path.as_os_str();

    // ⚡ 현재 넘겨받은 인자들을 가져와 전달
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args_str = args.join(" ");
    let args_wide = widestring(&args_str);

    let exe_wide: Vec<u16> = exe_osstr.encode_wide().chain(std::iter::once(0)).collect();

    unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            widestring("runas").as_ptr(),
            exe_wide.as_ptr(),
            args_wide.as_ptr(), // 👈 인자 전달!
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 문자열을 WideString으로 변환
fn widestring(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}
