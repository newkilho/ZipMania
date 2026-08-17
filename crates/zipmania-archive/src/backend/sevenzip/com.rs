//! 7z.dll COM 인터페이스 선언 + 공통 상수, IID 패턴 = {23170F69-40C1-278A-0000-<i>00<j>0000}
//! 호출: IInArchive, IOutArchive, ISetProperties
//! 구현: 스트림, 콜백, 선언만 여기, 구현체는 streams, callbacks

#![allow(non_camel_case_types)]

use std::ffi::c_void;

use windows_core::{interface, HRESULT, IUnknown, IUnknown_Vtbl};

use super::prop::PropVariant;

// ── HRESULT 상수 ──
/// 성공
pub const S_OK: HRESULT = HRESULT(0);
/// 성공이나 거짓, 예: 헤더암호 아카이브를 암호 없이 Open
pub const S_FALSE: HRESULT = HRESULT(1);
/// 사용자 취소, 콜백에서 반환 → 작업 중단
pub const E_ABORT: HRESULT = HRESULT(0x8000_4004u32 as i32);
/// 잘못된 인자
pub const E_INVALIDARG: HRESULT = HRESULT(0x8007_0057u32 as i32);

// ── 항목 속성 ID(kpid) ──
pub const KPID_PATH: u32 = 3;
pub const KPID_IS_DIR: u32 = 6;
pub const KPID_SIZE: u32 = 7;
pub const KPID_PACK_SIZE: u32 = 8;
pub const KPID_ATTRIB: u32 = 9;
pub const KPID_MTIME: u32 = 12;
pub const KPID_CRC: u32 = 19;

// ── Extract 콜백 askMode ──
pub const ASK_EXTRACT: i32 = 0;
pub const ASK_TEST: i32 = 1;
pub const ASK_SKIP: i32 = 2;

// ── Seek 원점(STREAM_SEEK_*) ──
pub const STREAM_SEEK_SET: u32 = 0;
pub const STREAM_SEEK_CUR: u32 = 1;
pub const STREAM_SEEK_END: u32 = 2;

// ===========================================================================
// 우리가 호출하는 인터페이스
// ===========================================================================

/// 읽기(목록, 해제) 아카이브 핸들러
#[interface("23170F69-40C1-278A-0000-000600600000")]
pub unsafe trait IInArchive: IUnknown {
    pub unsafe fn Open(&self, stream: *mut c_void, max_check: *const u64, cb: *mut c_void)
        -> HRESULT;
    pub unsafe fn Close(&self) -> HRESULT;
    pub unsafe fn GetNumberOfItems(&self, num: *mut u32) -> HRESULT;
    pub unsafe fn GetProperty(&self, index: u32, prop_id: u32, value: *mut PropVariant) -> HRESULT;
    pub unsafe fn Extract(
        &self,
        indices: *const u32,
        num: u32,
        test_mode: i32,
        cb: *mut c_void,
    ) -> HRESULT;
}

/// 쓰기(생성) 아카이브 핸들러
#[interface("23170F69-40C1-278A-0000-000600A00000")]
pub unsafe trait IOutArchive: IUnknown {
    pub unsafe fn UpdateItems(&self, out_stream: *mut c_void, num_items: u32, cb: *mut c_void)
        -> HRESULT;
}

/// 압축 옵션 설정(레벨, 헤더암호 등)
#[interface("23170F69-40C1-278A-0000-000600030000")]
pub unsafe trait ISetProperties: IUnknown {
    pub unsafe fn SetProperties(
        &self,
        names: *const *const u16,
        values: *const PropVariant,
        num: u32,
    ) -> HRESULT;
}

// ===========================================================================
// 우리가 구현하는 인터페이스 (스트림)
// ===========================================================================

/// 순차 입력 스트림
#[interface("23170F69-40C1-278A-0000-000300010000")]
pub unsafe trait ISequentialInStream: IUnknown {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT;
}

/// 탐색 가능 입력 스트림, 아카이브 열기, 생성 소스용
#[interface("23170F69-40C1-278A-0000-000300030000")]
pub unsafe trait IInStream: IUnknown {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT;
    unsafe fn Seek(&self, offset: i64, origin: u32, new_pos: *mut u64) -> HRESULT;
}

/// 입력 스트림 크기 조회, 생성 소스가 노출
#[interface("23170F69-40C1-278A-0000-000300060000")]
pub unsafe trait IStreamGetSize: IUnknown {
    unsafe fn GetSize(&self, size: *mut u64) -> HRESULT;
}

/// 순차 출력 스트림(해제 대상)
#[interface("23170F69-40C1-278A-0000-000300020000")]
pub unsafe trait ISequentialOutStream: IUnknown {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT;
}

/// 탐색 가능 출력 스트림, 7z 생성 필수 — 인코더가 헤더를 되감아 패치
#[interface("23170F69-40C1-278A-0000-000300040000")]
pub unsafe trait IOutStream: IUnknown {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT;
    unsafe fn Seek(&self, offset: i64, origin: u32, new_pos: *mut u64) -> HRESULT;
    unsafe fn SetSize(&self, new_size: u64) -> HRESULT;
}

// ===========================================================================
// 우리가 구현하는 인터페이스 (콜백)
// ===========================================================================

/// 아카이브 열기 콜백, 헤더암호 아카이브의 진행 상황, 암호 공급
#[interface("23170F69-40C1-278A-0000-000600100000")]
pub unsafe trait IArchiveOpenCallback: IUnknown {
    unsafe fn SetTotal(&self, files: *const u64, bytes: *const u64) -> HRESULT;
    unsafe fn SetCompleted(&self, files: *const u64, bytes: *const u64) -> HRESULT;
}

/// 해제 콜백, IProgress 평탄화 포함
#[interface("23170F69-40C1-278A-0000-000600200000")]
pub unsafe trait IArchiveExtractCallback: IUnknown {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT;
    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT;
    unsafe fn GetStream(
        &self,
        index: u32,
        out_stream: *mut *mut c_void,
        ask_mode: i32,
    ) -> HRESULT;
    unsafe fn PrepareOperation(&self, ask_mode: i32) -> HRESULT;
    unsafe fn SetOperationResult(&self, op_result: i32) -> HRESULT;
}

/// 생성 콜백, IProgress 평탄화 포함
#[interface("23170F69-40C1-278A-0000-000600800000")]
pub unsafe trait IArchiveUpdateCallback: IUnknown {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT;
    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT;
    unsafe fn GetUpdateItemInfo(
        &self,
        index: u32,
        new_data: *mut i32,
        new_props: *mut i32,
        index_in_archive: *mut u32,
    ) -> HRESULT;
    unsafe fn GetProperty(&self, index: u32, prop_id: u32, value: *mut PropVariant) -> HRESULT;
    unsafe fn GetStream(&self, index: u32, in_stream: *mut *mut c_void) -> HRESULT;
    unsafe fn SetOperationResult(&self, op_result: i32) -> HRESULT;
}

/// 해제, 열기용 암호 공급
#[interface("23170F69-40C1-278A-0000-000500100000")]
pub unsafe trait ICryptoGetTextPassword: IUnknown {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *const u16) -> HRESULT;
}

/// 생성용 암호 공급, 정의 여부 + 암호
#[interface("23170F69-40C1-278A-0000-000500110000")]
pub unsafe trait ICryptoGetTextPassword2: IUnknown {
    unsafe fn CryptoGetTextPassword2(
        &self,
        password_is_defined: *mut i32,
        password: *mut *const u16,
    ) -> HRESULT;
}
