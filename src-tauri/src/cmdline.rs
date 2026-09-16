//! 콘솔 압축 진입점, ZipMania.exe c [옵션] <출력> <입력...>, Tauri 전에 창 없이 처리 후 종료
//! 옵션(Bandizip 호환) = -l:# -fmt:zip|7z|tar -v:<크기> -p:<암호> -t:# -aou -aoa -aos -testdst -delsrc -date -y
//! 도움말 = ZipMania.exe c 단독, c -h, c -?, c --help, 또는 -h, -?, --help, /? 단독
//! 종료 코드 = 0 성공 / 1 경고(누락, 테스트 실패, 건너뜀) / 2 오류, 콘솔 출력은 영어만 (D3.18)

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use zipmania_archive::{CompressFormat, CreateOptions, CreateResult, Progress, Router};

/// 콘솔 명령 = argv[1] 이 정확히 "c"
const VERB: &str = "c";

/// 도움말 본문, 표준 출력
const HELP: &str = "\
ZipMania console compression

Usage:
  ZipMania.exe c [options] <output> <input...>

Options:
  -l:0..9          Compression level (default 5)
  -fmt:zip|7z|tar  Format (defaults to the output extension)
  -v:<size>        Volume size for split archives, unit K/M/G (e.g. -v:700M, -v:4GB) -> name.7z.001, .002 ...
  -p:<password>    Password
  -t:<n>           Thread count (7z)
  -aou             Rename to 'name (2)' if the target exists
  -aoa             Overwrite if the target exists (default)
  -aos             Skip if the target exists
  -testdst         Verify the archive after creation
  -delsrc          Delete the sources when verification passes (never when items are missing)
  -date            Replace %Y %y %m %d %H %M %S in the file name with the current time
  -y               Ignored (compatibility)

Example:
  ZipMania.exe c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date \"backup_%y%m%d_%H%M.7z\" \"D:\\Work\"

Exit code:
  0 success / 1 warning (missing items, failed check, skipped) / 2 error
";

/// 백엔드 오류 → 영어 문장, errors.<code> 의 en 번역, 없으면 code 그대로
fn err_en(e: &zipmania_archive::ZipManiaError) -> String {
    let key = format!("errors.{}", e.code);
    let en = zipmania_i18n::lang_index("en");
    zipmania_i18n::STRINGS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| format!("{} ({})", v[en], e.code))
        .unwrap_or_else(|| e.code.clone())
}

/// 도움말 요청인가, c 단독 또는 -h -? --help /? (c 뒤, 또는 단독)
fn wants_help(argv: &[String]) -> bool {
    let is_help = |a: &str| matches!(a, "-h" | "-?" | "--help" | "/?" | "/h");
    match argv.get(1).map(|a| a.as_str()) {
        Some(VERB) => argv.len() == 2 || argv[2..].iter().any(|a| is_help(a)),
        Some(a) => argv.len() == 2 && is_help(a),
        None => false,
    }
}

/// 해석된 명령
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub output: String,
    pub inputs: Vec<String>,
    pub format: Option<CompressFormat>,
    pub level: Option<u8>,
    pub volume: Option<u64>,
    pub password: Option<String>,
    pub threads: Option<u32>,
    pub exists: Exists,
    pub test_dst: bool,
    pub del_src: bool,
    pub date: bool,
}

/// 같은 이름이 있을 때, -aoa 덮어쓰기(기본) / -aou 새 이름 / -aos 건너뜀
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exists {
    Overwrite,
    Rename,
    Skip,
}

/// argv 가 콘솔 명령인가, 아니면 None, 명령이면 해석 결과(오류 = 사용법 위반)
pub fn parse(argv: &[String]) -> Option<Result<Cmd, String>> {
    if argv.get(1).map(|a| a.as_str()) != Some(VERB) {
        return None;
    }
    Some(parse_args(&argv[2..]))
}

