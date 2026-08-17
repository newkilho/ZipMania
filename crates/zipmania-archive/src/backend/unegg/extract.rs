//! 해제 엔진, 정규화된 항목 목록 → 디스크 기록
//! 담당: 경로 안전, 충돌 정책, 취소, 진행률, 경로 = 백엔드 공용 crate::paths

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::backend::{ExtractOptions, ExtractResult, ProgressFn};
use crate::error::ZipManiaError;
use crate::formats::{unique_path, OverwriteMode};
use crate::inputs::summarize;

use crate::paths;

use super::Item;

/// 취소 여부
fn canceled(cancel: &Arc<AtomicBool>) -> bool {
    cancel.load(Ordering::Relaxed)
}

/// 쓰기 조각마다 취소 확인하는 writer
/// 항목 사이에서만 보면 블록 1개가 수 GiB 인 아카이브에서 [취소] 가 수 분간 안 먹음
struct CancelWriter<'a, W: Write> {
    inner: W,
    cancel: &'a Arc<AtomicBool>,
}

impl<W: Write> Write for CancelWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if canceled(self.cancel) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "사용자가 취소했습니다.",
            ));
        }
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// 선택 범위 항목만 남김, 비면 전체
fn in_scope(item: &Item, selected: &[String]) -> bool {
    if selected.is_empty() {
        return true;
    }
    selected.iter().any(|s| {
        let s = s.replace('\\', "/");
        item.path == s || item.path.starts_with(&format!("{s}/"))
    })
}

/// 항목 → opts.dest 하위로 해제
pub fn extract_all(
    data: &[u8],
    items: &[Item],
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

    let targets: Vec<&Item> = items.iter().filter(|i| in_scope(i, &opts.selected)).collect();
    let total: u64 = targets.iter().filter(|i| !i.is_dir).map(|i| i.size).sum();
    let mut done: u64 = 0;
    let mut warned = false;
    // 신고 크기 != 실제 출력 크기라 기록하지 않은 항목
    let mut mismatched: Vec<String> = Vec::new();
    // 아예 생성 못 한 항목(폴더 생성 실패 등)
    let mut failed: Vec<String> = Vec::new();

    for item in targets {
        if canceled(&cancel) {
            return ExtractResult::Done {
                status: "canceled",
                message: "사용자가 취소했습니다.".into(),
            };
        }

        // 경로 유지 여부로 상대 경로 결정(평면 = 파일명만)
        let name = if opts.keep_paths {
            item.path.clone()
        } else {
            item.path.rsplit('/').next().unwrap_or(&item.path).to_string()
        };
        let rel = match paths::sanitize(&name) {
            Ok(p) => p,
            Err(e) => return ExtractResult::Failed(e),
        };
        let target = match paths::resolve_under(dest, &rel) {
            Ok(p) => p,
            Err(e) => return ExtractResult::Failed(e),
        };

        if item.is_dir {
            // 폴더 생성 실패도 누락 항목, 넘기면 ok → 앱이 원본 삭제(빈 폴더 정보 소실)
            // zip, 7z 도 이 실패를 기록
            if let Err(e) = fs::create_dir_all(&target) {
                failed.push(format!("{} ({e})", item.path));
            }
            continue;
        }

        // 충돌 정책: 파일별 선택 > 기본 정책
        let target = if target.exists() {
            match opts
                .decisions
                .get(&item.path)
                .copied()
                .unwrap_or(opts.overwrite)
            {
                OverwriteMode::Skip => continue,
                OverwriteMode::Rename => unique_path(&target),
                OverwriteMode::Overwrite => target,
            }
        } else {
            target
        };

        on_progress(
            percent(done, total),
            Some(item.path.clone()),
        );

        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // 기존 파일 truncate 없이 임시 파일에 기록(crate::outfile, 7z, zip 공용)
        // 내용은 블록 단위 스트리밍, Vec 수집 금지
        let (f, staged) = match crate::outfile::StagedFile::create(&target) {
            Ok(v) => v,
            Err(e) => {
                return ExtractResult::Failed(ZipManiaError::new(
                    "io_error",
                    format!("파일을 만들지 못했습니다({}): {e}", target.display()),
                ))
            }
        };
        // 조각마다 취소 확인 → 큰 파일 해제 중에도 [취소] 즉시 반응
        let mut out = CancelWriter {
            inner: std::io::BufWriter::new(f),
            cancel: &cancel,
        };
        let read = super::read_item_to(data, item, opts.password.as_deref(), 0, &mut out)
            .and_then(|n| {
                out.flush().map_err(|e| {
                    ZipManiaError::new("io_error", format!("파일을 쓰지 못했습니다: {e}"))
                })?;
                Ok(n)
            });
        drop(out);
        match read {
            Ok(wrote) => {
                // 신고 크기 != 출력 크기 → 이동 안 함(CRC 는 목록의 거짓말 미탐지)
                // 검사 순서: commit 보다 먼저, zip 백엔드도 같은 자리에서 동일 검사
                if wrote != item.size {
                    staged.abort();
                    mismatched.push(format!(
                        "{} ({wrote}바이트, 목록은 {}바이트)",
                        item.path, item.size
                    ));
                    // 미기록이어도 진행률상 지나간 항목, 빼면 끝에서 100 으로 튐
                    done += item.size;
                    continue;
                }
                if let Err(e) = staged.commit() {
                    return ExtractResult::Failed(e);
                }
            }
            // 개별 실패는 전체 중단 안 함(손상 아카이브 최대 복구), 암호는 전역 → 즉시 중단, UI 재질의
            Err(e) => {
                staged.abort();
                // 쓰기 도중 취소, 임시 파일은 위에서 삭제 → 잔류 없음
                if canceled(&cancel) {
                    return ExtractResult::Done {
                        status: "canceled",
                        message: "사용자가 취소했습니다.".into(),
                    };
                }
                if e.code == "password_required" || e.code == "wrong_password" {
                    return ExtractResult::Failed(e);
                }
                if e.code == "unsupported" || e.code == "corrupt" {
                    warned = true;
                    continue;
                }
                return ExtractResult::Failed(e);
            }
        }

        done += item.size;
    }

    on_progress(100, None);
    // 누락 항목 있으면 ok 아님, 앱이 ok 를 보고 [해제 후 원본 삭제] 수행
    if warned || !mismatched.is_empty() || !failed.is_empty() {
        let mut parts = Vec::new();
        if !mismatched.is_empty() {
            parts.push(format!(
                "목록과 크기가 달라 쓰지 않은 항목 {}개({})",
                mismatched.len(),
                summarize(&mismatched)
            ));
        }
        if !failed.is_empty() {
            parts.push(format!(
                "만들지 못한 항목 {}개({})",
                failed.len(),
                summarize(&failed)
            ));
        }
        if warned {
            parts.push("풀지 못한 항목(지원하지 않는 압축 방식이거나 손상)".to_string());
        }
        ExtractResult::Done {
            status: "warning",
            message: format!("해제를 마쳤지만 일부 항목이 빠졌습니다. {}.", parts.join(" / ")),
        }
    } else {
        ExtractResult::Done {
            status: "ok",
            message: "해제를 완료했습니다.".into(),
        }
    }
}

fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    ((done.saturating_mul(100) / total).min(100)) as u8
}


#[cfg(test)]
mod dir_tests {
    use super::*;
    use crate::backend::{ExtractOptions, ExtractResult};
    use crate::formats::OverwriteMode;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn dir_item(path: &str) -> Item {
        Item {
            path: path.to_string(),
            size: 0,
            packed_size: 0,
            crc: None,
            is_dir: true,
            modified: String::new(),
            enc: None,
            blocks: Vec::new(),
        }
    }

    /// 폴더 생성 실패도 누락 항목, create_dir_all 결과를 버리면 ok 가 나와 원본이 삭제됨
    #[test]
    fn 폴더를_만들지_못하면_경고한다() {
        let dest = std::env::temp_dir().join(format!("zm_eggdir_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);
        fs::create_dir_all(&dest).unwrap();
        // 폴더 자리에 파일 존재 → create_dir_all 실패
        fs::write(dest.join("docs"), "이미 있는 파일").unwrap();

        let opts = ExtractOptions {
            archive: String::new(),
            dest: dest.to_string_lossy().into_owned(),
            keep_paths: true,
            overwrite: OverwriteMode::Overwrite,
            password: None,
            selected: Vec::new(),
            decisions: Default::default(),
        };
        let items = [dir_item("docs")];
        let r = extract_all(&[], &items, &opts, &mut |_, _| {}, Arc::new(AtomicBool::new(false)));
        match r {
            ExtractResult::Done { status, message } => {
                assert_eq!(status, "warning", "폴더를 못 만들었는데 {status} 로 끝났다");
                assert!(message.contains("docs"), "무엇이 빠졌는지 알려 주지 않는다: {message}");
            }
            ExtractResult::Failed(e) => panic!("해제 실패: {} {}", e.code, e.message),
        }

        // 정상 폴더 = 생성 성공 + ok
        let items = [dir_item("사진")];
        let r = extract_all(&[], &items, &opts, &mut |_, _| {}, Arc::new(AtomicBool::new(false)));
        assert!(matches!(r, ExtractResult::Done { status: "ok", .. }));
        assert!(dest.join("사진").is_dir());

        let _ = fs::remove_dir_all(&dest);
    }
}
