//! EGG/ALZ 백엔드 통합 테스트 — 실제 알집 산출물로 검증
//!
//! 픽스처(tests/fixtures/test.egg, test.alz)는 같은 PDF 를 두 포맷으로 압축한 것이라, 해제
//! 결과가 서로 바이트 단위로 동일해야 함(E10.3), 파이썬 참조 구현 값과도
//! 대조

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use zipmania_archive::backend::unegg::Unegg;
use zipmania_archive::{ArchiveBackend, ExtractOptions, ExtractResult, OverwriteMode};

/// 파이썬 참조 구현과 실측이 일치한 값(README §10)
const ENTRY_NAME: &str = "EGG_Specification.pdf";
const UNPACKED_SIZE: u64 = 813_142;
const PACKED_SIZE: u64 = 716_715;
const CRC: &str = "6894DE48";
const MTIME: &str = "2016-09-19 14:34:06";

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn tempdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zipmania_unegg_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn egg_목록() {
    let entries = Unegg::new().list(&fixture("test.egg"), None).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.path, ENTRY_NAME);
    assert_eq!(e.size, UNPACKED_SIZE);
    assert_eq!(e.packed_size, PACKED_SIZE);
    assert_eq!(e.crc.as_deref(), Some(CRC));
    assert_eq!(e.modified, MTIME); // FILETIME 경로
    assert!(!e.is_dir);
}

#[test]
fn alz_목록() {
    let entries = Unegg::new().list(&fixture("test.alz"), None).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.path, ENTRY_NAME);
    assert_eq!(e.size, UNPACKED_SIZE);
    assert_eq!(e.packed_size, PACKED_SIZE);
    assert_eq!(e.crc.as_deref(), Some(CRC));
    assert_eq!(e.modified, MTIME); // DOS date/time 경로 — 서로 다른 표현이 같은 시각으로 도착
}

#[test]
fn 두_포맷의_해제_결과가_바이트로_동일하다() {
    let be = Unegg::new();
    let from_egg = be
        .read_entry_to_memory(&fixture("test.egg"), ENTRY_NAME, None)
        .unwrap();
    let from_alz = be
        .read_entry_to_memory(&fixture("test.alz"), ENTRY_NAME, None)
        .unwrap();

    assert_eq!(from_egg.len(), UNPACKED_SIZE as usize);
    assert_eq!(from_egg, from_alz, "EGG/ALZ 해제 결과가 다르다");
    // 내용물의 PDF 여부도 확인(엉뚱한 오프셋 해제 시 여기서 검출)
    assert_eq!(&from_egg[..8], b"%PDF-1.5");
    assert!(from_egg.ends_with(b"%%EOF\n") || from_egg.ends_with(b"\n%%EOF"));
}

#[test]
fn crc_검증이_실제로_동작한다() {
    // read_item 이 블록 CRC 를 검사 → test 통과 = CRC 일치
    Unegg::new().test(&fixture("test.egg"), None).unwrap();
    Unegg::new().test(&fixture("test.alz"), None).unwrap();
}

