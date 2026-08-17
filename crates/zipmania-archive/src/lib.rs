//! zipmania-archive — ZipMania 압축모듈(재사용 라이브러리)
//! ArchiveBackend 트레이트 + 확장자로 고르는 Router, 앱 프레임워크 무의존 → ZipMania.exe, WorldView 공용
//!
//! 전 플랫폼 컴파일, OS 의존 = backend::sevenzip 하나
//! 백엔드 없는 플랫폼 = 작업 unsupported, 타입과 상수는 사용 가능 (D3.5)

pub mod backend;
pub mod crc32;
pub mod error;
pub mod formats;
pub mod inputs;
pub mod models;
pub mod outfile;
pub mod paths;

// ── 공개 API 재노출 ──
#[cfg(windows)]
pub use backend::sevenzip::SevenZip;
pub use backend::unegg::Unegg;
pub use backend::unzip::Unzip;
pub use backend::{
    ArchiveBackend, CreateOptions, CreateResult, EditOptions, ExtractOptions, ExtractResult, Router,
};
pub use error::{ZipManiaError, SevenZipError};
pub use formats::{is_archive_path, CompressFormat, OverwriteMode, ScanFn, READ_EXTS};
pub use models::{ArchiveEntry, ScanEntry, TestEntry};
