//! ZIP 백엔드 검증, 교차 대조가 핵심 — 7z.dll 산출물을 우리가, 우리 산출물을 7z.dll 이 같게 읽는지
//! 목록, 바이트로 양방향 대조(#[cfg(windows)]), 나머지(경로 안전, 손실 방지, CP949, 취소)는 플랫폼 무관

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::backend::{ArchiveBackend, CreateOptions, CreateResult, ExtractOptions, ExtractResult};
#[cfg(windows)]
use crate::backend::EditOptions;
use crate::formats::{CompressFormat, OverwriteMode};

use super::Unzip;

// ─────────────────────────── 도구 ───────────────────────────

struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("zipmania_unzip_{}_{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("임시 폴더 생성 실패");
        TempDir { path }
    }
    fn join(&self, s: &str) -> PathBuf {
        self.path.join(s)
    }
    fn s(&self, s: &str) -> String {
        self.join(s).to_string_lossy().to_string()
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn create_opts(out: &str, inputs: &[String], level: u8, pw: Option<&str>) -> CreateOptions {
    CreateOptions {
        output: out.to_string(),
        inputs: inputs.to_vec(),
        format: CompressFormat::Zip,
        level,
        password: pw.map(|s| s.to_string()),
        encrypt_names: false,
    }
}

fn extract_opts(archive: &str, dest: &str, pw: Option<&str>) -> ExtractOptions {
    ExtractOptions {
        archive: archive.to_string(),
        dest: dest.to_string(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        password: pw.map(|s| s.to_string()),
        selected: vec![],
        decisions: HashMap::new(),
    }
}

/// 폴더 트리를 상대경로 -> 내용 으로 읽어들인다(두 해제 결과 비교용)
#[cfg(windows)]
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, root, &mut v);
    v.sort();
    v
}

fn status_of(r: &CreateResult) -> String {
    match r {
        CreateResult::Done { status, .. } => status.to_string(),
        CreateResult::Failed(e) => format!("failed:{}", e.code),
    }
}

fn ex_status(r: &ExtractResult) -> String {
    match r {
        ExtractResult::Done { status, .. } => status.to_string(),
        ExtractResult::Failed(e) => format!("failed:{}", e.code),
    }
}

/// 병렬 경로를 실제로 타는 입력 트리(파일 수 > parallel 최소치), make_tree 는 4개뿐이라 순차 처리
fn make_many(td: &TempDir, n: usize) -> Vec<String> {
    for i in 0..n {
        let body = format!("항목 {i} 내용 ").repeat(50 + i % 20);
        write(&td.join(&format!("many/d{}/f{i:04}.txt", i % 7)), body.as_bytes());
    }
    vec![td.s("many")]
}

/// 표준 입력 트리 생성(한글 이름, 중첩 폴더, 빈 파일 포함)
fn make_tree(td: &TempDir) -> Vec<String> {
    write(&td.join("src/문서.txt"), "안녕하세요 ZipMania".as_bytes());
    write(&td.join("src/sub/data.bin"), &(0u8..=255).collect::<Vec<u8>>());
    write(&td.join("src/sub/깊은/빈파일.txt"), b"");
    write(&td.join("src/ascii.log"), &vec![b'x'; 100_000]);
    vec![td.s("src")]
}

// ─────────────────── 7z.dll 과의 교차 대조(Windows) ───────────────────

#[cfg(windows)]
mod cross {
    use super::*;
    use crate::backend::sevenzip::SevenZip;

