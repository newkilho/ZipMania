//! 명령줄 동사 <a|c|x|e|bx|l|t> [옵션] <아카이브> [항목...], 파서는 하나, 실행은 둘
//! zm 경유(ZIPMANIA_CONSOLE_HOST) = run, Tauri 전에 콘솔로 처리 후 종료, 동사 아닌 첫 인자는 오류
//! ZipMania.exe 직접 = launch_gui, cli.rs 가 부르고 기존 창(압축, 해제, 메인)으로 진행, 콘솔 출력 없음
//! 동사 = a c (압축) x e bx (해제, bx = 아카이브마다 이름 폴더) l (목록) t (테스트), 옵션은 Bandizip 표기(-l:9 -o:dir)와 7-Zip 표기(-mx9 -odir) 둘 다
//! 도움말 = 동사 단독, <동사> -h, -?, --help, 또는 -h, -?, --help, /? 단독
//! 종료 코드 = 0 성공 / 1 경고(누락, 테스트 실패, 건너뜀) / 2 오류, 콘솔 출력은 영어만 (D3.18)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use crate::console::{fmt_bytes, fmt_dur, fmt_size, Report, Term};

use zipmania_archive::{
    CompressFormat, CreateOptions, CreateResult, EditOptions, ExtractOptions, ExtractResult,
    OverwriteMode, Progress, Router,
};

/// 시작 로고, ASCII 만(콘솔 코드 페이지 무관)
const LOGO: &str = r"
 _____  _         __  __                _
