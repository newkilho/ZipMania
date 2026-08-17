//! 7z.dll 로드 + CreateObject + GUID/CLSID
//! 번들 dll → 절대경로 LoadLibraryExW(LOAD_WITH_ALTERED_SEARCH_PATH) + GetProcAddress

use std::ffi::c_void;
use std::path::Path;

use windows_core::{Interface, GUID, HRESULT};

use super::com::{IInArchive, IOutArchive};
use crate::error::{ZipManiaError, SevenZipError};

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}
const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

// 파일 버전 리소스 조회(version.dll)
#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(filename: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(
        filename: *const u16,
        handle: u32,
        len: u32,
        data: *mut c_void,
    ) -> i32;
    fn VerQueryValueW(
        block: *const c_void,
        sub_block: *const u16,
        buffer: *mut *mut c_void,
        len: *mut u32,
    ) -> i32;
}

type CreateObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// 포맷 핸들러 CLSID = {23170F69-40C1-278A-1000-000110<XX>0000}, XX = 포맷 id, bit 16-23
pub fn clsid_format(id: u8) -> GUID {
    GUID::from_u128(0x23170F69_40C1_278A_1000_000110000000 | ((id as u128) << 16))
}

/// 문자열 → 널 종료 UTF-16
pub fn to_wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 로드된 7z.dll 핸들 + CreateObject 팩토리, FreeLibrary 없이 앱 수명 동안 유지
/// 원시 포인터 보유 → 스레드 간 이동 금지, 생성 스레드 전용
pub struct Dll {
    create_object: CreateObjectFn,
    _module: *mut c_void,
}

impl Dll {
    /// 절대경로의 7z.dll 로드
    pub fn load(path: &Path) -> Result<Dll, ZipManiaError> {
        let wide = to_wide_nul(&path.to_string_lossy());
        unsafe {
            let module =
                LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
            if module.is_null() {
                return Err(SevenZipError::Load(format!(
                    "7z.dll 로드 실패: {}",
                    path.display()
                ))
                .into());
            }
            let proc = GetProcAddress(module, b"CreateObject\0".as_ptr());
            if proc.is_null() {
                return Err(
                    SevenZipError::Load("CreateObject 심볼을 찾지 못했습니다".into()).into(),
                );
            }
            let create_object: CreateObjectFn = std::mem::transmute(proc);
            Ok(Dll {
                create_object,
                _module: module,
            })
        }
    }

    /// 포맷 id → 읽기 아카이브 객체 생성
    pub fn create_in_archive(&self, format_id: u8) -> Result<IInArchive, ZipManiaError> {
        let clsid = clsid_format(format_id);
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe {
            let hr = (self.create_object)(&clsid, &IInArchive::IID, &mut raw);
            if hr.0 != 0 || raw.is_null() {
                return Err(ZipManiaError::new(
                    "unsupported",
                    "이 포맷의 읽기 핸들러를 만들지 못했습니다.",
                ));
            }
            Ok(IInArchive::from_raw(raw))
        }
    }

    /// 포맷 id → 쓰기 아카이브 객체 생성
    pub fn create_out_archive(&self, format_id: u8) -> Result<IOutArchive, ZipManiaError> {
        let clsid = clsid_format(format_id);
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe {
            let hr = (self.create_object)(&clsid, &IOutArchive::IID, &mut raw);
            if hr.0 != 0 || raw.is_null() {
                return Err(ZipManiaError::new(
                    "unsupported",
                    "이 포맷의 쓰기 핸들러를 만들지 못했습니다.",
                ));
            }
            Ok(IOutArchive::from_raw(raw))
        }
    }
}

/// 7z.dll 버전 리소스 → 7-Zip <major>.<minor> (x64) 문자열, 조회 실패 시 None
pub fn dll_version_string(path: &Path) -> Option<String> {
    let wide = to_wide_nul(&path.to_string_lossy());
    unsafe {
        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buf.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        // \ 서브블록 → VS_FIXEDFILEINFO
        let sub = to_wide_nul("\\");
        let mut value: *mut c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        if VerQueryValueW(
            buf.as_ptr() as *const c_void,
            sub.as_ptr(),
            &mut value,
            &mut len,
        ) == 0
            || value.is_null()
        {
            return None;
        }
        // dwFileVersionMS = 오프셋 8. 상위16 = major, 하위16 = minor
        let ffi = value as *const u32;
        let ms = *ffi.add(2);
        let major = ms >> 16;
        let minor = ms & 0xFFFF;
        Some(format!("7-Zip {major}.{minor:02} (x64)"))
    }
}