    fn sz() -> SevenZip {
        SevenZip::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("src-tauri")
                .join("binaries")
                .join("7z.dll"),
        )
    }

    fn exe() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src-tauri")
            .join("binaries")
            .join("7z.exe")
    }

    /// 정상 AES-256 zip 을 손상으로 오진하지 않는다, AE-2 는 로컬 헤더 CRC 를 0 으로 적는다
    #[test]
    fn aes_zip_을_손상으로_보지_않는다() {
        let td = TempDir::new("aes_ae2");
        write(&td.join("src/문서.txt"), "안녕하세요 ZipMania".as_bytes());
        let arc = td.join("aes.zip");

        let o = std::process::Command::new(exe())
            .current_dir(&td.path)
            .args(["a", "-tzip", "-mem=AES256", "-ppw1234", "-y"])
            .arg(&arc)
            .arg("src")
            .output()
            .expect("7z.exe 실행 실패");
        assert!(
            o.status.success(),
            "AES zip 픽스처 생성 실패: {}",
            String::from_utf8_lossy(&o.stderr)
        );

        let report = Unzip::new()
            .test_report(
                arc.to_str().unwrap(),
                Some("pw1234"),
                &mut |_, _| {},
                no_cancel(),
            )
            .expect("무결성 검사가 실패했다");
        let files: Vec<_> = report.iter().filter(|e| !e.is_dir).collect();
        assert!(!files.is_empty(), "검사한 파일이 없다");
        for e in &files {
            assert!(e.ok, "정상 AES 항목을 손상으로 보고했다: {}", e.path);
            assert!(
                e.expected_crc.is_none(),
                "AE-2 의 CRC 0 을 기록된 값처럼 다뤘다: {}",
                e.path
            );
        }
    }

    /// 우리가 만든 zip 을 7z.dll 이 같게 읽는다
    #[test]
    fn 우리가_만든_zip_을_7z_가_같게_푼다() {
        let td = TempDir::new("mine_to_7z");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");

        let r = Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());
        assert_eq!(status_of(&r), "ok");

        let a = td.s("ex_mine");
        let b = td.s("ex_7z");
        let r1 = Unzip::new().extract(&extract_opts(&out, &a, None), &mut |_, _| {}, no_cancel());
        let r2 = sz().extract(&extract_opts(&out, &b, None), &mut |_, _| {}, no_cancel());
        assert_eq!(ex_status(&r1), "ok");
        assert_eq!(ex_status(&r2), "ok", "7z.dll 이 우리 zip 을 풀지 못했다");
        assert_eq!(snapshot(Path::new(&a)), snapshot(Path::new(&b)));
    }

    /// 7z.dll 이 만든 zip 을 우리가 같게 읽는다
    #[test]
    fn _7z_가_만든_zip_을_우리가_같게_푼다() {
        let td = TempDir::new("7z_to_mine");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");

        let r = sz().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());
        assert_eq!(status_of(&r), "ok");

        let a = td.s("ex_mine");
        let b = td.s("ex_7z");
        let r1 = Unzip::new().extract(&extract_opts(&out, &a, None), &mut |_, _| {}, no_cancel());
        let r2 = sz().extract(&extract_opts(&out, &b, None), &mut |_, _| {}, no_cancel());
        assert_eq!(ex_status(&r1), "ok");
        assert_eq!(ex_status(&r2), "ok");
        assert_eq!(snapshot(Path::new(&a)), snapshot(Path::new(&b)));
    }

    /// 목록 동일성 — 경로, 크기, 폴더 판정, CRC 문자열까지
    #[test]
    fn 목록이_7z_와_일치한다() {
        let td = TempDir::new("list_eq");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");
        sz().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

        let norm = |mut v: Vec<crate::models::ArchiveEntry>| {
            for e in &mut v {
                e.path = e.path.replace('\\', "/");
                e.modified.clear(); // 시각 표현은 엔진마다 다를 수 있음
                e.packed_size = 0; // 압축 크기는 인코더마다 다르다
            }
            v.sort_by(|a, b| a.path.cmp(&b.path));
            v
        };
        let mine = norm(Unzip::new().list(&out, None).unwrap());
        let theirs = norm(sz().list(&out, None).unwrap());
        assert_eq!(mine, theirs);
    }

    /// 암호 zip 을 서로 읽는다, 기본 = ZipCrypto(7-Zip 과 동일), 비ASCII 는 별도 시험
    #[test]
    fn 암호_zip_이_양방향으로_열린다() {
        let td = TempDir::new("pw");
        let inputs = make_tree(&td);
        let mine = td.s("mine.zip");
        let theirs = td.s("theirs.zip");
        const PW: &str = "Secret1234";

        Unzip::new().create(&create_opts(&mine, &inputs, 5, Some(PW)), &mut |_, _| {}, no_cancel());
        sz().create(&create_opts(&theirs, &inputs, 5, Some(PW)), &mut |_, _| {}, no_cancel());

        // 우리 산출물 → 7z 로 해제
        let d1 = td.s("d1");
        let r = sz().extract(&extract_opts(&mine, &d1, Some(PW)), &mut |_, _| {}, no_cancel());
        assert_eq!(ex_status(&r), "ok", "7z.dll 이 우리 암호 zip 을 풀지 못했다");

        // 7z 산출물 → 우리가 해제
        let d2 = td.s("d2");
        let r = Unzip::new().extract(&extract_opts(&theirs, &d2, Some(PW)), &mut |_, _| {}, no_cancel());
        assert_eq!(ex_status(&r), "ok");
        assert_eq!(snapshot(Path::new(&d1)), snapshot(Path::new(&d2)));
    }

    /// 한글 비밀번호 = 7z.dll 생성 불가(hr=0x80070057), 우리는 생성 가능
    /// 회귀가 아니라 늘어난 능력 — 그 전제가 뒤집히는 것을 잡는 시험
    #[test]
    fn 한글_비밀번호는_우리만_만든다() {
        let td = TempDir::new("pw_ko");
        let inputs = make_tree(&td);
        const PW: &str = "비밀1234";

        let theirs = td.s("theirs.zip");
        let r = sz().create(&create_opts(&theirs, &inputs, 5, Some(PW)), &mut |_, _| {}, no_cancel());
        assert!(
            matches!(r, CreateResult::Failed(_)),
            "7z.dll 이 한글 암호 zip 을 만들었다 — 인코딩을 맞춰야 한다"
        );

        let mine = td.s("mine.zip");
        let r = Unzip::new().create(&create_opts(&mine, &inputs, 5, Some(PW)), &mut |_, _| {}, no_cancel());
        assert_eq!(status_of(&r), "ok");
        let d = td.s("d");
        let r = Unzip::new().extract(&extract_opts(&mine, &d, Some(PW)), &mut |_, _| {}, no_cancel());
        assert_eq!(ex_status(&r), "ok");
    }

    /// 암호 없음 또는 오류 → 전역 오류, 항목 하나의 실패로 뭉개면 UI 의 재질의 불가
    #[test]
    fn 암호_오류가_전역으로_분류된다() {
        let td = TempDir::new("pw_err");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");
        Unzip::new().create(&create_opts(&out, &inputs, 5, Some("맞는암호")), &mut |_, _| {}, no_cancel());

        let r = Unzip::new().extract(&extract_opts(&out, &td.s("d1"), None), &mut |_, _| {}, no_cancel());
        assert_eq!(ex_status(&r), "failed:password_required");

        let r = Unzip::new().extract(
            &extract_opts(&out, &td.s("d2"), Some("틀린암호")),
            &mut |_, _| {},
            no_cancel(),
        );
        assert_eq!(ex_status(&r), "failed:wrong_password");
    }

    /// 편집(추가/삭제) 결과의 7z 가독성
    #[test]
    fn 편집_결과를_7z_가_읽는다() {
        let td = TempDir::new("edit");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");
        Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

        write(&td.join("added/새파일.txt"), "추가됨".as_bytes());
        let r = Unzip::new().edit(
            &EditOptions {
                archive: out.clone(),
                add: vec![td.s("added/새파일.txt")],
                remove: vec!["src/ascii.log".into()],
                password: None,
            },
            &mut |_, _| {},
            no_cancel(),
        );
        assert_eq!(status_of(&r), "ok");

        let names: Vec<String> = sz()
            .list(&out, None)
            .unwrap()
            .into_iter()
            .map(|e| e.path.replace('\\', "/"))
            .collect();
        assert!(names.iter().any(|n| n == "새파일.txt"), "{names:?}");
        assert!(!names.iter().any(|n| n == "src/ascii.log"), "{names:?}");
        // 미삭제 항목 잔존 확인
        assert!(names.iter().any(|n| n == "src/문서.txt"), "{names:?}");
    }

    /// 병렬 경로로 만든 zip 의 7z.dll 가독성, raw_copy_file 접합 과정의 어긋남 탐지
    #[test]
    fn 병렬로_만든_zip_을_7z_가_읽는다() {
        let td = TempDir::new("par_7z");
        let inputs = make_many(&td, 60);
        let out = td.s("out.zip");
        let r = Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());
        assert_eq!(status_of(&r), "ok");

        // 7z 의 목록/무결성 둘 다 통과 확인
        let listed = sz().list(&out, None).expect("7z.dll 이 병렬 산출물을 읽지 못했다");
        assert_eq!(listed.iter().filter(|e| !e.is_dir).count(), 60);
        assert!(sz().test(&out, None).is_ok(), "7z 무결성 검사 실패");

        // 양쪽으로 풀어 바이트까지 대조
        let a = td.s("ex_mine");
        let b = td.s("ex_7z");
        Unzip::new().extract(&extract_opts(&out, &a, None), &mut |_, _| {}, no_cancel());
        sz().extract(&extract_opts(&out, &b, None), &mut |_, _| {}, no_cancel());
        assert_eq!(snapshot(Path::new(&a)), snapshot(Path::new(&b)));
    }

    /// 병렬과 순차(암호로 병렬을 끈 경로)의 내용 동일, 순서 뒤바뀜과 누락 탐지
    #[test]
    fn 병렬과_순차의_내용이_같다() {
        let td = TempDir::new("par_eq");
        let inputs = make_many(&td, 40);

        let par = td.s("par.zip");
        Unzip::new().create(&create_opts(&par, &inputs, 5, None), &mut |_, _| {}, no_cancel());
        // 암호를 걸면 병렬 경로를 쓰지 않는다(raw_copy 가 암호를 옮기지 못하므로)
        let seq = td.s("seq.zip");
        Unzip::new().create(&create_opts(&seq, &inputs, 5, Some("pw")), &mut |_, _| {}, no_cancel());

        let a = td.s("a");
        let b = td.s("b");
        Unzip::new().extract(&extract_opts(&par, &a, None), &mut |_, _| {}, no_cancel());
        Unzip::new().extract(&extract_opts(&seq, &b, Some("pw")), &mut |_, _| {}, no_cancel());
        assert_eq!(snapshot(Path::new(&a)), snapshot(Path::new(&b)));

        // 항목 순서 동일(입력 순서 = 아카이브 순서)
        let pn: Vec<String> = Unzip::new().list(&par, None).unwrap().into_iter().map(|e| e.path).collect();
        let sn: Vec<String> = Unzip::new().list(&seq, None).unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(pn, sn);
    }

    /// 손상 zip → 7z 폴백, 중앙 디렉터리 절단 시 우리는 열기 실패, 7z 는 로컬 헤더로 복구
    #[test]
    fn 손상된_zip_은_폴백으로_넘어간다() {
        let td = TempDir::new("fallback");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");
        sz().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

        // 꼬리(중앙 디렉터리 + EOCD) 절단
        let mut bytes = fs::read(&out).unwrap();
        let cut = bytes.len() * 3 / 4;
        bytes.truncate(cut);
        fs::write(&out, &bytes).unwrap();

        // 폴백 없는 백엔드 = 명확한 거부
        let bare = Unzip::new().list(&out, None);
        assert!(bare.is_err(), "폴백 없이 손상 zip 을 열었다");

        // 폴백 물린 백엔드 = 7z 가 읽어 줌
        let with_fb = Unzip::with_fallback(Box::new(sz()));
        let listed = with_fb.list(&out, None);
        assert!(listed.is_ok(), "폴백이 동작하지 않았다: {:?}", listed.err());
        assert!(!listed.unwrap().is_empty());
    }

    /// 상태줄 엔진 버전 = 여전히 7-Zip
    /// Router::engine_version = 첫 응답 백엔드 값, Unzip 이 답하면 표시가 사실과 어긋남
    #[test]
    fn 엔진_버전은_7zip_을_가리킨다() {
        let router = crate::backend::Router::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("src-tauri")
                .join("binaries")
                .join("7z.dll"),
        );
        let v = router.engine_version().expect("엔진 버전을 못 읽었다");
        assert!(v.contains("7-Zip"), "상태줄에 엉뚱한 엔진이 뜬다: {v}");
    }

    /// 뷰어와 드래그도 폴백 경로, 손상 zip 의 항목 추출은 7z 몫
    #[test]
    fn 항목_추출도_폴백을_탄다() {
        let td = TempDir::new("entry_fb");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");
        sz().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

        let mut bytes = fs::read(&out).unwrap();
        let cut = bytes.len() * 3 / 4;
        bytes.truncate(cut);
        fs::write(&out, &bytes).unwrap();

        let with = Unzip::with_fallback(Box::new(sz()));
        // 잘린 뒤쪽이 아니라 앞쪽 항목 → 7z 는 로컬 헤더로 복구 가능
        let r = with.read_entry_to_memory(&out, "src/문서.txt", None);
        assert!(r.is_ok(), "폴백이 뷰어 경로에서 동작하지 않았다: {:?}", r.err());
        assert_eq!(r.unwrap(), "안녕하세요 ZipMania".as_bytes());
    }

    /// 라우터가 실제로 zip 을 우리에게 주고, 그 결과를 7z 가 읽는지 — 통합 경로 확인
    #[test]
    fn 라우터를_거쳐도_같은_결과다() {
        let td = TempDir::new("router");
        let inputs = make_tree(&td);
        let out = td.s("out.zip");
        let router = crate::backend::Router::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("src-tauri")
                .join("binaries")
                .join("7z.dll"),
        );
        let b = router.for_format("zip");
        assert_eq!(b.id(), "unzip");
        let r = b.create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());
        assert_eq!(status_of(&r), "ok");
        assert!(sz().list(&out, None).is_ok());
    }
}

