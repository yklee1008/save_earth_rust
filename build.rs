use std::fs;
use std::path::Path;

fn main() {
    // 1. 관련 파일이 변경되었을 때만 build.rs를 다시 실행하도록 Cargo에 알림
    println!("cargo:rerun-if-changed=app_icon.ico");
    println!("cargo:rerun-if-changed=myearth.png");
    println!("cargo:rerun-if-changed=build.rs");

    // 2. 윈도우즈 아이콘(.ico) 빌드 설정
    if Path::new("app_icon.ico").exists() {
        let mut res = winres::WindowsResource::new();
        res.set_icon("app_icon.ico");
        let _ = res.compile();
    }

    // 3. myearth.png 복사 로직 (폴더 생성 보장 및 안전한 처리)
    if Path::new("myearth.png").exists() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let src_path = Path::new(&manifest_dir).join("myearth.png");

        // target/debug 및 target/release 디렉터리가 없을 경우를 대비해 생성 후 복사
        for profile in &["debug", "release"] {
            let dest_dir = Path::new(&manifest_dir).join("target").join(profile);
            if let Ok(_) = fs::create_dir_all(&dest_dir) {
                let _ = fs::copy(&src_path, dest_dir.join("myearth.png"));
            }
        }
    }
}