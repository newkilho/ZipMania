//! 압축 입력(파일, 폴더) 재귀 수집, 백엔드 공용(7z, zip), 사본 금지
//! 링크 판정 = symlink_metadata, 미추적(최상위 입력만 예외)
//! 누락 항목 = collect 가 함께 반환 → 호출측이 warning 으로 마감 (D3.5)

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::backend::{MissingItem, MissingReason};

/// 압축 항목 1개, 백엔드가 자기 표현으로 변환
#[derive(Debug, Clone)]
pub struct InputItem {
    pub name: String,
    pub source: Option<PathBuf>,
    pub size: u64,
    pub is_dir: bool,
    pub mtime: Option<SystemTime>,
}

/// 폴더와 파일 입력 재귀 수집, 2번째 값 = 누락분(링크, 읽기 실패, 이름 없음)
pub fn collect(inputs: &[String]) -> (Vec<InputItem>, Vec<MissingItem>) {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for input in inputs {
        let p = Path::new(input);
        let base = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // 이름 못 뽑는 입력(드라이브 루트 C:\, ..), 조용히 넘기면 결과가 ok → 원본 삭제 허용
        if base.is_empty() {
            skipped.push(MissingItem::new(input, MissingReason::Unnamed));
            continue;
        }
        // 최상위 입력 = 링크여도 추적(사용자 선택), 하위 링크 = 제외
        if p.is_dir() {
            add_dir(&mut items, p, &base, &mut skipped);
        } else if p.is_file() {
            let meta = std::fs::metadata(p).ok();
            items.push(InputItem {
                name: base,
                source: Some(p.to_path_buf()),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                is_dir: false,
                mtime: meta.and_then(|m| m.modified().ok()),
            });
        } else {
            // 폴더도 파일도 아님(소실, 읽기 실패) → 누락 보고, 조용히 넘기면 ok
            skipped.push(MissingItem::new(input, MissingReason::Missing));
        }
    }
    (items, skipped)
}

fn add_dir(items: &mut Vec<InputItem>, dir: &Path, rel: &str, skipped: &mut Vec<MissingItem>) {
    items.push(InputItem {
        name: rel.to_string(),
        source: None,
        size: 0,
        is_dir: true,
        mtime: std::fs::metadata(dir).and_then(|m| m.modified()).ok(),
    });
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            skipped.push(MissingItem::detailed(rel, MissingReason::DirRead, e.to_string()));
            return;
        }
    };
    let sep = std::path::MAIN_SEPARATOR;
    for entry in read {
        let entry = match entry {
            Ok(x) => x,
            Err(e) => {
                skipped.push(MissingItem::detailed(rel, MissingReason::EntryRead, e.to_string()));
                continue;
            }
        };
        let child = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let child_rel = format!("{rel}{sep}{name}");

        let meta = match std::fs::symlink_metadata(&child) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(MissingItem::detailed(
                    &child_rel,
                    MissingReason::Stat,
                    e.to_string(),
                ));
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            skipped.push(MissingItem::new(&child_rel, MissingReason::Link));
            continue;
        }

        if meta.is_dir() {
            add_dir(items, &child, &child_rel, skipped);
        } else if meta.is_file() {
            items.push(InputItem {
                name: child_rel,
                source: Some(child),
                size: meta.len(),
                is_dir: false,
                mtime: meta.modified().ok(),
            });
        }
    }
}