fn parse_args(args: &[String]) -> Result<Cmd, String> {
    let mut cmd = Cmd {
        output: String::new(),
        inputs: Vec::new(),
        format: None,
        level: None,
        volume: None,
        password: None,
        threads: None,
        exists: Exists::Overwrite,
        test_dst: false,
        del_src: false,
        date: false,
    };
    let mut positional: Vec<String> = Vec::new();
    for a in args {
        // 옵션 = '-' 로 시작, 단 경로는 첫 위치 인자 이후에도 '-' 로 시작할 수 있으므로 위치 인자가 먼저 오면 옵션으로 보지 않음
        let Some(opt) = a.strip_prefix('-').filter(|_| positional.is_empty() || a.len() > 1) else {
            positional.push(a.clone());
            continue;
        };
        let (key, val) = match opt.split_once(':') {
            Some((k, v)) => (k.to_ascii_lowercase(), Some(v)),
            None => (opt.to_ascii_lowercase(), None),
        };
        match (key.as_str(), val) {
            ("l", Some(v)) => {
                let n: u8 = v.parse().map_err(|_| format!("-l is not a number: {v}"))?;
                if n > 9 {
                    return Err(format!("-l must be 0..9: {v}"));
                }
                cmd.level = Some(n);
            }
            ("fmt", Some(v)) => {
                cmd.format = Some(match v.to_ascii_lowercase().as_str() {
                    "zip" => CompressFormat::Zip,
                    "7z" => CompressFormat::SevenZip,
                    "tar" => CompressFormat::Tar,
                    _ => return Err(format!("-fmt must be zip, 7z or tar: {v}")),
                });
            }
            ("v", Some(v)) => cmd.volume = Some(parse_size(v)?),
            ("p", Some(v)) => cmd.password = Some(v.to_string()),
            ("t", Some(v)) => {
                let n: u32 = v.parse().map_err(|_| format!("-t is not a number: {v}"))?;
                if n == 0 {
                    return Err("-t must be 1 or more".to_string());
                }
                cmd.threads = Some(n);
            }
            ("aou", None) => cmd.exists = Exists::Rename,
            ("aoa", None) => cmd.exists = Exists::Overwrite,
            ("aos", None) => cmd.exists = Exists::Skip,
            ("testdst", None) => cmd.test_dst = true,
            ("delsrc", None) => cmd.del_src = true,
            ("date", None) => cmd.date = true,
            ("y", None) => {}
            _ => return Err(format!("unknown option: {a}")),
        }
    }
    if positional.len() < 2 {
        return Err("usage: ZipMania.exe c [options] <output> <input...> (see --help)".to_string());
    }
    cmd.output = positional.remove(0);
    cmd.inputs = positional;
    Ok(cmd)
}

/// 분할 크기, 숫자 + 단위(b, k, m, g, 뒤에 b 허용, 대소문자 무관), 단위 없음 = 바이트
pub fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim().to_ascii_lowercase();
    let digits_end = t.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_end == 0 {
        return Err(format!("-v is not a size: {s}"));
    }
    let n: u64 = t[..digits_end].parse().map_err(|_| format!("-v is too large: {s}"))?;
    let unit = t[digits_end..].trim_end_matches('b');
    let mul: u64 = match unit {
        "" => 1,
        "k" => 1 << 10,
        "m" => 1 << 20,
        "g" => 1 << 30,
        _ => return Err(format!("-v unit must be b, k, m or g: {s}")),
    };
    let bytes = n.checked_mul(mul).ok_or_else(|| format!("-v is too large: {s}"))?;
    if bytes == 0 {
        return Err("-v must not be 0".to_string());
    }
    Ok(bytes)
}

/// 로컬 시각 (년, 월, 일, 시, 분, 초)
fn local_now() -> (i32, u8, u8, u8, u8, u8) {
    let t = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    (t.year(), t.month() as u8, t.day(), t.hour(), t.minute(), t.second())
}

/// 파일 이름의 %Y %y %m %d %H %M %S 치환, 그 외 % 는 그대로, 폴더 부분은 손대지 않음
pub fn apply_date(output: &str, now: (i32, u8, u8, u8, u8, u8)) -> String {
    let p = Path::new(output);
    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
        return output.to_string();
    };
    let (y, mo, d, h, mi, s) = now;
    let mut out = String::with_capacity(name.len() + 8);
    let mut it = name.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some('Y') => out.push_str(&format!("{y:04}")),
            Some('y') => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{mo:02}")),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('H') => out.push_str(&format!("{h:02}")),
            Some('M') => out.push_str(&format!("{mi:02}")),
            Some('S') => out.push_str(&format!("{s:02}")),
            _ => {
                out.push('%');
                continue;
            }
        }
        it.next();
    }
    p.with_file_name(out).to_string_lossy().to_string()
}

