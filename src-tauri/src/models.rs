//! Tauri 이벤트 직렬화 페이로드, 목록 항목은 zipmania-archive 몫

#![allow(dead_code)]

use zipmania_archive::{ScanEntry, TestEntry};
use serde::{Deserialize, Serialize};

/// 무결성 테스트 결과(test:report), 요약 개수는 프런트가 계산
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReportEvent {
    pub job_id: String,
    pub entries: Vec<TestEntry>,
}

/// 바이러스 검사 결과(scan:report)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReportEvent {
    pub job_id: String,
    pub entries: Vec<ScanEntry>,
}

/// 작업 진행률, (job:progress 이벤트 페이로드)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: String,
    pub percent: u8,
    pub current_file: String,
}

/// 작업 완료(job:done), status = ok, warning(빠진 항목), canceled(부분 파일 잔존 가능)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDone {
    pub job_id: String,
    pub status: String,
    pub message: String,
}

/// 작업 오류(job:error), code = ZipManiaError 체계
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobErrorEvent {
    pub job_id: String,
    pub code: String,
    pub message: String,
}

/// 작업 시작(job:started, kind = compress/extract), job_id 반환 직전 전 창에 방송
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStarted {
    pub job_id: String,
    pub kind: String,
}

/// 해제 창 초기 컨텍스트(take_extract_context 반환), 창 생성 직전 적재 → mount 때 1회 회수
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractContext {
    pub archive: String,
    pub selected: Vec<String>,
    pub auto_start: bool,
    pub dest: Option<String>,
    #[serde(default)]
    pub batch: Vec<ExtractBatchItem>,
}

/// "각각 풀기" 배치의 항목 하나(아카이브 → 최종 대상 폴더)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractBatchItem {
    pub archive: String,
    pub dest: String,
}

/// 압축 창 초기 컨텍스트(take_compress_inputs 반환), 일반 = inputs 만, 즉시 zip = format, output, auto_start 까지
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressLaunch {
    pub inputs: Vec<String>,
    pub format: Option<String>,
    pub output: Option<String>,
    pub auto_start: bool,
    #[serde(default)]
    pub batch: Vec<CompressBatchItem>,
}

/// lease_compress_launch 반환값, 요청 1개 + more(잔여 여부), more 부재 시 뒤엣것이 큐에 잔류(D3.5)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressTake {
    pub id: u64,
    pub gen: u64,
    pub launch: Option<CompressLaunch>,
    pub more: bool,
}

/// "각각 압축" 배치의 항목 하나(원본 → 출력 zip 경로)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressBatchItem {
    pub input: String,
    pub output: String,
}

/// 경로 표시용 메타데이터(stat_paths 반환), 디렉터리 = 재귀 없음, size 0
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
}

/// 폴더 하위 파일 하나(list_folder_files 반환 항목)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderFile {
    pub rel: String,
    pub size: u64,
}

/// 디렉터리 트리 노드(list_dir_children 반환 항목), 파일 제외, 지연 로딩
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirNode {
    pub name: String,
    pub path: String,
    pub has_children: bool,
}

/// 즐겨찾기 항목(list_quick_access 반환), 라벨 번역은 프런트가 kind 로
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickAccess {
    pub kind: String,
    pub name: String,
    pub path: String,
}