#[test]
fn egg_해제하면_파일이_생긴다() {
    let dest = tempdir("egg_x");
    let opts = ExtractOptions {
        archive: fixture("test.egg"),
        dest: dest.to_string_lossy().into_owned(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        password: None,
        selected: Vec::new(),
        decisions: Default::default(),
    };
    let mut seen_percent = 0u8;
    let mut prog = |p: u8, _f: Option<String>| seen_percent = seen_percent.max(p);
    match Unegg::new().extract(&opts, &mut prog, Arc::new(AtomicBool::new(false))) {
        ExtractResult::Done { status, .. } => assert_eq!(status, "ok"),
        ExtractResult::Failed(e) => panic!("해제 실패: {} {}", e.code, e.message),
    }
    let out = dest.join(ENTRY_NAME);
    assert!(out.exists(), "해제 파일이 없다");
    assert_eq!(std::fs::metadata(&out).unwrap().len(), UNPACKED_SIZE);
    assert_eq!(seen_percent, 100, "진행률이 100 으로 마감되지 않았다");
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn 이미_있는_파일은_충돌_정책을_따른다() {
    let dest = tempdir("egg_conflict");
    std::fs::write(dest.join(ENTRY_NAME), "기존 내용").unwrap();

    let mut opts = ExtractOptions {
        archive: fixture("test.egg"),
        dest: dest.to_string_lossy().into_owned(),
        keep_paths: true,
        overwrite: OverwriteMode::Skip,
        password: None,
        selected: Vec::new(),
        decisions: Default::default(),
    };
    let mut prog = |_p: u8, _f: Option<String>| {};

    // Skip — 기존 파일 보존
    let _ = Unegg::new().extract(&opts, &mut prog, Arc::new(AtomicBool::new(false)));
    assert_eq!(
        std::fs::read(dest.join(ENTRY_NAME)).unwrap(),
        "기존 내용".as_bytes()
    );

    // Rename — 기존은 그대로, 새 이름으로 저장
    opts.overwrite = OverwriteMode::Rename;
    let _ = Unegg::new().extract(&opts, &mut prog, Arc::new(AtomicBool::new(false)));
    assert_eq!(
        std::fs::read(dest.join(ENTRY_NAME)).unwrap(),
        "기존 내용".as_bytes()
    );
    assert!(dest.join("EGG_Specification (2).pdf").exists());

    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn 아카이브가_아니면_명확히_거부한다() {
    let dir = tempdir("notarc");
    let f = dir.join("plain.egg");
    std::fs::write(&f, b"not an archive at all").unwrap();
    let e = Unegg::new()
        .list(&f.to_string_lossy(), None)
        .unwrap_err();
    assert_eq!(e.code, "unsupported");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 잘린_아카이브는_패닉하지_않는다() {
    let dir = tempdir("trunc");
    let full = std::fs::read(fixture("test.egg")).unwrap();
    for cut in [20usize, 60, 100, 5000] {
        let f = dir.join(format!("cut{cut}.egg"));
        std::fs::write(&f, &full[..cut]).unwrap();
        // 성공이든 실패든 패닉만 없으면 충족
        let _ = Unegg::new().list(&f.to_string_lossy(), None);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ───────────────────── 암호(ZipCrypto) 아카이브 — test2.* ─────────────────────
//
// test2.egg / test2.alz = 비밀번호 test 로 생성

const PASSWORD: &str = "test";

#[test]
fn 암호_아카이브도_목록은_읽힌다() {
    // 파일명 미암호화 시 비밀번호 없이도 목록 산출
    for f in ["test2.egg", "test2.alz"] {
        let entries = Unegg::new().list(&fixture(f), None).unwrap();
        assert!(!entries.is_empty(), "{f}: 목록이 비었다");
        eprintln!("{f}: {} 항목, 첫 항목 = {}", entries.len(), entries[0].path);
    }
}

#[test]
fn 비밀번호_없이_읽으면_password_required() {
    for f in ["test2.egg", "test2.alz"] {
        let entries = Unegg::new().list(&fixture(f), None).unwrap();
        let name = &entries.iter().find(|e| !e.is_dir).unwrap().path;
        let e = Unegg::new()
            .read_entry_to_memory(&fixture(f), name, None)
            .unwrap_err();
        assert_eq!(e.code, "password_required", "{f}");
    }
}

#[test]
fn 틀린_비밀번호는_wrong_password() {
    for f in ["test2.egg", "test2.alz"] {
        let entries = Unegg::new().list(&fixture(f), None).unwrap();
        let name = &entries.iter().find(|e| !e.is_dir).unwrap().path;
        let e = Unegg::new()
            .read_entry_to_memory(&fixture(f), name, Some("nope"))
            .unwrap_err();
        assert_eq!(e.code, "wrong_password", "{f}");
    }
}

#[test]
fn 올바른_비밀번호로_풀리고_crc가_맞는다() {
    for f in ["test2.egg", "test2.alz"] {
        let entries = Unegg::new().list(&fixture(f), None).unwrap();
        let file = entries.iter().find(|e| !e.is_dir).unwrap();
        let bytes = Unegg::new()
            .read_entry_to_memory(&fixture(f), &file.path, Some(PASSWORD))
            .unwrap();
        assert_eq!(bytes.len() as u64, file.size, "{f}: 크기 불일치");
        // test 통과 = 모든 항목의 CRC 일치
        Unegg::new().test(&fixture(f), Some(PASSWORD)).unwrap();
    }
}

#[test]
fn 암호_아카이브_해제() {
    let dest = tempdir("enc_x");
    let opts = ExtractOptions {
        archive: fixture("test2.egg"),
        dest: dest.to_string_lossy().into_owned(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        password: Some(PASSWORD.to_string()),
        selected: Vec::new(),
        decisions: Default::default(),
    };
    let mut prog = |_p: u8, _f: Option<String>| {};
    match Unegg::new().extract(&opts, &mut prog, Arc::new(AtomicBool::new(false))) {
        ExtractResult::Done { status, .. } => assert_eq!(status, "ok"),
        ExtractResult::Failed(e) => panic!("해제 실패: {} {}", e.code, e.message),
    }
    let produced: Vec<_> = std::fs::read_dir(&dest)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!produced.is_empty(), "해제 결과가 없다");
    eprintln!("암호 EGG 해제 산출물: {produced:?}");
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn 비밀번호가_틀리면_해제를_즉시_중단한다() {
    let dest = tempdir("enc_bad");
    let opts = ExtractOptions {
        archive: fixture("test2.alz"),
        dest: dest.to_string_lossy().into_owned(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        password: Some("nope".into()),
        selected: Vec::new(),
        decisions: Default::default(),
    };
    let mut prog = |_p: u8, _f: Option<String>| {};
    match Unegg::new().extract(&opts, &mut prog, Arc::new(AtomicBool::new(false))) {
        ExtractResult::Failed(e) => assert_eq!(e.code, "wrong_password"),
        ExtractResult::Done { status, .. } => panic!("틀린 암호인데 {status} 로 끝났다"),
    }
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn 충돌_검사가_암호_아카이브에서도_동작한다() {
    // 빠른 해제는 해제 전에 find_conflicts 를 부른다, 여기서 실패하거나 패닉하면 UI 는 암호를
    // 묻기도 전에 멈춘다
    let dest = tempdir("conf_enc");
    for f in ["test2.egg", "test2.alz"] {
        let r = Unegg::new().find_conflicts(&fixture(f), &dest.to_string_lossy(), true, &[], None);
        match r {
            Ok(v) => eprintln!("{f}: 충돌 {}건", v.len()),
            Err(e) => panic!("{f}: find_conflicts 실패 {} {}", e.code, e.message),
        }
    }
    let _ = std::fs::remove_dir_all(&dest);
}

/// 목록이 신고한 크기와 실제가 다르면 ok 가 아니다
/// EGG 는 헤더 size 와 블록 unpacked_size 를 별도 기재, CRC 는 나온 바이트 기준이라 통과
/// 검사는 commit 보다 먼저 — 이동 후 경고는 되돌릴 것 없음, 기존 파일 보존까지 확인
#[test]
fn 목록이_부풀린_크기는_쓰지_않는다() {
    const SIG_FILE: [u8; 4] = 0x0A85_90E3u32.to_le_bytes();

    let dest = tempdir("egg_size_lie");
    let mut b = std::fs::read(fixture("test.egg")).unwrap();

    // 파일 헤더 = SIG_FILE(4) | id(4) | size(8), 그 size 만 부풀림 (블록의
    // unpacked_size, CRC, 데이터는 미변경이므로 해제 자체는 성공)
    let mut patched = 0;
    let mut i = 0;
    while i + 16 <= b.len() {
        if b[i..i + 4] == SIG_FILE && b[i + 8..i + 16] == UNPACKED_SIZE.to_le_bytes() {
            b[i + 8..i + 16].copy_from_slice(&(UNPACKED_SIZE * 2).to_le_bytes());
            patched += 1;
        }
        i += 1;
    }
    assert_eq!(patched, 1, "파일 헤더의 크기 필드를 찾지 못했다");

    let arc = dest.join("size_lie.egg");
    std::fs::write(&arc, &b).unwrap();

    // 덮어쓸 기존 파일 배치 — 이것의 생존 확인
    let out = dest.join(ENTRY_NAME);
    std::fs::write(&out, "기존 내용").unwrap();

    let opts = ExtractOptions {
        archive: arc.to_string_lossy().into_owned(),
        dest: dest.to_string_lossy().into_owned(),
        keep_paths: true,
        overwrite: OverwriteMode::Overwrite,
        password: None,
        selected: Vec::new(),
        decisions: Default::default(),
    };
    match Unegg::new().extract(&opts, &mut |_, _| {}, Arc::new(AtomicBool::new(false))) {
        ExtractResult::Done { status, message } => {
            assert_eq!(status, "warning", "부풀린 크기를 {status} 로 마감했다: {message}");
        }
        ExtractResult::Failed(e) => panic!("해제 실패: {} {}", e.code, e.message),
    }
    assert_eq!(
        std::fs::read(&out).unwrap(),
        "기존 내용".as_bytes(),
        "모순된 항목이 기존 파일을 덮었다"
    );

    // 임시 파일도 남지 않는다(abort 가 치운다)
    let left: Vec<String> = std::fs::read_dir(&dest)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".zmtmp-"))
        .collect();
    assert!(left.is_empty(), "임시 파일이 남았다: {left:?}");

    let _ = std::fs::remove_dir_all(&dest);
}