// ─────────────────── 플랫폼 무관 검증 ───────────────────

/// 국산 압축기 zip(CP949 이름, UTF-8 플래그 없음)의 이름을 바르게 읽는다
/// 크레이트 기본 name() 은 CP437 로 읽어 깨진다 — super::entry_name 이 직접 푼다
#[test]
fn cp949_파일이름을_바르게_읽는다() {
    let td = TempDir::new("cp949");
    let path = td.join("cp949.zip");
    // "한글문서.txt" 의 CP949 바이트
    let name: &[u8] = &[0xC7, 0xD1, 0xB1, 0xDB, 0xB9, 0xAE, 0xBC, 0xAD, b'.', b't', b'x', b't'];
    fs::write(&path, store_zip(name, b"hello", 0)).unwrap();

    let list = Unzip::new().list(&path.to_string_lossy(), None).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].path, "한글문서.txt");
}

/// UTF-8 플래그가 있으면 그대로 읽는다(CP949 로 잘못 되돌리지 않는다)
#[test]
fn utf8_플래그가_있으면_그대로_읽는다() {
    let td = TempDir::new("utf8name");
    let path = td.join("utf8.zip");
    fs::write(
        &path,
        store_zip("한글문서.txt".as_bytes(), b"hello", 0x0800),
    )
    .unwrap();
    let list = Unzip::new().list(&path.to_string_lossy(), None).unwrap();
    assert_eq!(list[0].path, "한글문서.txt");
}

