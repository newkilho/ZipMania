//! UI 직렬화 데이터 구조체, 이벤트 페이로드는 앱(src-tauri) 몫

use serde::{Deserialize, Serialize};

/// 아카이브 내 항목 하나
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    pub path: String,
    pub size: u64,
    pub packed_size: u64,
    pub modified: String,
    pub is_dir: bool,
    pub crc: Option<String>,
}

/// 무결성 테스트 한 항목의 결과
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestEntry {
    pub path: String,
    pub is_dir: bool,
    pub expected_crc: Option<u32>,
    pub actual_crc: Option<u32>,
    pub ok: bool,
}

/// 바이러스 검사 한 항목의 결과(AMSI)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub status: String,
}
