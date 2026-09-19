//! zm.exe = 콘솔 서브시스템 실행기, 옆의 ZipMania.exe 를 같은 인자로 띄우고 끝날 때까지 기다린 뒤 종료 코드 전달
//! ZipMania.exe 는 GUI 서브시스템이라 cmd 가 기다리지 않음(출력이 프롬프트 뒤에 찍히고 errorlevel 이 0) → 이 실행기가 그 자리를 메움
//! 인자 없음 = --help, ZIPMANIA_CONSOLE_HOST=1 로 자식이 Enter 주입을 건너뜀 (D3.18)

use std::process::{exit, Command};

fn main() {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("ZipMania.exe")));
    let Some(exe) = exe.filter(|p| p.exists()) else {
        eprintln!("zm: ZipMania.exe not found next to zm.exe");
        exit(2);
    };
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        args.push("--help".into());
    }
    let status = Command::new(&exe)
        .args(&args)
        .env("ZIPMANIA_CONSOLE_HOST", "1")
        .status();
    match status {
        Ok(s) => exit(s.code().unwrap_or(2)),
        Err(e) => {
            eprintln!("zm: cannot start {}: {e}", exe.display());
            exit(2);
        }
    }
}