/// 경로 탈출 항목 = 건너뛰기 + warning 통지
/// 거부가 아닌 건너뛰기 = 하나가 전체를 죽이지 않게, ok 가 아닌 이유 = 앱이 ok 를 보고 원본을 지운다
#[test]
fn 경로_탈출_항목은_건너뛰고_경고한다() {
    let td = TempDir::new("slip");
    let path = td.join("evil.zip");
    fs::write(
        &path,
        store_zip(b"../../evil.txt", b"pwned", 0),
    )
    .unwrap();

    let dest = td.join("out");
    let r = Unzip::new().extract(
        &extract_opts(&path.to_string_lossy(), &dest.to_string_lossy(), None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(ex_status(&r), "warning", "탈출 항목을 조용히 넘겼다");
    assert!(!td.join("evil.txt").exists(), "해제 폴더 밖에 파일이 생겼다");
    assert!(!dest.join("evil.txt").exists());
}

/// 절대 경로 항목도 해제 폴더 안으로 낙착(7-Zip 과 같은 동작)
#[test]
fn 절대경로_항목은_루트_아래로_들어온다() {
    let td = TempDir::new("abs");
    let path = td.join("abs.zip");
    fs::write(&path, store_zip(b"/etc/passwd", b"x", 0)).unwrap();

    let dest = td.join("out");
    let r = Unzip::new().extract(
        &extract_opts(&path.to_string_lossy(), &dest.to_string_lossy(), None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(ex_status(&r), "ok");
    assert!(dest.join("etc/passwd").exists());
}

/// 압축 실패에도 기존 파일 잔존, 임시 파일 → rename 규칙의 핵심
#[test]
fn 압축_실패해도_기존_파일이_남는다() {
    let td = TempDir::new("keep_on_fail");
    let out = td.join("out.zip");
    write(&out, "소중한 기존 아카이브".as_bytes());

    // 존재하지 않는 입력만 → no_input 실패
    let r = Unzip::new().create(
        &create_opts(&out.to_string_lossy(), &[td.s("없는파일.txt")], 5, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(status_of(&r), "failed:no_input");
    assert_eq!(fs::read(&out).unwrap(), "소중한 기존 아카이브".as_bytes());
}

/// 취소해도 기존 파일이 남고 임시 파일도 남지 않는다
#[test]
fn 압축_취소해도_기존_파일이_남는다() {
    let td = TempDir::new("cancel");
    let inputs = make_tree(&td);
    let out = td.join("out.zip");
    write(&out, "기존".as_bytes());

    let cancel = Arc::new(AtomicBool::new(true)); // 시작부터 취소 상태
    let r = Unzip::new().create(
        &create_opts(&out.to_string_lossy(), &inputs, 5, None),
        &mut |_, _| {},
        cancel,
    );
    assert_eq!(status_of(&r), "canceled");
    assert_eq!(fs::read(&out).unwrap(), "기존".as_bytes());

    let leftovers: Vec<_> = fs::read_dir(&td.path)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".zmtmp-"))
        .collect();
    assert!(leftovers.is_empty(), "임시 파일이 남았다: {leftovers:?}");
}

/// 병렬 경로에서 취소해도 멈추지 않는다
/// 워커는 예산 미확보 시 대기, 주 스레드가 세우지 않으면 그대로 멈춤(미종료 = 실패)
#[test]
fn 병렬_압축을_취소해도_멈추지_않는다() {
    let td = TempDir::new("par_cancel");
    let inputs = make_many(&td, 60);
    let out = td.join("out.zip");
    write(&out, "기존".as_bytes());

    let cancel = Arc::new(AtomicBool::new(true));
    let r = Unzip::new().create(
        &create_opts(&out.to_string_lossy(), &inputs, 5, None),
        &mut |_, _| {},
        cancel,
    );
    assert_eq!(status_of(&r), "canceled");
    assert_eq!(fs::read(&out).unwrap(), "기존".as_bytes());

    let left: Vec<_> = fs::read_dir(&td.path)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".zmtmp-"))
        .collect();
    assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");
}

/// 목록이 신고한 크기와 실제가 다르면 ok 가 아니다
/// CRC = 나온 바이트 기준이라 목록의 거짓말 탐지 불가, ok = [해제 후 원본 삭제] 의 조건
#[test]
fn 목록과_크기가_다르면_경고한다() {
    let td = TempDir::new("size_lie");
    let body = b"0123456789";
    write(&td.join("src/a.txt"), body);
    let arc = td.s("a.zip");
    let r = Unzip::new().create(
        &create_opts(&arc, &[td.s("src")], 0, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(status_of(&r), "ok");

    // 중앙 디렉터리의 uncompressed size 를 10 → 100 으로 부풀림, 무압축이라 두 크기가 같은 값
    let mut b = fs::read(&arc).unwrap();
    let want = (body.len() as u32).to_le_bytes();
    let mut patched = 0;
    let mut i = 0;
    while i + 4 <= b.len() {
        // 중앙 디렉터리 헤더(PK\x01\x02)의 uncompressed size 필드 = 오프셋 24
        if &b[i..i + 4] == b"PK\x01\x02" && i + 28 <= b.len() && b[i + 24..i + 28] == want {
            b[i + 24..i + 28].copy_from_slice(&100u32.to_le_bytes());
            patched += 1;
        }
        i += 1;
    }
    assert_eq!(patched, 1, "중앙 디렉터리 크기 필드를 찾지 못했다");
    fs::write(&arc, &b).unwrap();

    let dest = td.join("out");
    let r = Unzip::new().extract(
        &extract_opts(&arc, &dest.to_string_lossy(), None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(
        ex_status(&r),
        "warning",
        "목록이 부풀린 크기를 성공으로 마감했다"
    );
    // 미기록, 모순된 항목이라 옳은 값 판정 불가 — 검사가 commit 보다 먼저
    assert!(
        !dest.join("src/a.txt").exists(),
        "모순된 항목을 그대로 써 버렸다"
    );
    let left = leftover_tmp(&dest);
    assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");
}

/// 모순된 항목이 기존 정상 파일을 덮지 않는다
/// 크기 검증이 commit() 뒤면 이미 덮어쓴 뒤라 되돌릴 것 없음, 검사는 옮기기 전
#[test]
fn 모순된_항목이_기존_파일을_덮지_않는다() {
    let td = TempDir::new("size_lie_keep");
    let body = b"0123456789";
    write(&td.join("src/a.txt"), body);
    let arc = td.s("a.zip");
    Unzip::new().create(
        &create_opts(&arc, &[td.s("src")], 0, None),
        &mut |_, _| {},
        no_cancel(),
    );

    let mut b = fs::read(&arc).unwrap();
    let want = (body.len() as u32).to_le_bytes();
    let mut i = 0;
    while i + 28 <= b.len() {
        if &b[i..i + 4] == b"PK\x01\x02" && b[i + 24..i + 28] == want {
            b[i + 24..i + 28].copy_from_slice(&100u32.to_le_bytes());
            break;
        }
        i += 1;
    }
    fs::write(&arc, &b).unwrap();

    let dest = td.join("out");
    let victim = dest.join("src/a.txt");
    let keep = "덮이면 안 되는 기존 파일".as_bytes();
    write(&victim, keep);

    let r = Unzip::new().extract(
        &extract_opts(&arc, &dest.to_string_lossy(), None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(ex_status(&r), "warning");
    assert_eq!(
        fs::read(&victim).unwrap(),
        keep,
        "모순된 항목이 기존 정상 파일을 덮었다"
    );
}

/// 없는 입력을 조용히 빼놓지 않는다
/// 그냥 건너뛰면 멀쩡한 파일과 함께 넘겼을 때 ok, 사용자의 원본 삭제 판단 유발
#[test]
fn 없는_입력은_누락으로_보고한다() {
    let td = TempDir::new("missing_input");
    write(&td.join("있는파일.txt"), "내용".as_bytes());
    let out = td.s("out.zip");

    let inputs = vec![td.s("있는파일.txt"), td.s("없는파일.txt")];
    let r = Unzip::new().create(
        &create_opts(&out, &inputs, 5, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(
        status_of(&r),
        "warning",
        "없는 입력을 빼놓고 성공으로 마감했다"
    );
    if let CreateResult::Done { message, .. } = &r {
        assert!(
            message.contains("없는파일.txt"),
            "어느 것이 빠졌는지 알려 주지 않는다: {message}"
        );
    }
}

/// 무압축으로 담긴 바이트열을 찾아 한 바이트를 뒤집는다 — CRC 를 깨뜨리는 가장 확실한 방법
fn corrupt_stored(zip: &Path, needle: &[u8]) {
    let mut b = fs::read(zip).unwrap();
    let at = b
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("무압축 데이터를 찾지 못했다");
    b[at] ^= 0xFF;
    fs::write(zip, b).unwrap();
}

/// 대상 폴더에 남은 임시 파일 목록(하위 폴더까지)
fn leftover_tmp(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(leftover_tmp(&p));
        } else {
            let n = e.file_name().to_string_lossy().to_string();
            if n.contains(".zmtmp-") {
                out.push(n);
            }
        }
    }
    out
}

/// 손상 아카이브 덮어쓰기에도 기존 파일 잔존
/// 예전에는 File::create 로 먼저 자른 뒤 풀어, CRC 가 어긋나면 기존 파일까지 사라졌다
#[test]
fn 해제_실패해도_기존_파일이_남는다() {
    let td = TempDir::new("keep_on_crc_error");
    let body = "원래 내용 ".repeat(64);
    write(&td.join("src/문서.txt"), body.as_bytes());

    // 레벨 0(무압축) → 내용이 아카이브 안에 그대로
    let arc = td.s("a.zip");
    let r = Unzip::new().create(
        &create_opts(&arc, &[td.s("src")], 0, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(status_of(&r), "ok");
    corrupt_stored(Path::new(&arc), body.as_bytes());

    let dest = td.join("out");
    let victim = dest.join("src/문서.txt");
    let keep = "건드리면 안 되는 기존 파일".as_bytes();
    write(&victim, keep);

    let r = Unzip::new().extract(
        &extract_opts(&arc, &dest.to_string_lossy(), None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(ex_status(&r), "warning", "CRC 오류는 빠진 항목으로 보고한다");
    assert_eq!(
        fs::read(&victim).unwrap(),
        keep,
        "손상된 아카이브를 푸는 것이 기존 파일을 날렸다"
    );
    let left = leftover_tmp(&dest);
    assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");
}

/// 해제 취소에도 기존 파일 잔존
/// 취소는 정상 경로다, 쓰던 것을 지우며 원본까지 지우면 [취소] 한 번이 파일을 없앤다
#[test]
fn 해제_취소해도_기존_파일이_남는다() {
    let td = TempDir::new("keep_on_cancel");
    write(&td.join("src/문서.txt"), &vec![b'z'; 400_000]);
    let arc = td.s("a.zip");
    let r = Unzip::new().create(
        &create_opts(&arc, &[td.s("src")], 0, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(status_of(&r), "ok");

    let dest = td.join("out");
    let victim = dest.join("src/문서.txt");
    let keep = "취소했으니 그대로여야 한다".as_bytes();
    write(&victim, keep);

    // 첫 진행률 보고에서 취소 → 대상 파일을 연 뒤 중단되는 상황 재현
    let cancel = no_cancel();
    let flip = cancel.clone();
    let mut prog = move |_p: u8, _f: Option<String>| {
        flip.store(true, std::sync::atomic::Ordering::SeqCst);
    };
    let r = Unzip::new().extract(
        &extract_opts(&arc, &dest.to_string_lossy(), None),
        &mut prog,
        cancel,
    );
    assert_eq!(ex_status(&r), "canceled");
    assert_eq!(fs::read(&victim).unwrap(), keep, "취소가 기존 파일을 날렸다");
    let left = leftover_tmp(&dest);
    assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");
}

/// 레벨 0 은 무압축(Store) 이고, 레벨을 올리면 더 작아진다
#[test]
fn 레벨이_산출물_크기에_반영된다() {
    let td = TempDir::new("levels");
    write(&td.join("src/big.txt"), &vec![b'a'; 200_000]);
    let inputs = vec![td.s("src")];

    let store = td.s("store.zip");
    let best = td.s("best.zip");
    Unzip::new().create(&create_opts(&store, &inputs, 0, None), &mut |_, _| {}, no_cancel());
    Unzip::new().create(&create_opts(&best, &inputs, 9, None), &mut |_, _| {}, no_cancel());

    let s = fs::metadata(&store).unwrap().len();
    let b = fs::metadata(&best).unwrap().len();
    assert!(s > 200_000, "레벨 0 이 압축을 했다: {s}");
    assert!(b < s / 10, "레벨 9 가 줄이지 못했다: {b} vs {s}");
}

/// 원본 수정 시각 보존, 압축 후 해제 시 날짜가 오늘로 바뀌던 동작 고정
#[test]
fn 원본_수정시각을_보존한다() {
    let td = TempDir::new("mtime");
    let src = td.join("src/old.txt");
    write(&src, b"old");
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &[td.s("src")], 5, None), &mut |_, _| {}, no_cancel());

    let listed = Unzip::new().list(&out, None).unwrap();
    let file = listed.iter().find(|e| !e.is_dir).unwrap();
    // 방금 만든 파일이라 올해 날짜 필요, 1980-01-01(= 값 없음)이면 실패
    assert!(!file.modified.is_empty(), "수정 시각이 비어 있다");
    assert!(
        !file.modified.starts_with("1980"),
        "수정 시각이 기본값으로 떨어졌다: {}",
        file.modified
    );
}

/// 못 읽는 압축 방식 → 7z 위임, zstd(93) 항목 하나라도 있으면 아카이브 전체를 폴백으로
/// 폴백 없는 구성(다른 플랫폼)에서는 unsupported 로 거부
#[test]
fn 모르는_압축방식은_우리가_맡지_않는다() {
    let td = TempDir::new("unknown_method");
    let path = td.join("zstd.zip");
    // 방식 id 만 93 으로 박은 최소 zip, 라우팅 판정만 보므로 데이터는 임의 값
    fs::write(&path, raw_zip(b"a.txt", b"anything", 0, 0, 93)).unwrap();

    let e = Unzip::new()
        .list(&path.to_string_lossy(), None)
        .expect_err("우리가 모르는 방식을 처리해 버렸다");
    assert_eq!(e.code, "unsupported");
    assert!(e.message.contains("93"), "어느 방식인지 알려야 한다: {}", e.message);
}

/// 반대쪽 — 읽을 수 있는 방식은 폴백 금지, 컴파일 feature 누락으로 전부 7z 로 흐르는 것 탐지
#[test]
fn 읽을_수_있는_방식은_우리가_맡는다() {
    let td = TempDir::new("known_methods");
    // Store(0) 과 Deflate(8) = 확실히 우리 몫
    for (m, tag) in [(0u16, "store"), (8u16, "deflate")] {
        let path = td.join(&format!("{tag}.zip"));
        let body = b"hello";
        let data: Vec<u8> = if m == 0 {
            body.to_vec()
        } else {
            // 최소 deflate 스트림(비압축 블록)
            let mut v = vec![0x01, 5, 0, 0xFA, 0xFF];
            v.extend_from_slice(body);
            v
        };
        fs::write(&path, raw_zip(b"a.txt", &data, 0, crc32(body), m)).unwrap();
        let list = Unzip::new()
            .list(&path.to_string_lossy(), None)
            .unwrap_or_else(|e| panic!("{tag}: 우리가 거부했다 [{}] {}", e.code, e.message));
        assert_eq!(list.len(), 1, "{tag}");
    }
}

// ─────────────── 앱이 실제로 쓰는 나머지 경로 ───────────────
//
// 뷰어, 드래그, 바이러스 검사, 충돌 검사 = UI 에서만 드러나는 기능, 여기서 빠지면 압축/해제는
// 되는데 미리보기가 안 되는 상태로 배포

/// 뷰어 — 임시 해제 없이 항목 하나를 메모리로 읽기
#[test]
fn 뷰어_경로가_바이트를_그대로_준다() {
    let td = TempDir::new("mem");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

    let got = Unzip::new()
        .read_entry_to_memory(&out, "src/문서.txt", None)
        .expect("메모리 읽기 실패");
    assert_eq!(got, "안녕하세요 ZipMania".as_bytes());

    // 없는 항목은 not_found 다(조용히 빈 값을 주면 뷰어가 빈 화면을 띄운다)
    let e = Unzip::new()
        .read_entry_to_memory(&out, "src/없는파일.txt", None)
        .expect_err("없는 항목을 성공으로 돌려줬다");
    assert_eq!(e.code, "not_found");
}

/// 드래그 — 파일로, 그리고 임의 writer 로 스트리밍 추출
#[test]
fn 드래그_경로가_파일과_writer_로_추출한다() {
    let td = TempDir::new("drag");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());
    let want = (0u8..=255).collect::<Vec<u8>>();

    let dest = td.join("dragged.bin");
    Unzip::new()
        .extract_entry_to_file(&out, "src/sub/data.bin", &dest, None)
        .expect("파일 추출 실패");
    assert_eq!(fs::read(&dest).unwrap(), want);

    // writer 경로(셸 드래그의 지연 렌더링)는 임시 파일 없이 흘려보낸다
    struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
    Unzip::new()
        .extract_entry_to_writer(&out, "src/sub/data.bin", Box::new(Sink(buf.clone())), None)
        .expect("writer 추출 실패");
    assert_eq!(*buf.lock().unwrap(), want);
}

/// 바이러스 검사 — 항목별 콜백 + 상한 초과는 skipped
/// skipped 를 안전에 섞지 않는 것이 핵심(ScanResultDialog 가 그 값으로 미검사 항목을 띄운다)
#[test]
fn 바이러스_검사가_항목별로_돈다() {
    let td = TempDir::new("scan");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    // 10KB 상한 — 100KB 짜리 ascii.log 는 검사 제외
    let report = Unzip::new()
        .scan_report(
            &out,
            None,
            10_000,
            Box::new(move |path, bytes| {
                s2.lock().unwrap().push(path.to_string());
                if bytes.starts_with(b"EICAR") { "malware".into() } else { "clean".into() }
            }),
            &mut |_, _| {},
            no_cancel(),
        )
        .expect("검사 실패");

    assert!(report.iter().all(|e| !e.is_dir), "폴더가 결과에 섞였다");
    let big = report.iter().find(|e| e.path.ends_with("ascii.log")).unwrap();
    assert_eq!(big.status, "skipped", "상한 초과를 검사한 척했다");
    let small = report.iter().find(|e| e.path.ends_with("문서.txt")).unwrap();
    assert_eq!(small.status, "clean");
    assert!(
        seen.lock().unwrap().iter().all(|p| !p.ends_with("ascii.log")),
        "상한을 넘는 항목까지 콜백에 넘겼다"
    );
}

/// 충돌 검사 — 대상 폴더에 이미 있는 파일만, 목록이 비면 UI 가 덮어씀
#[test]
fn 충돌_검사가_기존_파일만_찾는다() {
    let td = TempDir::new("conflict");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

    let dest = td.join("d");
    write(&dest.join("src/문서.txt"), "기존".as_bytes());
    let found = Unzip::new()
        .find_conflicts(&out, &dest.to_string_lossy(), true, &[], None)
        .expect("충돌 검사 실패");
    assert_eq!(found, vec!["src/문서.txt".to_string()], "{found:?}");
}

/// 손상된 항목을 test_report 가 잡아낸다(예상 CRC 와 실제가 다르다)
#[test]
fn 무결성_검사가_손상을_잡는다() {
    let td = TempDir::new("test_report");
    let path = td.join("bad.zip");
    // CRC 를 일부러 틀리게 적은 Store 아카이브
    fs::write(&path, raw_zip(b"a.txt", b"hello", 0, 0xDEAD_BEEF, 0)).unwrap();

    let report = Unzip::new()
        .test_report(&path.to_string_lossy(), None, &mut |_, _| {}, no_cancel())
        .unwrap();
    assert_eq!(report.len(), 1);
    assert!(!report[0].ok, "손상을 정상으로 봤다");
    assert_eq!(report[0].expected_crc, Some(0xDEAD_BEEF));
    assert!(report[0].actual_crc.is_some(), "실제 CRC 를 보고하지 않았다");
}

/// 정상 아카이브는 전부 ok 다
#[test]
fn 무결성_검사가_정상을_통과시킨다() {
    let td = TempDir::new("test_ok");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

    let report = Unzip::new()
        .test_report(&out, None, &mut |_, _| {}, no_cancel())
        .unwrap();
    assert!(!report.is_empty());
    assert!(report.iter().all(|e| e.ok), "{report:?}");
    assert!(Unzip::new().test(&out, None).is_ok());
}

/// 메모리 읽기 경로의 상한(압축 폭탄 방어)
#[test]
fn 메모리_읽기에_상한이_걸린다() {
    // 상한 자체 = formats 의 상수, 여기서는 뷰어 경로가 그 상수를 쓰는지만 고정
    assert_eq!(crate::formats::MAX_MEMORY_ENTRY_BYTES, 256 * 1024 * 1024);
}

/// 선택 해제 — 고른 폴더 아래만 산출
#[test]
fn 선택한_경로만_해제한다() {
    let td = TempDir::new("selected");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

    let dest = td.join("sel");
    let mut opts = extract_opts(&out, &dest.to_string_lossy(), None);
    opts.selected = vec!["src/sub".into()];
    let r = Unzip::new().extract(&opts, &mut |_, _| {}, no_cancel());
    assert_eq!(ex_status(&r), "ok");
    assert!(dest.join("src/sub/data.bin").exists());
    assert!(!dest.join("src/문서.txt").exists());
}

/// 충돌 정책 — 건너뛰기 = 기존 파일 보존
#[test]
fn 건너뛰기가_기존_파일을_보존한다() {
    let td = TempDir::new("skip");
    let inputs = make_tree(&td);
    let out = td.s("out.zip");
    Unzip::new().create(&create_opts(&out, &inputs, 5, None), &mut |_, _| {}, no_cancel());

    let dest = td.join("d");
    write(&dest.join("src/문서.txt"), b"KEEP");
    let mut opts = extract_opts(&out, &dest.to_string_lossy(), None);
    opts.overwrite = OverwriteMode::Skip;
    Unzip::new().extract(&opts, &mut |_, _| {}, no_cancel());
    assert_eq!(fs::read(dest.join("src/문서.txt")).unwrap(), b"KEEP");
}

// ─────────────────────────── 픽스처 생성기 ───────────────────────────

/// Store(무압축) 항목 1개짜리 최소 zip 을 손으로 조립
/// 크레이트는 이름을 정규화 → 비정상 입력(CP949, 경로 탈출) 주입 불가
fn store_zip(name: &[u8], data: &[u8], flags: u16) -> Vec<u8> {
    raw_zip(name, data, flags, crc32(data), 0)
}

fn raw_zip(name: &[u8], data: &[u8], flags: u16, crc: u32, method: u16) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);

    let cd_off = out.len() as u32;
    let mut cd: Vec<u8> = Vec::new();
    cd.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
    cd.extend_from_slice(&20u16.to_le_bytes());
    cd.extend_from_slice(&20u16.to_le_bytes());
    cd.extend_from_slice(&flags.to_le_bytes());
    cd.extend_from_slice(&method.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&crc.to_le_bytes());
    cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
    cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
    cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u16.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.extend_from_slice(&0u32.to_le_bytes());
    cd.extend_from_slice(name);
    let cd_len = cd.len() as u32;
    out.extend_from_slice(&cd);

    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = crate::crc32::Crc32::new();
    c.update(data);
    c.finalize()
}

/// 수집 뒤에 커진 파일은 병렬로 감당하지 않는다 — 잘라 쓰지도 않는다
/// MAX_ENTRY 판정도 예산도 수집 시점 크기라, 그 뒤 커지면 상한이 무의미해진다
/// 신고한 만큼만 읽고 끊으면 내용 절단 → 그 항목만 병렬 포기(Piece::TooBig), 주 스레드로 위임
#[test]
fn 커진_파일은_병렬을_포기한다() {
    use super::parallel::{self, Piece};
    use crate::inputs::InputItem;

    let td = TempDir::new("par_grow");
    let body = vec![b'x'; 4096];
    let liar = 3usize;

    let mut items = Vec::new();
    for i in 0..10 {
        let p = td.join(&format!("f{i}.bin"));
        write(&p, &body);
        items.push(InputItem {
            name: format!("f{i}.bin"),
            source: Some(p),
            // 하나만 실제보다 작게 신고(= 수집 뒤에 커진 것과 같은 상태)
            size: if i == liar { 10 } else { body.len() as u64 },
            is_dir: false,
            mtime: None,
        });
    }

    let workers = 4;
    let elig = parallel::eligible(&items, workers);
    assert_eq!(elig.len(), items.len(), "전부 병렬 대상이어야 한다");

    let cancel = Arc::new(AtomicBool::new(false));
    let pipe = parallel::Pipeline::start(
        &items,
        &elig,
        workers,
        |it| super::create::plain_options(5, it.mtime, it.size),
        cancel.clone(),
    );

    for &idx in &elig {
        match pipe.take(idx, &cancel).expect("취소하지 않았는데 결과가 없다") {
            Ok(Piece::TooBig) => assert_eq!(idx, liar, "멀쩡한 항목을 병렬에서 뺐다"),
            Ok(Piece::Zip(b)) => {
                assert_ne!(idx, liar, "커진 항목을 병렬 결과로 내놓았다(잘렸을 수 있다)");
                let mut one = zip::ZipArchive::new(std::io::Cursor::new(b)).expect("1개짜리 zip");
                let mut f = one.by_index(0).expect("항목");
                let mut got = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut got).expect("읽기");
                assert_eq!(got, body, "병렬 결과의 내용이 다르다");
            }
            Err(e) => panic!("항목 {idx} 실패: {} {}", e.code, e.message),
        }
    }
    pipe.stop();
}

/// 이름을 알 수 없는 입력도 누락으로 보고
/// 드라이브 루트(C:\) 와 .. 는 file_name() 없음, 그냥 건너뛰면 빠졌는데도 ok
#[test]
fn 이름을_알_수_없는_입력도_누락으로_보고한다() {
    let td = TempDir::new("nameless_input");
    write(&td.join("있는파일.txt"), "내용".as_bytes());
    let out = td.s("out.zip");

    // .. 는 어느 플랫폼에서나 file_name() 없음(드라이브 루트와 같은 부류)
    let inputs = vec![td.s("있는파일.txt"), "..".to_string()];
    let r = Unzip::new().create(
        &create_opts(&out, &inputs, 5, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(
        status_of(&r),
        "warning",
        "담지 못한 입력을 빼놓고 성공으로 마감했다"
    );
    if let CreateResult::Done { message, .. } = &r {
        assert!(
            message.contains(".."),
            "어느 것이 빠졌는지 알려 주지 않는다: {message}"
        );
    }
}

/// 단일 항목 추출도 대상을 먼저 자르지 않는다
/// 예전에는 File::create 라 CRC 어긋남 시 잘린 파일만 잔존, 이 크레이트는 다른 앱도 사용
#[test]
fn 단일_항목_추출은_대상을_먼저_자르지_않는다() {
    let td = TempDir::new("entry_to_file");
    let body = b"0123456789abcdef";
    write(&td.join("src/a.txt"), body);
    let arc = td.s("a.zip");
    let r = Unzip::new().create(
        &create_opts(&arc, &[td.s("src")], 0, None),
        &mut |_, _| {},
        no_cancel(),
    );
    assert_eq!(status_of(&r), "ok");

    // ── 1. 정상 항목 그대로 산출 ──
    let out = td.join("뽑은것.txt");
    Unzip::new()
        .extract_entry_to_file(&arc, "src/a.txt", &out, None)
        .expect("정상 항목 추출이 실패했다");
    assert_eq!(fs::read(&out).unwrap(), body);

    // ── 2. 손상 항목은 기존 파일을 건드리지 않는다 ──
    corrupt_stored(Path::new(&arc), body);
    let keep = "기존 내용".as_bytes();
    write(&out, keep);
    let e = Unzip::new()
        .extract_entry_to_file(&arc, "src/a.txt", &out, None)
        .expect_err("CRC 가 깨졌는데 성공했다");
    assert_eq!(e.code, "corrupt");
    assert_eq!(
        fs::read(&out).unwrap(),
        keep,
        "실패했는데 기존 파일이 잘렸다"
    );
    assert!(
        leftover_tmp(&td.path).is_empty(),
        "임시 파일이 남았다: {:?}",
        leftover_tmp(&td.path)
    );
}