/// 산출 자리 점유 여부, 통짜 파일 또는 분할 첫 볼륨
fn occupied(target: &Path) -> bool {
    target.exists() || zipmania_archive::volumes::volume_name(target, 1).exists()
}

/// -aou, 비어 있는 이름 = <이름> (2).<확장자> 순서로
pub fn unique_name(target: &Path) -> PathBuf {
    if !occupied(target) {
        return target.to_path_buf();
    }
    let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("archive");
    let ext = target.extension().and_then(|s| s.to_str());
    for i in 2.. {
        let name = match ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let cand = target.with_file_name(name);
        if !occupied(&cand) {
            return cand;
        }
    }
    unreachable!()
}

/// 콘솔 명령이면 처리 후 종료 코드, 아니면 None
pub fn run(argv: &[String]) -> Option<i32> {
    let help = wants_help(argv);
    let parsed = if help { None } else { Some(parse(argv)?) };
    let attached = attach_console();
    let code = if help {
        println!("{HELP}");
        0
    } else {
        match parsed.unwrap_or_else(|| Err(String::new())) {
            Ok(cmd) => execute(&cmd, &mut |line| println!("{line}")),
            Err(e) => {
                eprintln!("ZipMania: {e}");
                2
            }
        }
    };
    if attached {
        release_console();
    }
    Some(code)
}

/// 명령 실행, say = 진행, 결과 출력(테스트에서 수집)
pub fn execute(cmd: &Cmd, say: &mut dyn FnMut(String)) -> i32 {
    let mut output = cmd.output.clone();
    if cmd.date {
        output = apply_date(&output, local_now());
    }
    let format = cmd
        .format
        .unwrap_or_else(|| CompressFormat::from_str(&zipmania_archive::formats::ext_of(&output)));
    let mut target = PathBuf::from(&output);
    match cmd.exists {
        Exists::Rename => target = unique_name(&target),
        Exists::Skip if occupied(&target) => {
            say(format!("Skipped (already exists): {}", target.display()));
            return 1;
        }
        _ => {}
    }

    let dll = match crate::commands::sevenzip_dll_file() {
        Ok(d) => d,
        Err(e) => {
            say(format!("Error: 7z.dll not found ({e:?})"));
            return 2;
        }
    };
    let router = Router::new(dll);
    let format_str = match format {
        CompressFormat::Zip => "zip",
        CompressFormat::SevenZip => "7z",
        CompressFormat::Tar => "tar",
    };
    let opts = CreateOptions {
        output: target.to_string_lossy().to_string(),
        inputs: cmd.inputs.clone(),
        format,
        level: cmd.level.unwrap_or(5),
        password: cmd.password.clone(),
        encrypt_names: false,
        volume: cmd.volume,
        threads: cmd.threads,
    };

    say(format!("Compressing: {}", target.display()));
    let mut last_pct = u32::MAX;
    let mut progress = |p: Progress| {
        if p.total > 0 {
            let pct = ((p.done.min(p.total) * 100) / p.total) as u32;
            if pct / 10 != last_pct / 10 {
                last_pct = pct;
                say(format!("  {pct}%"));
            }
        }
    };
    let result = router.for_format(format_str).create(
        &opts,
        &mut progress,
        Arc::new(AtomicBool::new(false)),
    );
    let (status, missing) = match result {
        CreateResult::Done { status, missing, missing_total } => {
            for m in &missing {
                say(format!("  missing: {} ({:?})", m.path, m.reason));
            }
            if missing_total > missing.len() {
                say(format!("  ... and {} more missing", missing_total - missing.len()));
            }
            (status, missing_total)
        }
        CreateResult::Failed(e) => {
            say(format!("Error: {}", err_en(&e)));
            return 2;
        }
    };

    // 실제 산출 경로, 분할이면 .001(볼륨 1개로 끝나면 통짜)
    let produced = if target.exists() {
        target.clone()
    } else {
        zipmania_archive::volumes::volume_name(&target, 1)
    };
    let mut code = if status == "ok" { 0 } else { 1 };

    if cmd.test_dst {
        let p = produced.to_string_lossy().to_string();
        match router.for_archive(&p).test(&p, cmd.password.as_deref()) {
            Ok(()) => say("Test: OK".to_string()),
            Err(e) => {
                say(format!("Test failed: {}", err_en(&e)));
                code = code.max(1);
                return code;
            }
        }
    }

    // 원본 삭제 = 전부 담겼고(ok) 테스트(있다면)를 통과했을 때만
    if cmd.del_src {
        if code == 0 && missing == 0 {
            for i in &cmd.inputs {
                let p = Path::new(i);
                let r = if p.is_dir() {
                    std::fs::remove_dir_all(p)
                } else {
                    std::fs::remove_file(p)
                };
                if let Err(e) = r {
                    say(format!("Delete failed: {i}: {e}"));
                    code = 1;
                }
            }
            if code == 0 {
                say("Sources deleted".to_string());
            }
        } else {
            say("Sources kept (items missing or check failed)".to_string());
        }
    }

    say(format!(
        "{}: {}",
        if code == 0 { "Done" } else { "Warning" },
        produced.display()
    ));
    code
}