|__  / (_) _ __  |  \/  |  __ _  _ __  (_)  __ _
  / /  | || '_ \ | |\/| | / _` || '_ \ | | / _` |
 / /_  | || |_) || |  | || (_| || | | || || (_| |
/____| |_|| .__/ |_|  |_| \__,_||_| |_||_| \__,_|
          |_|
";

/// 로고 아래 구분선
const RULE: &str = "=================================================";

/// 도움말 본문, 표준 출력
const HELP: &str = "\
zm.exe runs commands in the console. ZipMania.exe takes the same commands but shows the
progress in a window instead (like Bandizip.exe / 7zG.exe); ZipMania.exe <file> opens the file.

Usage:
  zm a|c [options] <archive> <input...>    add / create
  zm x|e [options] <archive> [entry...]    extract (x keeps paths, e flattens)
  zm bx  [options] <archive...>            extract each archive into a folder named after it
  zm l   [options] <archive>               list
  zm t   [options] <archive>               test

Options (Bandizip and 7-Zip spellings are both accepted):
  -l:0..9  -mx=0..9  -mx9      Compression level (default 5)
  -fmt:zip|7z|tar  -t7z        Format (defaults to the archive extension)
  -v:<size>  -v<size>          Volume size for split archives, unit K/M/G (e.g. -v:700M, -v4g) -> name.7z.001 ...
  -p:<password>  -p<password>  Password
  -t:<n>  -mmt=<n>             Thread count (7z)
  -o:<dir>  -o<dir>            Extract to this folder (default: current folder)
  -target:auto|name|none       Extract into a subfolder named after the archive: auto = only when the
                               archive has more than one top-level item, name = always (bx default),
                               none = never (default for x and e)
  -aoa  -y                     Overwrite existing files (default when creating)
  -aos                         Skip existing files (default when extracting)
  -aou                         Rename to 'name (2)' when the target exists
  -testdst                     Verify the archive after creation
  -delsrc  -sdel               Delete the sources when verification passes (never when items are missing)
  -date                        Replace %Y %y %m %d %H %M %S in the archive name with the current time
  -r  -bd  -bb*  -bs*  -cp:*   Ignored (compatibility)

Examples:
  zm c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date \"backup_%y%m%d_%H%M.7z\" \"D:\\Work\"
  zm a -mx9 -psecret backup.7z D:\\Work
  zm x -o:D:\\Out -target:auto backup.7z
  zm bx a.zip b.7z
  zm e -y backup.zip docs\\readme.txt

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

/// 동사, argv[1] 정확히 일치(소문자만)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Add,
    Create,
    Extract,
    ExtractFlat,
    ExtractEach,
    List,
    Test,
}

impl Verb {
    fn parse(s: &str) -> Option<Verb> {
        Some(match s {
            "a" => Verb::Add,
            "c" => Verb::Create,
            "x" => Verb::Extract,
            "e" => Verb::ExtractFlat,
            "bx" => Verb::ExtractEach,
            "l" => Verb::List,
            "t" => Verb::Test,
            _ => return None,
        })
    }
}

/// 도움말 요청인가, 동사 단독 또는 -h -? --help /? (동사 뒤, 또는 단독)
fn wants_help(argv: &[String]) -> bool {
    let is_help = |a: &str| matches!(a, "-h" | "-?" | "--help" | "/?" | "/h");
    match argv.get(1).map(|a| a.as_str()) {
        Some(v) if Verb::parse(v).is_some() => argv.len() == 2 || argv[2..].iter().any(|a| is_help(a)),
        Some(a) => argv.len() == 2 && is_help(a),
        None => false,
    }
}

/// 해석된 명령, archive = 첫 위치 인자, items = 나머지(압축 입력, 해제 항목, bx 는 아카이브 더)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub verb: Verb,
    pub archive: String,
    pub items: Vec<String>,
    pub format: Option<CompressFormat>,
    pub level: Option<u8>,
    pub volume: Option<u64>,
    pub password: Option<String>,
    pub threads: Option<u32>,
    pub exists: Option<Exists>,
    pub out_dir: Option<String>,
    pub target: Target,
    pub test_dst: bool,
    pub del_src: bool,
    pub date: bool,
}

/// 같은 이름이 있을 때, -aoa 덮어쓰기 / -aou 새 이름 / -aos 건너뜀
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exists {
    Overwrite,
    Rename,
    Skip,
}

/// 해제 하위 폴더, none = 대상 폴더 그대로 / name = 항상 아카이브 이름 폴더 / auto = 최상위 항목이 둘 이상일 때만
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    None,
    Name,
    Auto,
}

/// argv 가 콘솔 명령인가, 아니면 None, 명령이면 해석 결과(오류 = 사용법 위반)
pub fn parse(argv: &[String]) -> Option<Result<Cmd, String>> {
    let verb = Verb::parse(argv.get(1)?)?;
    Some(parse_args(verb, &argv[2..]))
}

/// 옵션 하나 → (키, 값), -k:v -k=v 는 구분자, -mx9 -o<dir> -p<pw> -v<size> -t<fmt> 는 붙여 쓰기(7-Zip)
fn split_opt(opt: &str) -> (String, Option<String>) {
    // 긴 이름의 옵션은 구분자로만, 그 외는 키 뒤에 값이 바로 붙음(-oD:\x 의 : 은 경로의 일부)
    let lower = opt.to_ascii_lowercase();
    let long = ["target", "testdst"].iter().any(|k| lower.starts_with(k));
    if !long {
        for k in ["mmt", "mx", "o", "p", "v", "t"] {
            if let Some(v) = opt.strip_prefix(k) {
                if let Some(v) = v.strip_prefix([':', '=']).or(Some(v)).filter(|v| !v.is_empty()) {
                    return (k.to_string(), Some(v.to_string()));
                }
            }
        }
    }
    if let Some(i) = opt.find([':', '=']) {
        return (lower[..i].to_string(), Some(opt[i + 1..].to_string()));
    }
    (lower, None)
}

fn parse_args(verb: Verb, args: &[String]) -> Result<Cmd, String> {
    let mut cmd = Cmd {
        verb,
        archive: String::new(),
        items: Vec::new(),
        format: None,
        level: None,
        volume: None,
        password: None,
        threads: None,
        exists: None,
        out_dir: None,
        target: if verb == Verb::ExtractEach { Target::Name } else { Target::None },
        test_dst: false,
        del_src: false,
        date: false,
    };
    let mut positional: Vec<String> = Vec::new();
    let level = |v: &str| -> Result<u8, String> {
        let n: u8 = v.parse().map_err(|_| format!("level is not a number: {v}"))?;
        if n > 9 {
            return Err(format!("level must be 0..9: {v}"));
        }
        Ok(n)
    };
    let format = |v: &str| -> Result<CompressFormat, String> {
        Ok(match v.to_ascii_lowercase().as_str() {
            "zip" => CompressFormat::Zip,
            "7z" => CompressFormat::SevenZip,
            "tar" => CompressFormat::Tar,
            _ => return Err(format!("format must be zip, 7z or tar: {v}")),
        })
    };
    let threads = |v: &str| -> Result<u32, String> {
        let n: u32 = v.parse().map_err(|_| format!("thread count is not a number: {v}"))?;
        if n == 0 {
            return Err("thread count must be 1 or more".to_string());
        }
        Ok(n)
    };
    for a in args {
        // 옵션 = '-' 로 시작, 단 경로는 첫 위치 인자 이후에도 '-' 로 시작할 수 있으므로 위치 인자가 먼저 오면 옵션으로 보지 않음
        let Some(opt) = a.strip_prefix('-').filter(|_| positional.is_empty() || a.len() > 1) else {
            positional.push(a.clone());
            continue;
        };
        let (key, val) = split_opt(opt);
        match (key.as_str(), val.as_deref()) {
            ("l", Some(v)) | ("mx", Some(v)) => cmd.level = Some(level(v)?),
            ("fmt", Some(v)) => cmd.format = Some(format(v)?),
            ("v", Some(v)) => cmd.volume = Some(parse_size(v)?),
            ("p", Some(v)) => cmd.password = Some(v.to_string()),
            ("mmt", Some(v)) => cmd.threads = Some(threads(v)?),
            // -t:4 = 스레드(Bandizip), -t7z = 형식(7-Zip), 숫자면 스레드
            ("t", Some(v)) => {
                if v.bytes().all(|b| b.is_ascii_digit()) {
                    cmd.threads = Some(threads(v)?);
                } else {
                    cmd.format = Some(format(v)?);
                }
            }
            ("o", Some(v)) => cmd.out_dir = Some(v.to_string()),
            ("target", Some(v)) => {
                cmd.target = match v.to_ascii_lowercase().as_str() {
                    "auto" => Target::Auto,
                    "name" => Target::Name,
                    "none" => Target::None,
                    _ => return Err(format!("-target must be auto, name or none: {v}")),
                }
            }
            ("aou", None) => cmd.exists = Some(Exists::Rename),
            ("aoa", None) | ("y", None) => cmd.exists = Some(Exists::Overwrite),
            ("aos", None) => cmd.exists = Some(Exists::Skip),
            ("testdst", None) => cmd.test_dst = true,
            ("delsrc", None) | ("sdel", None) => cmd.del_src = true,
            ("date", None) => cmd.date = true,
            ("r", _) | ("bd", None) | ("cp", Some(_)) => {}
            (k, _) if k.starts_with("bb") || k.starts_with("bs") => {}
            _ => return Err(format!("unknown option: {a}")),
        }
    }
    let need = match verb {
        Verb::Add | Verb::Create => 2,
        _ => 1,
    };
    if positional.len() < need {
        return Err("usage: zm <a|c|x|e|bx|l|t> [options] <archive> [items...] (see --help)".to_string());
    }
    cmd.archive = positional.remove(0);
    cmd.items = positional;
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

/// 콘솔 명령이면 처리 후 종료 코드, 아니면 None, 창은 어떤 경우에도 띄우지 않음(실패는 종료 코드)
pub fn run(argv: &[String]) -> Option<i32> {
    // zm 경유만 콘솔, ZipMania.exe 직접은 cli.rs → launch_gui(창)
    if std::env::var_os("ZIPMANIA_CONSOLE_HOST").is_none() {
        return None;
    }
    let help = wants_help(argv);
    let parsed = if help { None } else { parse(argv) };
    // 동사가 아님 = 사용법 오류, GUI 로 넘기지 않음(검은 창이 GUI 종료까지 남음)
    if !help && parsed.is_none() {
        let mut term = Term::open();
        eprintln!("ZipMania: unknown command: {}", argv.get(1).map(String::as_str).unwrap_or(""));
        eprintln!("          zm takes a command first (zm --help); to open a file in the window use ZipMania.exe <file>");
        term.close();
        return Some(2);
    }
    let mut term = Term::open();
    let code = if help {
        print!("{LOGO}");
        println!("{RULE}");
        println!();
        println!("ZipMania {} command line", env!("CARGO_PKG_VERSION"));
        println!();
        println!("{HELP}");
        0
    } else {
        match parsed.unwrap_or_else(|| Err(String::new())) {
            Ok(cmd) => execute(&cmd, &mut term),
            Err(e) => {
                eprintln!("ZipMania: {e}");
                2
            }
        }
    };
    term.close();
    Some(code)
}

/// 명령 실행, 출력은 Report 로(테스트는 Lines 로 수집)
pub fn execute(cmd: &Cmd, out: &mut dyn Report) -> i32 {
    for l in LOGO.lines() {
        out.line(l);
    }
    out.line(RULE);
    out.line("");
    out.line(&format!("ZipMania {} command line", env!("CARGO_PKG_VERSION")));
    out.line("");
    let dll = match crate::commands::sevenzip_dll_file() {
        Ok(d) => d,
        Err(e) => {
            out.line(&format!("Error: 7z.dll not found ({e:?})"));
            return 2;
        }
    };
    // 해제, 목록, 검사, 추가는 아카이브가 있어야 함, 없으면 백엔드의 뭉뚱그린 io_error 대신 이름을 알림
    let archives: Vec<&String> = if cmd.verb == Verb::ExtractEach {
        std::iter::once(&cmd.archive).chain(cmd.items.iter()).collect()
    } else {
        vec![&cmd.archive]
    };
    if !matches!(cmd.verb, Verb::Create | Verb::Add) {
        if let Some(a) = archives.iter().find(|a| !Path::new(a.as_str()).is_file()) {
            out.line(&format!("Error: archive not found: {a}"));
            return 2;
        }
    }
    let router = Router::new(dll);
    let started = Instant::now();
    let code = match cmd.verb {
        Verb::Add | Verb::Create => run_create(cmd, &router, out),
        Verb::Extract | Verb::ExtractFlat => run_extract(cmd, &router, out),
        // 아카이브마다 따로, 종료 코드는 가장 나쁜 것
        Verb::ExtractEach => archives
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if i > 0 {
                    out.line("");
                }
                let one = Cmd { archive: (*a).clone(), items: Vec::new(), ..cmd.clone() };
                run_extract(&one, &router, out)
            })
            .max()
            .unwrap_or(2),
        Verb::List => run_list(cmd, &router, out),
        Verb::Test => run_test(cmd, &router, out),
    };
    if cmd.verb != Verb::List {
        out.line(&format!("Time: {}", fmt_dur(started.elapsed())));
        out.line("");
        out.line(match code {
            0 => "Everything is OK",
            1 => "Finished with warnings",
            _ => "Failed",
        });
    }
    code
}

/// 진행률 콜백 → Report
fn forward<'a>(out: &'a mut dyn Report) -> impl FnMut(Progress) + 'a {
    move |p: Progress| out.progress(&p)
}

/// 누락 항목 출력, 반환 = 누락 총수
fn report_missing(out: &mut dyn Report, missing: &[zipmania_archive::MissingItem], total: usize) -> usize {
    if total > 0 {
        out.line("");
        out.line(&format!("Warning: {total} item(s) skipped"));
        for m in missing {
            out.line(&format!("  {} ({:?})", m.path, m.reason));
        }
        if total > missing.len() {
            out.line(&format!("  ... and {} more", total - missing.len()));
        }
    }
    total
}

/// 산출물 볼륨 목록(통짜면 1개), 크기 합산용
fn produced_files(target: &Path) -> Vec<PathBuf> {
    if target.exists() {
        return vec![target.to_path_buf()];
    }
    let mut v = Vec::new();
    for i in 1.. {
        let p = zipmania_archive::volumes::volume_name(target, i);
        if !p.exists() {
            break;
        }
        v.push(p);
    }
    v
}

fn total_len(files: &[PathBuf]) -> u64 {
    files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
}

/// 압축(a, c), a 는 대상이 있으면 추가(edit), c 와 -aoa 는 새로 만듦
fn run_create(cmd: &Cmd, router: &Router, out: &mut dyn Report) -> i32 {
    let mut output = cmd.archive.clone();
    if cmd.date {
        output = apply_date(&output, local_now());
    }
    let format = cmd
        .format
        .unwrap_or_else(|| CompressFormat::from_str(&zipmania_archive::formats::ext_of(&output)));
    let mut target = PathBuf::from(&output);
    match cmd.exists.unwrap_or(Exists::Overwrite) {
        Exists::Rename => target = unique_name(&target),
        Exists::Skip if occupied(&target) => {
            out.line(&format!("Skipped, already exists: {}", target.display()));
            return 1;
        }
        _ => {}
    }
    let append = cmd.verb == Verb::Add && cmd.exists.is_none() && target.exists();

    // 입력 요약, 백엔드가 다시 수집하므로 여기서는 표시만
    out.line(&format!("Scanning: {}", cmd.items.join(", ")));
    let (items, _) = zipmania_archive::inputs::collect(&cmd.items);
    let folders = items.iter().filter(|i| i.is_dir).count();
    let files = items.len() - folders;
    let src_bytes: u64 = items.iter().map(|i| i.size).sum();
    out.line(&format!(
        "{folders} folder(s), {files} file(s), {} bytes ({})",
        fmt_bytes(src_bytes),
        fmt_size(src_bytes)
    ));
    out.line("");

    let format_str = match format {
        CompressFormat::Zip => "zip",
        CompressFormat::SevenZip => "7z",
        CompressFormat::Tar => "tar",
    };
    let mut detail = format!("Format: {format_str}");
    if format != CompressFormat::Tar {
        detail.push_str(&format!(", level {}", cmd.level.unwrap_or(5)));
    }
    if let Some(v) = cmd.volume {
        detail.push_str(&format!(", volumes of {}", fmt_size(v)));
    }
    if cmd.password.is_some() {
        detail.push_str(", password");
    }
    if let Some(t) = cmd.threads {
        detail.push_str(&format!(", {t} threads"));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let result = if append {
        out.line(&format!("Updating archive: {}", target.display()));
        out.line(&detail);
        out.line("");
        let opts = EditOptions {
            archive: target.to_string_lossy().to_string(),
            add: cmd.items.clone(),
            remove: Vec::new(),
            password: cmd.password.clone(),
        };
        let p = opts.archive.clone();
        router.for_archive(&p).edit(&opts, &mut forward(out), cancel)
    } else {
        out.line(&format!("Creating archive: {}", target.display()));
        out.line(&detail);
        out.line("");
        let opts = CreateOptions {
            output: target.to_string_lossy().to_string(),
            inputs: cmd.items.clone(),
            format,
            level: cmd.level.unwrap_or(5),
            password: cmd.password.clone(),
            encrypt_names: false,
            volume: cmd.volume,
            threads: cmd.threads,
        };
        router.for_format(format_str).create(&opts, &mut forward(out), cancel)
    };
    out.progress_end();
    let (status, missing) = match result {
        CreateResult::Done { status, missing, missing_total } => {
            (status, report_missing(out, &missing, missing_total))
        }
        CreateResult::Failed(e) => {
            out.line(&format!("Error: {}", err_en(&e)));
            return 2;
        }
    };

    let files = produced_files(&target);
    let produced = files.first().cloned().unwrap_or_else(|| target.clone());
    let arc_bytes = total_len(&files);
    let mut line = format!("Archive size: {} bytes ({})", fmt_bytes(arc_bytes), fmt_size(arc_bytes));
    if files.len() > 1 {
        line.push_str(&format!(", {} volumes", files.len()));
    }
    // 추가(edit)는 아카이브 전체 대비 입력이라 비율이 뜻이 없음
    if src_bytes > 0 && !append {
        line.push_str(&format!(", ratio {:.1}%", arc_bytes as f64 * 100.0 / src_bytes as f64));
    }
    out.line(&line);
    let mut code = if status == "ok" { 0 } else { 1 };

    if cmd.test_dst {
        out.line("");
        out.line(&format!("Testing archive: {}", produced.display()));
        let p = produced.to_string_lossy().to_string();
        match router.for_archive(&p).test(&p, cmd.password.as_deref()) {
            Ok(()) => out.line("Test: OK"),
            Err(e) => {
                out.line(&format!("Test failed: {}", err_en(&e)));
                code = code.max(1);
                return code;
            }
        }
    }

    // 원본 삭제 = 전부 담겼고(ok) 테스트(있다면)를 통과했을 때만
    if cmd.del_src {
        out.line("");
        if code == 0 && missing == 0 {
            for (i, r) in crate::commands::remove_paths(&cmd.items) {
                match r {
                    Ok(()) => out.line(&format!("Deleted: {i}")),
                    Err(e) => {
                        out.line(&format!("Delete failed: {i}: {e}"));
                        code = 1;
                    }
                }
            }
        } else {
            out.line("Sources kept (items missing or check failed)");
        }
    }
    out.line("");
    out.line(&format!("Output: {}", produced.display()));
    code
}

/// 아카이브 이름에서 확장자를 뗀 폴더 이름, 분할 .001 과 .tar.gz 류의 이중 확장자 포함
fn archive_stem(archive: &str) -> String {
    let name = Path::new(archive)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let mut stem = name;
    loop {
        let Some(i) = stem.rfind('.') else { break };
        let ext = stem[i + 1..].to_ascii_lowercase();
        let strip = (ext.len() >= 3 && ext.bytes().all(|b| b.is_ascii_digit()))
            || zipmania_archive::formats::READ_EXTS.contains(&ext.as_str())
            || ext == "tar";
        if !strip || i == 0 {
            break;
        }
        stem = &stem[..i];
    }
    stem.to_string()
}

/// 최상위 항목 수, 목록의 첫 경로 조각을 세어 -target:auto 판정
fn top_level_count(entries: &[zipmania_archive::ArchiveEntry]) -> usize {
    let mut tops: Vec<&str> = entries
        .iter()
        .map(|e| e.path.trim_start_matches(['/', '\\']))
        .map(|p| p.split(['/', '\\']).next().unwrap_or(""))
        .filter(|t| !t.is_empty())
        .collect();
    tops.sort_unstable();
    tops.dedup();
    tops.len()
}

/// 해제(x, e), 대상 = -o 또는 현재 폴더, -target 으로 하위 폴더, 기본 충돌 처리 = 건너뜀

/// 해제(x, e), 대상 = -o 또는 현재 폴더, -target 으로 하위 폴더, 기본 충돌 처리 = 건너뜀
fn run_extract(cmd: &Cmd, router: &Router, out: &mut dyn Report) -> i32 {
    let archive = cmd.archive.clone();
    let backend = router.for_archive(&archive);
    // 대상은 절대 경로로 표시, 상대 -o 는 현재 폴더 기준
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let base = match &cmd.out_dir {
        Some(d) => cwd.join(d),
        None => cwd,
    };
    out.line(&format!("Extracting archive: {archive}"));
    let entries = match backend.list(&archive, cmd.password.as_deref()) {
        Ok(e) => e,
        Err(e) => {
            out.line(&format!("Error: {}", err_en(&e)));
            return 2;
        }
    };
    let folders = entries.iter().filter(|e| e.is_dir).count();
    let files = entries.len() - folders;
    let bytes: u64 = entries.iter().map(|e| e.size).sum();
    out.line(&format!(
        "{folders} folder(s), {files} file(s), {} bytes ({})",
        fmt_bytes(bytes),
        fmt_size(bytes)
    ));
    let subfolder = match cmd.target {
        Target::None => false,
        Target::Name => true,
        Target::Auto => top_level_count(&entries) != 1,
    };
    let dest = if subfolder {
        base.join(archive_stem(&archive))
    } else {
        base
    };
    let (overwrite, mode) = match cmd.exists.unwrap_or(Exists::Skip) {
        Exists::Overwrite => (OverwriteMode::Overwrite, "overwrite"),
        Exists::Rename => (OverwriteMode::Rename, "rename"),
        Exists::Skip => (OverwriteMode::Skip, "skip"),
    };
    out.line(&format!("Destination: {}", dest.display()));
    let mut detail = format!("Existing files: {mode}");
    if !cmd.items.is_empty() {
        detail.push_str(&format!(", {} selected entries", cmd.items.len()));
    }
    if cmd.verb == Verb::ExtractFlat {
        detail.push_str(", paths dropped");
    }
    out.line(&detail);
    out.line("");
    let opts = ExtractOptions {
        archive: archive.clone(),
        dest: dest.to_string_lossy().to_string(),
        keep_paths: cmd.verb != Verb::ExtractFlat,
        overwrite,
        password: cmd.password.clone(),
        selected: cmd.items.clone(),
        decisions: HashMap::new(),
    };
    let result = backend.extract(&opts, &mut forward(out), Arc::new(AtomicBool::new(false)));
    out.progress_end();
    match result {
        ExtractResult::Done { status, missing, missing_total } => {
            report_missing(out, &missing, missing_total);
            out.line(&format!("Output: {}", dest.display()));
            if status == "ok" { 0 } else { 1 }
        }
        ExtractResult::Failed(e) => {
            out.line(&format!("Error: {}", err_en(&e)));
            2
        }
    }
}

/// GUI 모드 오류, 콘솔이 없으므로 메시지 박스(영어), 비-Windows 는 stderr
pub fn gui_error(msg: &str) {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let text = HSTRING::from(msg);
        let title = HSTRING::from("ZipMania");
        unsafe {
            let _ = MessageBoxW(None, &text, &title, MB_OK | MB_ICONERROR);
        }
    }
    #[cfg(not(windows))]
    eprintln!("ZipMania: {msg}");
}

/// ZipMania.exe 직접 실행의 동사 → 기존 창, a c = 압축 창 자동 시작, x e bx = 해제 창 자동 시작, l t = 메인 창에 열기
/// 콘솔 전용 옵션(-t 스레드, -aoa/-aos 해제 충돌 = 창이 물음)은 무시, a 의 기존 아카이브 추가는 zm 만 (D3.18)
pub fn launch_gui(app: &tauri::AppHandle, cmd: Cmd, startup: bool) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let abs = |p: &str| -> String {
        let pb = Path::new(p);
        if pb.is_absolute() {
            p.to_string()
        } else {
            cwd.join(pb).to_string_lossy().to_string()
        }
    };
    match cmd.verb {
        Verb::Add | Verb::Create => {
            let mut output = abs(&cmd.archive);
            if cmd.date {
                output = apply_date(&output, local_now());
            }
            let format = cmd
                .format
                .unwrap_or_else(|| CompressFormat::from_str(&zipmania_archive::formats::ext_of(&output)));
            let mut target = PathBuf::from(&output);
            match cmd.exists.unwrap_or(Exists::Overwrite) {
                Exists::Rename => target = unique_name(&target),
                Exists::Skip if occupied(&target) => {
                    gui_error(&format!("Skipped, already exists: {}", target.display()));
                    return;
                }
                _ => {}
            }
            if cmd.verb == Verb::Add && cmd.exists.is_none() && target.exists() {
                gui_error(&format!(
                    "{} already exists. Adding to an existing archive is a console command: zm a ...",
                    target.display()
                ));
                return;
            }
            let format_str = match format {
                CompressFormat::Zip => "zip",
                CompressFormat::SevenZip => "7z",
                CompressFormat::Tar => "tar",
            };
            let launch = crate::models::CompressLaunch {
                inputs: cmd.items.iter().map(|i| abs(i)).collect(),
                format: Some(format_str.to_string()),
                output: Some(target.to_string_lossy().to_string()),
                auto_start: true,
                batch: Vec::new(),
                level: cmd.level,
                password: cmd.password.clone(),
                volume: cmd.volume,
                verify: cmd.test_dst,
                delete_sources: cmd.del_src,
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::commands::open_compress_launch(app, launch) {
                    gui_error(&e);
                }
            });
        }
        Verb::Extract | Verb::ExtractFlat | Verb::ExtractEach => {
            let base = match &cmd.out_dir {
                Some(d) => PathBuf::from(abs(d)),
                None => cwd.clone(),
            };
            let archives: Vec<String> = if cmd.verb == Verb::ExtractEach {
                std::iter::once(&cmd.archive).chain(cmd.items.iter()).map(|a| abs(a)).collect()
            } else {
                vec![abs(&cmd.archive)]
            };
            if let Some(a) = archives.iter().find(|a| !Path::new(a.as_str()).is_file()) {
                gui_error(&format!("Archive not found: {a}"));
                return;
            }
            // -target:auto 는 목록이 필요, dll 을 못 열면 창이 같은 오류를 내므로 name 으로 진행
            let router = crate::commands::sevenzip_dll_file().ok().map(Router::new);
            let dest_of = |a: &str| -> String {
                let subfolder = match cmd.target {
                    Target::None => false,
                    Target::Name => true,
                    Target::Auto => router
                        .as_ref()
                        .and_then(|r| r.for_archive(a).list(a, cmd.password.as_deref()).ok())
                        .map(|e| top_level_count(&e) != 1)
                        .unwrap_or(true),
                };
                let d = if subfolder { base.join(archive_stem(a)) } else { base.clone() };
                d.to_string_lossy().to_string()
            };
            let flatten = cmd.verb == Verb::ExtractFlat;
            let app = app.clone();
            if archives.len() > 1 {
                let items: Vec<crate::models::ExtractBatchItem> = archives
                    .iter()
                    .map(|a| crate::models::ExtractBatchItem { archive: a.clone(), dest: dest_of(a) })
                    .collect();
                let first = items[0].archive.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::commands::open_extract_window(
                        app,
                        first,
                        Vec::new(),
                        None,
                        Some(true),
                        Some(items),
                        Some(flatten),
                    )
                    .await
                    {
                        gui_error(&e);
                    }
                });
            } else {
                let archive = archives[0].clone();
                let dest = dest_of(&archive);
                let selected = cmd.items.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::commands::open_extract_window(
                        app,
                        archive,
                        selected,
                        Some(dest),
                        Some(true),
                        None,
                        Some(flatten),
                    )
                    .await
                    {
                        gui_error(&e);
                    }
                });
            }
        }
        Verb::List | Verb::Test => {
            // 목록, 검사 화면은 메인 창 자체, 검사는 사용자가 [무결성 검사] 로
            let archive = abs(&cmd.archive);
            if !Path::new(&archive).is_file() {
                gui_error(&format!("Archive not found: {archive}"));
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::commands::open_from_shell(&app, archive, startup).await;
            });
        }
    }
}

/// 목록(l), 7-Zip 과 같은 표
fn run_list(cmd: &Cmd, router: &Router, out: &mut dyn Report) -> i32 {
    out.line(&format!("Listing archive: {}", cmd.archive));
    match router.for_archive(&cmd.archive).list(&cmd.archive, cmd.password.as_deref()) {
        Ok(entries) => {
            out.line("");
            out.line(&format!("{:<19} {:<4} {:>12} {:>12}  Name", "Date", "Attr", "Size", "Packed"));
            let rule = format!("{:-<19} {:-<4} {:-<12} {:-<12}  {:-<24}", "", "", "", "", "");
            out.line(&rule);
            let (mut size, mut packed, mut files, mut folders) = (0u64, 0u64, 0usize, 0usize);
            for e in &entries {
                if e.is_dir {
                    folders += 1;
                } else {
                    files += 1;
                    size += e.size;
                    packed += e.packed_size;
                }
                out.line(&format!(
                    "{:<19} {:<4} {:>12} {:>12}  {}",
                    e.modified,
                    if e.is_dir { "D..." } else { "...." },
                    if e.is_dir { String::new() } else { e.size.to_string() },
                    if e.is_dir { String::new() } else { e.packed_size.to_string() },
                    e.path
                ));
            }
            out.line(&rule);
            out.line(&format!(
                "{:<19} {:<4} {:>12} {:>12}  {} file(s), {} folder(s)",
                "", "", size, packed, files, folders
            ));
            0
        }
        Err(e) => {
            out.line(&format!("Error: {}", err_en(&e)));
            2
        }
    }
}

/// 테스트(t)
fn run_test(cmd: &Cmd, router: &Router, out: &mut dyn Report) -> i32 {
    out.line(&format!("Testing archive: {}", cmd.archive));
    match router.for_archive(&cmd.archive).test(&cmd.archive, cmd.password.as_deref()) {
        Ok(()) => {
            out.line("Test: OK");
            0
        }
        Err(e) => {
            out.line(&format!("Test failed: {}", err_en(&e)));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Lines;

    fn mk(args: &[&str]) -> Vec<String> {
        std::iter::once("ZipMania.exe")
            .chain(args.iter().copied())
            .map(String::from)
            .collect()
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 콘솔_명령_해석() {
        assert!(parse(&mk(&[])).is_none());
        assert!(parse(&mk(&["--open", "a.zip"])).is_none());
        assert!(parse(&mk(&["C"])).is_none());
        assert!(parse(&mk(&["ab", "o.zip"])).is_none());

        // bx = 아카이브 여러 개, 기본 -target:name, 명시하면 그것
        let c = parse(&mk(&["bx", "a.zip", "b.7z"])).unwrap().unwrap();
        assert_eq!((c.verb, c.target), (Verb::ExtractEach, Target::Name));
        assert_eq!((c.archive.as_str(), c.items.as_slice()), ("a.zip", &["b.7z".to_string()][..]));
        let c = parse(&mk(&["bx", "-target:auto", "a.zip"])).unwrap().unwrap();
        assert_eq!(c.target, Target::Auto);

        let c = parse(&mk(&[
            "c", "-l:9", "-fmt:7z", "-v:4GB", "-aou", "-testdst", "-delsrc", "-t:4", "-date",
            "D:\\out\\%y%m%d_%H%M.7z", "D:\\src\\a", "D:\\src\\b.txt",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(c.verb, Verb::Create);
        assert_eq!(c.level, Some(9));
        assert_eq!(c.format, Some(CompressFormat::SevenZip));
        assert_eq!(c.volume, Some(4 << 30));
        assert_eq!(c.exists, Some(Exists::Rename));
        assert!(c.test_dst && c.del_src && c.date);
        assert_eq!(c.threads, Some(4));
        assert_eq!(c.archive, "D:\\out\\%y%m%d_%H%M.7z");
        assert_eq!(c.items, vec!["D:\\src\\a", "D:\\src\\b.txt"]);

        // 7-Zip 표기
        let a = parse(&mk(&[
            "a", "-mx9", "-t7z", "-v700m", "-psecret", "-mmt=4", "-sdel", "-y", "-r", "-bb1", "o.7z", "d",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(a.verb, Verb::Add);
        assert_eq!(a.level, Some(9));
        assert_eq!(a.format, Some(CompressFormat::SevenZip));
        assert_eq!(a.volume, Some(700 << 20));
        assert_eq!(a.password.as_deref(), Some("secret"));
        assert_eq!(a.threads, Some(4));
        assert!(a.del_src);
        assert_eq!(a.exists, Some(Exists::Overwrite));
        let a = parse(&mk(&["a", "-mx=7", "-oD:\\x", "o.zip", "d"])).unwrap().unwrap();
        assert_eq!(a.level, Some(7));
        assert_eq!(a.out_dir.as_deref(), Some("D:\\x"));

        // 해제, 대상 폴더와 항목
        let x = parse(&mk(&["x", "-o:D:\\out dir", "-target:auto", "-aos", "a.zip", "docs/a.txt"]))
            .unwrap()
            .unwrap();
        assert_eq!(x.verb, Verb::Extract);
        assert_eq!(x.out_dir.as_deref(), Some("D:\\out dir"));
        assert_eq!(x.target, Target::Auto);
        assert_eq!(x.exists, Some(Exists::Skip));
        assert_eq!(x.items, vec!["docs/a.txt"]);
        let e = parse(&mk(&["e", "a.zip"])).unwrap().unwrap();
        assert_eq!(e.verb, Verb::ExtractFlat);
        assert_eq!(e.exists, None);
        assert_eq!(e.target, Target::None);
        assert_eq!(parse(&mk(&["l", "a.zip"])).unwrap().unwrap().verb, Verb::List);
        assert_eq!(parse(&mk(&["t", "-p:x", "a.zip"])).unwrap().unwrap().verb, Verb::Test);

        // 사용법 위반
        assert!(parse(&mk(&["c", "out.zip"])).unwrap().is_err());
        assert!(parse(&mk(&["x"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-l:10", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-fmt:rar", "o.rar", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-trar", "o.rar", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-x", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-v:0", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["c", "-t:0", "o.zip", "a"])).unwrap().is_err());
        assert!(parse(&mk(&["x", "-target:x", "a.zip"])).unwrap().is_err());
    }

    #[test]
    fn 도움말_판정() {
        assert!(wants_help(&mk(&["c"])));
        assert!(wants_help(&mk(&["x"])));
        assert!(wants_help(&mk(&["c", "-h"])));
        assert!(wants_help(&mk(&["c", "-l:9", "--help", "o.zip", "a"])));
        assert!(wants_help(&mk(&["/?"])));
        assert!(wants_help(&mk(&["--help"])));
        assert!(!wants_help(&mk(&[])));
        assert!(!wants_help(&mk(&["--open", "a.zip"])));
        assert!(!wants_help(&mk(&["c", "o.zip", "a"])));
        assert!(!wants_help(&mk(&["-h", "x"])));
        for k in ["-testdst", "-v:", "-target:", "-o:", "-mx", "-sdel", " x|e "] {
            assert!(HELP.contains(k), "{k}");
        }
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

    #[test]
    fn 아카이브_이름_폴더() {
        assert_eq!(archive_stem("D:\\x\\backup.7z"), "backup");
        assert_eq!(archive_stem("backup.7z.001"), "backup");
        assert_eq!(archive_stem("src.tar.gz"), "src");
        assert_eq!(archive_stem("v1.2.zip"), "v1.2");
        assert_eq!(archive_stem("noext"), "noext");
    }

    /// 실물 경로, 압축 → 테스트 → 삭제 → 추가 → 목록 → 해제(x, e, 분할), 충돌 기본 = 건너뜀
    #[test]
    fn 콘솔_압축_실행() {
        let root = std::env::temp_dir().join(format!("zm_cmd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("한글.txt"), "hello").unwrap();
        let out = root.join("out.zip");
        let src = root.join("src");
        let outs = out.to_string_lossy().to_string();
        let srcs = src.to_string_lossy().to_string();

        let mut lines = Lines::default();
        let cmd = parse_args(Verb::Create, &args(&["-l:5", "-testdst", "-delsrc", &outs, &srcs])).unwrap();
        let code = execute(&cmd, &mut lines);
        assert_eq!(code, 0, "{:?}", lines.0);
        assert!(out.exists());
        assert!(!src.exists(), "원본이 남았다");

        // 같은 이름 + -aou → (2)
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("b.txt"), "x").unwrap();
        let cmd = parse_args(Verb::Create, &args(&["-aou", &outs, &srcs])).unwrap();
        let code = execute(&cmd, &mut Lines::default());
        assert_eq!(code, 0);
        assert!(root.join("out (2).zip").exists());
        assert!(src.exists(), "-delsrc 없이 원본이 지워졌다");

        // -aos → 건너뜀 = 1
        let cmd = parse_args(Verb::Create, &args(&["-aos", &outs, &srcs])).unwrap();
        assert_eq!(execute(&cmd, &mut Lines::default()), 1);

        // a = 기존 아카이브에 추가, 목록에 새 항목
        std::fs::write(root.join("extra.txt"), "more").unwrap();
        let extra = root.join("extra.txt").to_string_lossy().to_string();
        let cmd = parse_args(Verb::Add, &args(&[&outs, &extra])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0, "{:?}", lines.0);
        assert!(lines.0.iter().any(|l| l.starts_with("Updating archive:")), "{:?}", lines.0);
        let cmd = parse_args(Verb::List, &args(&[&outs])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0);
        assert!(lines.0.iter().any(|l| l.ends_with("extra.txt")), "{:?}", lines.0);
        assert!(lines.0.iter().any(|l| l.ends_with("한글.txt")), "{:?}", lines.0);

        // t
        let cmd = parse_args(Verb::Test, &args(&[&outs])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0);
        assert!(lines.0.iter().any(|l| l == "Test: OK"), "{:?}", lines.0);

        // x -target:auto, 최상위가 둘(src/, extra.txt) → 아카이브 이름 폴더
        let dest = root.join("dest");
        let o = format!("-o:{}", dest.display());
        let cmd = parse_args(Verb::Extract, &args(&[&o, "-target:auto", &outs])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0, "{:?}", lines.0);
        assert_eq!(std::fs::read(dest.join("out").join("src").join("한글.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dest.join("out").join("extra.txt")).unwrap(), b"more");

        // 다시 풀면 기본은 건너뜀(기존 파일 유지), -y 면 덮어씀
        std::fs::write(dest.join("out").join("extra.txt"), "changed").unwrap();
        let cmd = parse_args(Verb::Extract, &args(&[&o, "-target:name", &outs])).unwrap();
        assert_eq!(execute(&cmd, &mut Lines::default()), 0);
        assert_eq!(std::fs::read(dest.join("out").join("extra.txt")).unwrap(), b"changed");
        let cmd = parse_args(Verb::Extract, &args(&[&o, "-target:name", "-y", &outs])).unwrap();
        assert_eq!(execute(&cmd, &mut Lines::default()), 0);
        assert_eq!(std::fs::read(dest.join("out").join("extra.txt")).unwrap(), b"more");

        // e = 경로 없이, 항목 하나만
        let flat = root.join("flat");
        let of = format!("-o:{}", flat.display());
        let cmd = parse_args(Verb::ExtractFlat, &args(&[&of, &outs, "src/한글.txt"])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0, "{:?}", lines.0);
        assert!(flat.join("한글.txt").exists());
        assert!(!flat.join("extra.txt").exists());

        // 없는 아카이브 → 2
        let none = root.join("none.zip").to_string_lossy().to_string();
        let cmd = parse_args(Verb::Extract, &args(&[&of, &none])).unwrap();
        assert_eq!(execute(&cmd, &mut Lines::default()), 2);

        // bx = 아카이브마다 자기 이름 폴더, 둘째 것이 없으면 아무것도 풀지 않고 2
        let each = root.join("each");
        let oe = format!("-o:{}", each.display());
        let out2 = root.join("out (2).zip").to_string_lossy().to_string();
        let cmd = parse_args(Verb::ExtractEach, &args(&[&oe, &outs, &none])).unwrap();
        assert_eq!(execute(&cmd, &mut Lines::default()), 2);
        assert!(!each.exists());
        let cmd = parse_args(Verb::ExtractEach, &args(&[&oe, &outs, &out2])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0, "{:?}", lines.0);
        assert!(each.join("out").join("extra.txt").exists());
        assert!(each.join("out (2)").join("src").join("b.txt").exists());
        assert_eq!(lines.0.iter().filter(|l| l.starts_with("Extracting archive:")).count(), 2);

        // 분할 zip, 1KiB 볼륨, 압축 안 되는 입력
        let mut noise = vec![0u8; 5000];
        let mut x: u32 = 7;
        for b in noise.iter_mut() {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *b = (x >> 16) as u8;
        }
        std::fs::write(src.join("n.bin"), &noise).unwrap();
        let sp = root.join("split.zip");
        let sps = sp.to_string_lossy().to_string();
        let nb = src.join("n.bin").to_string_lossy().to_string();
        let cmd = parse_args(Verb::Create, &args(&["-v:1k", "-l:0", "-testdst", &sps, &nb])).unwrap();
        let mut lines = Lines::default();
        let code = execute(&cmd, &mut lines);
        assert_eq!(code, 0, "{:?}", lines.0);
        assert!(!sp.exists());
        assert!(zipmania_archive::volumes::volume_name(&sp, 5).exists());
        assert!(lines.0.iter().any(|l| l == "Test: OK"), "{:?}", lines.0);

        // 분할 첫 볼륨 해제, -target:auto 는 항목 하나라 폴더 없이, -o 붙여 쓰기
        let first = zipmania_archive::volumes::volume_name(&sp, 1).to_string_lossy().to_string();
        let sd = root.join("sd");
        let osd = format!("-o{}", sd.display());
        let cmd = parse_args(Verb::Extract, &args(&[&osd, "-target:auto", &first])).unwrap();
        let mut lines = Lines::default();
        assert_eq!(execute(&cmd, &mut lines), 0, "{:?}", lines.0);
        assert_eq!(std::fs::read(sd.join("n.bin")).unwrap(), noise);

        let _ = std::fs::remove_dir_all(&root);
    }
}
