//! ZIP 해제 엔진, 담당: 경로 안전, 충돌 정책, 취소, 진행률, 누락 보고
//! 경로 = 백엔드 공용 crate::paths, 크레이트 enclosed_name() 사용 금지

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::backend::{ExtractOptions, ExtractResult, ProgressFn};
use crate::error::ZipManiaError;
use crate::formats::{unique_path, OverwriteMode};
use crate::inputs::summarize;
use crate::paths;

use super::{canceled, entry_name, open_entry, percent, Archive};

/// 항목의 선택 범위 포함 여부, 비면 전체
fn in_scope(path: &str, selected: &[String]) -> bool {
    if selected.is_empty() {
        return true;
    }
    selected.iter().any(|s| {
        let s = s.replace('\\', "/");
        path == s || path.starts_with(&format!("{s}/"))
    })
}

/// 아카이브 → opts.dest 하위로 해제
pub fn extract_all(
    mut ar: Archive,
    opts: &ExtractOptions,
    on_progress: &mut ProgressFn<'_>,
    cancel: Arc<AtomicBool>,
) -> ExtractResult {
    let dest = Path::new(&opts.dest);
    if let Err(e) = fs::create_dir_all(dest) {
        return ExtractResult::Failed(ZipManiaError::new(
            "io_error",
            format!("대상 폴더를 만들지 못했습니다: {e}"),
        ));
    }

    // 진행률 분모
    let mut targets: Vec<(usize, String, bool, u64)> = Vec::new();
    for i in 0..ar.len() {
        let Ok(f) = ar.by_index_raw(i) else {
            return ExtractResult::Failed(ZipManiaError::new(
                "corrupt",
                "항목 헤더를 읽지 못했습니다.",
            ));
        };
        let name = entry_name(&f);
        if in_scope(&name, &opts.selected) {
            targets.push((i, name, f.is_dir(), f.size()));
        }
    }
    let total: u64 = targets.iter().filter(|t| !t.2).map(|t| t.3).sum();
    let mut done = 0u64;

    // 미기록 항목 기록 필수, ok = 전부 풀림 → 앱이 그 값으로 [해제 후 원본 삭제] 수행
    let mut unsafe_paths: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    // 신고 크기 != 실제 출력 크기인 항목(풀리긴 함)
    let mut mismatched: Vec<String> = Vec::new();
    let mut last_parent: Option<std::path::PathBuf> = None;

    for (index, name, is_dir, size) in targets {
        if canceled(&cancel) {
            return ExtractResult::Done {
                status: "canceled",
                message: "사용자가 취소했습니다.".into(),
            };
        }

        let rel_name = if opts.keep_paths {
            name.clone()
        } else {
            name.rsplit('/').next().unwrap_or(&name).to_string()
        };

        // 안전하지 않은 항목 = 거부 아니라 건너뛰고 기록(하나가 전체를 죽이지 않게)
        let target = match paths::sanitize(&rel_name)
            .and_then(|rel| paths::resolve_under(dest, &rel))
        {
            Ok(t) => t,
            Err(_) => {
                unsafe_paths.push(name.clone());
                continue;
            }
        };

        if is_dir {
            if let Err(e) = fs::create_dir_all(&target) {
                failed.push(format!("{name} ({e})"));
            }
            continue;
        }

        // 파일별 선택 > 기본 정책
        let target = if target.exists() {
            match opts.decisions.get(&name).copied().unwrap_or(opts.overwrite) {
                OverwriteMode::Skip => continue,
                OverwriteMode::Rename => unique_path(&target),
                OverwriteMode::Overwrite => target,
            }
        } else {
            target
        };

        on_progress(percent(done, total), Some(name.clone()));

        let mut f = match open_entry(&mut ar, index, opts.password.as_deref()) {
            Ok(f) => f,
            // 암호 = 전역 실패 → 즉시 중단, UI 가 재질의
            Err(e) if e.code == "password_required" || e.code == "wrong_password" => {
                return ExtractResult::Failed(e)
            }
            Err(e) => {
                failed.push(format!("{name} ({})", e.message));
                continue;
            }
        };

        // 부모 폴더 생성은 직전과 다를 때만, 파일마다 호출 시 항목 수만큼 왕복(3000개에서 체감)
        if let Some(parent) = target.parent() {
            if last_parent.as_deref() != Some(parent) {
                if let Err(e) = fs::create_dir_all(parent) {
                    failed.push(format!("{name} (폴더를 만들지 못함: {e})"));
                    continue;
                }
                last_parent = Some(parent.to_path_buf());
            }
        }
        // 기존 파일 truncate 금지, 임시 파일에 쓰고 전체 성공 시에만 이동
        let (out, staged) = match crate::outfile::StagedFile::create(&target) {
            Ok(v) => v,
            Err(e) => {
                failed.push(format!("{name} (파일을 만들지 못함: {e})"));
                continue;
            }
        };
        let mut out = std::io::BufWriter::new(out);

        // 디스크 경로에는 크기 상한 미적용(큰 파일 해제는 정상, 메모리 미적재), 대신 청크마다 취소 확인
        let mut buf = [0u8; 64 * 1024];
        let mut wrote = 0u64;
        let mut broke = None;
        loop {
            if canceled(&cancel) {
                drop(out);
                staged.abort();
                return ExtractResult::Done {
                    status: "canceled",
                    message: "사용자가 취소했습니다.".into(),
                };
            }
            let n = match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    broke = Some(format!("{e}"));
                    break;
                }
            };
            if let Err(e) = std::io::Write::write_all(&mut out, &buf[..n]) {
                broke = Some(format!("{e}"));
                break;
            }
            wrote += n as u64;
            done += n as u64;
            on_progress(percent(done, total), Some(name.clone()));
        }
        drop(f);
        if let Err(e) = std::io::Write::flush(&mut out) {
            broke.get_or_insert(format!("{e}"));
        }
        drop(out);

        if let Some(why) = broke {
            // 반쯤 쓰인 파일 잔류 금지, 임시 파일만 삭제, 원본 유지
            staged.abort();
            failed.push(format!("{name} ({why})"));
            continue;
        }
        // 신고 크기와 산출 크기 불일치 시 미이동(CRC 는 목록의 거짓말 미탐지)
        // 검사가 commit 보다 먼저 — 모순 항목은 미기록 + 누락 보고
        if size != wrote {
            staged.abort();
            mismatched.push(format!("{name} ({wrote}바이트, 목록은 {size}바이트)"));
            continue;
        }
        // 전체 기록 후 이동, 이동 실패 시 원본 유지 → 실패로 기록
        if let Err(e) = staged.commit() {
            failed.push(format!("{name} ({})", e.message));
            continue;
        }
    }

    on_progress(100, None);
    if !unsafe_paths.is_empty() || !failed.is_empty() || !mismatched.is_empty() {
        let mut parts = Vec::new();
        if !mismatched.is_empty() {
            parts.push(format!(
                "목록과 크기가 달라 쓰지 않은 항목 {}개({})",
                mismatched.len(),
                summarize(&mismatched)
            ));
        }
        if !unsafe_paths.is_empty() {
            parts.push(format!(
                "해제 폴더를 벗어나는 경로 {}개를 건너뛰었습니다({})",
                unsafe_paths.len(),
                summarize(&unsafe_paths)
            ));
        }
        if !failed.is_empty() {
            parts.push(format!(
                "쓰지 못한 항목 {}개({})",
                failed.len(),
                summarize(&failed)
            ));
        }
        return ExtractResult::Done {
            status: "warning",
            message: format!("해제를 마쳤지만 일부 항목이 빠졌습니다. {}.", parts.join(" / ")),
        };
    }
    ExtractResult::Done {
        status: "ok",
        message: "해제를 완료했습니다.".to_string(),
    }
}