/// 부모 콘솔에 붙임(cmd 에서 실행 시 출력 표시), GUI 서브시스템이라 기본은 콘솔 없음, 반환 = 붙었나
/// 출력 코드 페이지 UTF-8, Rust 가 쓰는 바이트가 cp949 콘솔에서 깨지지 않게
#[cfg(windows)]
fn attach_console() -> bool {
    use windows::Win32::System::Console::{AttachConsole, SetConsoleOutputCP, ATTACH_PARENT_PROCESS};
    unsafe {
        let ok = AttachConsole(ATTACH_PARENT_PROCESS).is_ok();
        if ok {
            let _ = SetConsoleOutputCP(65001);
        }
        ok
    }
}

/// 콘솔 입력에 Enter 1회, cmd 는 GUI exe 를 기다리지 않아 프롬프트가 출력 위에 먼저 찍힘
/// → 끝에 Enter 를 넣어 프롬프트를 다시 그리게 함(멈춘 것처럼 보이는 것 방지)
#[cfg(windows)]
fn release_console() {
    use std::io::Write;
    use windows::Win32::System::Console::{
        GetStdHandle, WriteConsoleInputW, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT,
        KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, STD_INPUT_HANDLE,
    };
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        let Ok(h) = GetStdHandle(STD_INPUT_HANDLE) else { return };
        let key = |down: bool| INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: down.into(),
                    wRepeatCount: 1,
                    wVirtualKeyCode: 0x0D,
                    wVirtualScanCode: 0x1C,
                    uChar: KEY_EVENT_RECORD_0 { UnicodeChar: 0x0D },
                    dwControlKeyState: 0,
                },
            },
        };
        let recs = [key(true), key(false)];
        let mut n = 0u32;
        let _ = WriteConsoleInputW(h, &recs, &mut n);
    }
}

#[cfg(not(windows))]
fn attach_console() -> bool {
    false
}

#[cfg(not(windows))]
fn release_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(args: &[&str]) -> Vec<String> {
        std::iter::once("ZipMania.exe")
            .chain(args.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn 콘솔_명령_해석() {
        assert!(parse(&mk(&[])).is_none());
        assert!(parse(&mk(&["--open", "a.zip"])).is_none());
        assert!(parse(&mk(&["C"])).is_none());

        let c = parse(&mk(&[
            "c", "-l:9", "-fmt:7z", "-v:4GB", "-aou", "-testdst", "-delsrc", "-t:4", "-date",
            "D:\\out\\%y%m%d_%H%M.7z", "D:\\src\\a", "D:\\src\\b.txt",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(c.level, Some(9));
        assert_eq!(c.format, Some(CompressFormat::SevenZip));
        assert_eq!(c.volume, Some(4 << 30));
        assert_eq!(c.exists, Exists::Rename);
        assert!(c.test_dst && c.del_src && c.date);
        assert_eq!(c.threads, Some(4));
        assert_eq!(c.output, "D:\\out\\%y%m%d_%H%M.7z");
        assert_eq!(c.inputs, vec!["D:\\src\\a", "D:\\src\\b.txt"]);

        // 사용법 위반
        assert!(parse(&mk(&["c", "out.zip"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-l:10", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-fmt:rar", "o.rar", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-x", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-v:0", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-t:0", "o.zip", "a"])).unwrap().is_err());
    }

    #[test]
    fn 도움말_판정() {
        assert!(wants_help(&mk(&["c"])));
        assert!(wants_help(&mk(&["c", "-h"])));
        assert!(wants_help(&mk(&["c", "-l:9", "--help", "o.zip", "a"])));
        assert!(wants_help(&mk(&["/?"])));
        assert!(wants_help(&mk(&["--help"])));
        assert!(!wants_help(&mk(&[])));
        assert!(!wants_help(&mk(&["--open", "a.zip"])));
        assert!(!wants_help(&mk(&["c", "o.zip", "a"])));
        assert!(!wants_help(&mk(&["-h", "x"])));
        assert!(HELP.contains("-testdst") && HELP.contains("-v:"));
    }

    #[test]
    fn 분할_크기_해석() {
        assert_eq!(parse_size("4GB").unwrap(), 4 << 30);
        assert_eq!(parse_size("700m").unwrap(), 700 << 20);
        assert_eq!(parse_size("100K").unwrap(), 100 << 10);
        assert_eq!(parse_size("12345").unwrap(), 12345);
        assert!(parse_size("4tb").is_err());
        assert!(parse_size("gb").is_err());
        assert!(parse_size("99999999999999999999g").is_err());
    }

    #[test]
    fn 날짜_치환은_파일_이름만() {
        let now = (2026, 9, 16, 14, 5, 7);
        assert_eq!(apply_date("D:\\백업 %y%m%d_%H%M.7z", now), "D:\\백업 260916_1405.7z");
        assert_eq!(apply_date("%Y-%m-%d %H%M%S.zip", now), "2026-09-16 140507.zip");
        assert_eq!(apply_date("%Q %.zip", now), "%Q %.zip");
        assert_eq!(apply_date("a%", now), "a%");
    }

    /// 실물 경로, zip 생성 → -testdst → -delsrc, -aou 두 번째 이름
    #[test]
    fn 콘솔_압축_실행() {
        let root = std::env::temp_dir().join(format!("zm_cmd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("한글.txt"), "hello").unwrap();
        let out = root.join("out.zip");
        let src = root.join("src");

        let mut lines = Vec::new();
        let cmd = parse_args(&[
            "-l:5".to_string(),
            "-testdst".to_string(),
            "-delsrc".to_string(),
            out.to_string_lossy().to_string(),
            src.to_string_lossy().to_string(),
        ])
        .unwrap();
        let code = execute(&cmd, &mut |l| lines.push(l));
        assert_eq!(code, 0, "{lines:?}");
        assert!(out.exists());
        assert!(!src.exists(), "원본이 남았다");

        // 같은 이름 + -aou → (2)
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("b.txt"), "x").unwrap();
        let cmd = parse_args(&[
            "-aou".to_string(),
            out.to_string_lossy().to_string(),
            src.to_string_lossy().to_string(),
        ])
        .unwrap();
        let code = execute(&cmd, &mut |_| {});
        assert_eq!(code, 0);
        assert!(root.join("out (2).zip").exists());
        assert!(src.exists(), "-delsrc 없이 원본이 지워졌다");

        // -aos → 건너뜀 = 1
        let cmd = parse_args(&[
            "-aos".to_string(),
            out.to_string_lossy().to_string(),
            src.to_string_lossy().to_string(),
        ])
        .unwrap();
        assert_eq!(execute(&cmd, &mut |_| {}), 1);

        // 분할 zip, 1KiB 볼륨, 압축 안 되는 입력
        let mut noise = vec![0u8; 5000];
        let mut x: u32 = 7;
        for b in noise.iter_mut() {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *b = (x >> 16) as u8;
        }
        std::fs::write(src.join("n.bin"), &noise).unwrap();
        let sp = root.join("split.zip");
        let cmd = parse_args(&[
            "-v:1k".to_string(),
            "-l:0".to_string(),
            "-testdst".to_string(),
            sp.to_string_lossy().to_string(),
            src.join("n.bin").to_string_lossy().to_string(),
        ])
        .unwrap();
        let mut lines = Vec::new();
        let code = execute(&cmd, &mut |l| lines.push(l));
        assert_eq!(code, 0, "{lines:?}");
        assert!(!sp.exists());
        assert!(zipmania_archive::volumes::volume_name(&sp, 5).exists());
        assert!(lines.iter().any(|l| l == "Test: OK"), "{lines:?}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
