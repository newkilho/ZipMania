//! Feasibility spike: drive 7z.dll's COM API from Rust to implement the READ path
//! (open archive, list entries, extract one entry into memory, incl. encrypted).
//!
//! Fully isolated from the ZipMania app. Loads the bundled 7z.dll by absolute path via
//! LoadLibraryExW + GetProcAddress("CreateObject") -- no import lib / no registration.
//! COM callback objects (input stream, output stream, extract/open callbacks,
//! password provider) are implemented with windows-core `#[interface]` + `#[implement]`.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use windows_core::{
    implement, interface, Interface, BSTR, GUID, HRESULT, IUnknown, IUnknown_Vtbl,
};

// ---------------------------------------------------------------------------
// Win32 FFI (avoid pulling the full `windows` crate)
// ---------------------------------------------------------------------------
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}
const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x00000008;

type CreateObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

// ---------------------------------------------------------------------------
// GUIDs. 7-Zip interface IIDs follow {23170F69-40C1-278A-0000-0000<i>00<j>0000}
// (SDK macro: Data4 = {0,0,0,i,0,j,0,0}). Format CLSIDs: {..-1000-000110<id>0000}.
// ---------------------------------------------------------------------------
fn clsid_format(id: u8) -> GUID {
    GUID::from_u128(0x23170F69_40C1_278A_1000_000110000000 | ((id as u128) << 16))
}
const IID_IInArchive: GUID = GUID::from_u128(0x23170F69_40C1_278A_0000_000600600000);

// ---------------------------------------------------------------------------
// PROPVARIANT (hand-rolled; only the fields we read)
// ---------------------------------------------------------------------------
const VT_EMPTY: u16 = 0;
const VT_BSTR: u16 = 8;
const VT_BOOL: u16 = 11;
const VT_UI4: u16 = 19;
const VT_UI8: u16 = 21;

#[repr(C)]
struct PropVariant {
    vt: u16,
    _r1: u16,
    _r2: u16,
    _r3: u16,
    val: [u8; 8],
}
impl PropVariant {
    fn empty() -> Self {
        PropVariant { vt: VT_EMPTY, _r1: 0, _r2: 0, _r3: 0, val: [0; 8] }
    }
    fn as_u64(&self) -> Option<u64> {
        match self.vt {
            VT_UI8 => Some(u64::from_ne_bytes(self.val)),
            VT_UI4 => Some(u32::from_ne_bytes([self.val[0], self.val[1], self.val[2], self.val[3]]) as u64),
            _ => None,
        }
    }
    fn as_bool(&self) -> Option<bool> {
        if self.vt == VT_BOOL {
            Some(i16::from_ne_bytes([self.val[0], self.val[1]]) != 0)
        } else {
            None
        }
    }
    fn as_string(&self) -> Option<String> {
        if self.vt != VT_BSTR {
            return None;
        }
        let ptr = u64::from_ne_bytes(self.val) as *const u16;
        if ptr.is_null() {
            return Some(String::new());
        }
        unsafe {
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            Some(String::from_utf16_lossy(slice))
        }
    }
}

// Property IDs
const KPID_PATH: u32 = 3;
const KPID_IS_DIR: u32 = 6;
const KPID_SIZE: u32 = 7;

// ---------------------------------------------------------------------------
// Interface we CALL: IInArchive (only the methods the read path needs)
// ---------------------------------------------------------------------------
#[interface("23170F69-40C1-278A-0000-000600600000")]
unsafe trait IInArchive: IUnknown {
    unsafe fn Open(&self, stream: *mut c_void, maxCheck: *const u64, cb: *mut c_void) -> HRESULT;
    unsafe fn Close(&self) -> HRESULT;
    unsafe fn GetNumberOfItems(&self, num: *mut u32) -> HRESULT;
    unsafe fn GetProperty(&self, index: u32, propID: u32, value: *mut PropVariant) -> HRESULT;
    unsafe fn Extract(&self, indices: *const u32, num: u32, testMode: i32, cb: *mut c_void) -> HRESULT;
}

// ---------------------------------------------------------------------------
// Interfaces we IMPLEMENT
// ---------------------------------------------------------------------------
// IInStream (flattened: Read from ISequentialInStream, then Seek).
#[interface("23170F69-40C1-278A-0000-000300030000")]
unsafe trait IInStream: IUnknown {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT;
    unsafe fn Seek(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300020000")]
unsafe trait ISequentialOutStream: IUnknown {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600100000")]
unsafe trait IArchiveOpenCallback: IUnknown {
    unsafe fn SetTotal(&self, files: *const u64, bytes: *const u64) -> HRESULT;
    unsafe fn SetCompleted(&self, files: *const u64, bytes: *const u64) -> HRESULT;
}

// IArchiveExtractCallback flattened with its IProgress base (SetTotal/SetCompleted).
#[interface("23170F69-40C1-278A-0000-000600200000")]
unsafe trait IArchiveExtractCallback: IUnknown {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT;
    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT;
    unsafe fn GetStream(&self, index: u32, outStream: *mut *mut c_void, askMode: i32) -> HRESULT;
    unsafe fn PrepareOperation(&self, askMode: i32) -> HRESULT;
    unsafe fn SetOperationResult(&self, opRes: i32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000500100000")]
unsafe trait ICryptoGetTextPassword: IUnknown {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *const u16) -> HRESULT;
}

const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);

// ------- Memory input stream backed by a Vec<u8> + cursor -------
#[implement(IInStream)]
struct MemInStream {
    data: Vec<u8>,
    pos: Mutex<u64>,
}
impl IInStream_Impl for MemInStream_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        let mut pos = self.pos.lock().unwrap();
        let cur = (*pos).min(self.data.len() as u64) as usize;
        let n = ((self.data.len() - cur) as u64).min(size as u64) as usize;
        if n > 0 {
            std::ptr::copy_nonoverlapping(self.data.as_ptr().add(cur), data as *mut u8, n);
        }
        *pos = cur as u64 + n as u64;
        if !processed.is_null() {
            *processed = n as u32;
        }
        S_OK
    }
    unsafe fn Seek(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT {
        let mut pos = self.pos.lock().unwrap();
        let base = match origin {
            0 => 0i64,                    // STREAM_SEEK_SET
            1 => *pos as i64,             // STREAM_SEEK_CUR
            2 => self.data.len() as i64,  // STREAM_SEEK_END
            _ => return HRESULT(0x80070057u32 as i32), // E_INVALIDARG
        };
        let np = (base + offset).max(0) as u64;
        *pos = np;
        if !newPos.is_null() {
            *newPos = np;
        }
        S_OK
    }
}

// ------- Memory output stream that accumulates into a shared buffer -------
#[implement(ISequentialOutStream)]
struct MemOutStream {
    buf: Arc<Mutex<Vec<u8>>>,
}
impl ISequentialOutStream_Impl for MemOutStream_Impl {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        if size > 0 && !data.is_null() {
            let slice = std::slice::from_raw_parts(data, size as usize);
            self.buf.lock().unwrap().extend_from_slice(slice);
        }
        if !processed.is_null() {
            *processed = size;
        }
        S_OK
    }
}

// ------- Open callback that also supplies a password (for -mhe archives) -------
#[implement(IArchiveOpenCallback, ICryptoGetTextPassword)]
struct OpenCb {
    password: Option<Vec<u16>>,
}
impl IArchiveOpenCallback_Impl for OpenCb_Impl {
    unsafe fn SetTotal(&self, _f: *const u64, _b: *const u64) -> HRESULT { S_OK }
    unsafe fn SetCompleted(&self, _f: *const u64, _b: *const u64) -> HRESULT { S_OK }
}
impl ICryptoGetTextPassword_Impl for OpenCb_Impl {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *const u16) -> HRESULT {
        write_password(&self.password, password)
    }
}

// ------- Extract callback: routes one index into a memory buffer, supplies pw -------
#[implement(IArchiveExtractCallback, ICryptoGetTextPassword)]
struct ExtractCb {
    target: u32,
    buf: Arc<Mutex<Vec<u8>>>,
    password: Option<Vec<u16>>,
    op_result: Arc<Mutex<i32>>,
}
impl IArchiveExtractCallback_Impl for ExtractCb_Impl {
    unsafe fn SetTotal(&self, _total: u64) -> HRESULT { S_OK }
    unsafe fn SetCompleted(&self, _complete: *const u64) -> HRESULT { S_OK }
    unsafe fn GetStream(&self, index: u32, outStream: *mut *mut c_void, askMode: i32) -> HRESULT {
        if !outStream.is_null() {
            *outStream = std::ptr::null_mut();
        }
        // askMode 0 = extract. Only hand back a sink for our target index.
        if askMode == 0 && index == self.target {
            let out: ISequentialOutStream =
                MemOutStream { buf: self.buf.clone() }.into();
            if !outStream.is_null() {
                *outStream = out.into_raw();
            }
        }
        S_OK
    }
    unsafe fn PrepareOperation(&self, _askMode: i32) -> HRESULT { S_OK }
    unsafe fn SetOperationResult(&self, opRes: i32) -> HRESULT {
        *self.op_result.lock().unwrap() = opRes;
        S_OK
    }
}
impl ICryptoGetTextPassword_Impl for ExtractCb_Impl {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *const u16) -> HRESULT {
        write_password(&self.password, password)
    }
}

unsafe fn write_password(pw: &Option<Vec<u16>>, out: *mut *const u16) -> HRESULT {
    match pw {
        Some(wide) => {
            // BSTR-allocated copy; 7z takes ownership and frees via SysFreeString.
            let s = String::from_utf16_lossy(&wide[..wide.len().saturating_sub(1)]);
            let b = BSTR::from(s);
            if !out.is_null() {
                *out = b.into_raw();
            }
            S_OK
        }
        None => {
            if !out.is_null() {
                *out = std::ptr::null();
            }
            // No password available.
            S_FALSE
        }
    }
}

fn to_wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ===========================================================================
// WRITE (creation) path -- IOutArchive + IArchiveUpdateCallback + ISetProperties
// ===========================================================================
const IID_IOutArchive: GUID = GUID::from_u128(0x23170F69_40C1_278A_0000_000600A00000);

const KPID_ATTRIB: u32 = 9;
const KPID_MTIME: u32 = 12;
const VT_FILETIME: u16 = 64;

// --- PROPVARIANT writers (the read path only read them; creation must write) ---
unsafe fn pv_set_empty(p: *mut PropVariant) {
    (*p).vt = VT_EMPTY;
    (*p).val = [0; 8];
}
unsafe fn pv_set_bool(p: *mut PropVariant, v: bool) {
    (*p).vt = VT_BOOL;
    let x: i16 = if v { -1 } else { 0 };
    let b = x.to_ne_bytes();
    (*p).val = [b[0], b[1], 0, 0, 0, 0, 0, 0];
}
unsafe fn pv_set_u32(p: *mut PropVariant, v: u32) {
    (*p).vt = VT_UI4;
    let b = v.to_ne_bytes();
    (*p).val = [b[0], b[1], b[2], b[3], 0, 0, 0, 0];
}
unsafe fn pv_set_u64(p: *mut PropVariant, v: u64) {
    (*p).vt = VT_UI8;
    (*p).val = v.to_ne_bytes();
}
unsafe fn pv_set_filetime(p: *mut PropVariant, v: u64) {
    (*p).vt = VT_FILETIME;
    (*p).val = v.to_ne_bytes();
}
unsafe fn pv_set_bstr(p: *mut PropVariant, s: &str) {
    (*p).vt = VT_BSTR;
    let b = BSTR::from(s);
    let ptr = b.into_raw(); // ownership transferred to 7z (freed via PropVariantClear)
    (*p).val = (ptr as usize as u64).to_ne_bytes();
}

fn now_filetime() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (secs + 11_644_473_600) * 10_000_000
}

// --- Interfaces we CALL for creation ---
#[interface("23170F69-40C1-278A-0000-000600A00000")]
unsafe trait IOutArchive: IUnknown {
    unsafe fn UpdateItems(&self, outStream: *mut c_void, numItems: u32, cb: *mut c_void) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600030000")]
unsafe trait ISetProperties: IUnknown {
    unsafe fn SetProperties(&self, names: *const *const u16, values: *const PropVariant, num: u32) -> HRESULT;
}

// --- Interfaces we IMPLEMENT for creation ---
#[interface("23170F69-40C1-278A-0000-000300010000")]
unsafe trait ISequentialInStream: IUnknown {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT;
}
#[interface("23170F69-40C1-278A-0000-000300060000")]
unsafe trait IStreamGetSize: IUnknown {
    unsafe fn GetSize(&self, size: *mut u64) -> HRESULT;
}

// IArchiveUpdateCallback flattened with its IProgress base.
#[interface("23170F69-40C1-278A-0000-000600800000")]
unsafe trait IArchiveUpdateCallback: IUnknown {
    unsafe fn SetTotal(&self, total: u64) -> HRESULT;
    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT;
    unsafe fn GetUpdateItemInfo(&self, index: u32, newData: *mut i32, newProps: *mut i32, indexInArchive: *mut u32) -> HRESULT;
    unsafe fn GetProperty(&self, index: u32, propID: u32, value: *mut PropVariant) -> HRESULT;
    unsafe fn GetStream(&self, index: u32, inStream: *mut *mut c_void) -> HRESULT;
    unsafe fn SetOperationResult(&self, opRes: i32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000500110000")]
unsafe trait ICryptoGetTextPassword2: IUnknown {
    unsafe fn CryptoGetTextPassword2(&self, passwordIsDefined: *mut i32, password: *mut *const u16) -> HRESULT;
}

// Source stream feeding one item's bytes to 7z (exposes all 3 IIDs 7z may QI).
#[implement(ISequentialInStream, IInStream, IStreamGetSize)]
struct SourceStream {
    data: Vec<u8>,
    pos: Mutex<u64>,
}
impl SourceStream {
    unsafe fn read_into(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        let mut pos = self.pos.lock().unwrap();
        let cur = (*pos).min(self.data.len() as u64) as usize;
        let n = ((self.data.len() - cur) as u64).min(size as u64) as usize;
        if n > 0 {
            std::ptr::copy_nonoverlapping(self.data.as_ptr().add(cur), data as *mut u8, n);
        }
        *pos = cur as u64 + n as u64;
        if !processed.is_null() {
            *processed = n as u32;
        }
        S_OK
    }
    unsafe fn seek_to(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT {
        let mut pos = self.pos.lock().unwrap();
        let base = match origin {
            0 => 0i64,
            1 => *pos as i64,
            2 => self.data.len() as i64,
            _ => return HRESULT(0x80070057u32 as i32),
        };
        let np = (base + offset).max(0) as u64;
        *pos = np;
        if !newPos.is_null() {
            *newPos = np;
        }
        S_OK
    }
}
impl ISequentialInStream_Impl for SourceStream_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        self.read_into(data, size, processed)
    }
}
impl IInStream_Impl for SourceStream_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        self.read_into(data, size, processed)
    }
    unsafe fn Seek(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT {
        self.seek_to(offset, origin, newPos)
    }
}
impl IStreamGetSize_Impl for SourceStream_Impl {
    unsafe fn GetSize(&self, size: *mut u64) -> HRESULT {
        if !size.is_null() {
            *size = self.data.len() as u64;
        }
        S_OK
    }
}

#[derive(Clone)]
struct UpdateItem {
    name: String,
    data: Vec<u8>,
    is_dir: bool,
}

#[implement(IArchiveUpdateCallback, ICryptoGetTextPassword2)]
struct UpdateCb {
    items: Vec<UpdateItem>,
    password: Option<Vec<u16>>,
    mtime: u64,
    op_result: Arc<Mutex<i32>>,
}
impl IArchiveUpdateCallback_Impl for UpdateCb_Impl {
    unsafe fn SetTotal(&self, _total: u64) -> HRESULT { S_OK }
    unsafe fn SetCompleted(&self, _complete: *const u64) -> HRESULT { S_OK }
    unsafe fn GetUpdateItemInfo(&self, _index: u32, newData: *mut i32, newProps: *mut i32, indexInArchive: *mut u32) -> HRESULT {
        // Everything is a brand new item (no base archive).
        if !newData.is_null() { *newData = 1; }
        if !newProps.is_null() { *newProps = 1; }
        if !indexInArchive.is_null() { *indexInArchive = u32::MAX; } // -1
        S_OK
    }
    unsafe fn GetProperty(&self, index: u32, propID: u32, value: *mut PropVariant) -> HRESULT {
        let it = match self.items.get(index as usize) {
            Some(i) => i,
            None => { pv_set_empty(value); return S_OK; }
        };
        match propID {
            KPID_PATH => pv_set_bstr(value, &it.name),
            KPID_IS_DIR => pv_set_bool(value, it.is_dir),
            KPID_SIZE => pv_set_u64(value, it.data.len() as u64),
            KPID_ATTRIB => pv_set_u32(value, if it.is_dir { 0x10 } else { 0x20 }),
            KPID_MTIME => pv_set_filetime(value, self.mtime),
            _ => pv_set_empty(value),
        }
        S_OK
    }
    unsafe fn GetStream(&self, index: u32, inStream: *mut *mut c_void) -> HRESULT {
        if !inStream.is_null() { *inStream = std::ptr::null_mut(); }
        let it = match self.items.get(index as usize) {
            Some(i) => i,
            None => return S_OK,
        };
        if it.is_dir {
            return S_OK; // no stream for directories
        }
        let src: ISequentialInStream = SourceStream {
            data: it.data.clone(),
            pos: Mutex::new(0),
        }
        .into();
        if !inStream.is_null() {
            *inStream = src.into_raw();
        }
        S_OK
    }
    unsafe fn SetOperationResult(&self, opRes: i32) -> HRESULT {
        *self.op_result.lock().unwrap() = opRes;
        S_OK
    }
}
impl ICryptoGetTextPassword2_Impl for UpdateCb_Impl {
    unsafe fn CryptoGetTextPassword2(&self, passwordIsDefined: *mut i32, password: *mut *const u16) -> HRESULT {
        match &self.password {
            Some(wide) => {
                if !passwordIsDefined.is_null() { *passwordIsDefined = 1; }
                let s = String::from_utf16_lossy(&wide[..wide.len().saturating_sub(1)]);
                let b = BSTR::from(s);
                if !password.is_null() { *password = b.into_raw(); }
            }
            None => {
                if !passwordIsDefined.is_null() { *passwordIsDefined = 0; }
                if !password.is_null() { *password = std::ptr::null(); }
            }
        }
        S_OK
    }
}

// Seekable in-memory output stream. The 7z encoder QIs the output for IOutStream
// (it seeks back to patch header offsets); a sequential-only stream yields E_NOTIMPL.
#[interface("23170F69-40C1-278A-0000-000300040000")]
unsafe trait IOutStream: IUnknown {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT;
    unsafe fn Seek(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT;
    unsafe fn SetSize(&self, newSize: u64) -> HRESULT;
}

#[implement(IOutStream, ISequentialOutStream)]
struct SeekableOut {
    data: Arc<Mutex<Vec<u8>>>,
    pos: Mutex<u64>,
}
impl SeekableOut {
    unsafe fn write_at(&self, src: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        let n = size as usize;
        if n > 0 && !src.is_null() {
            let mut data = self.data.lock().unwrap();
            let mut pos = self.pos.lock().unwrap();
            let p = *pos as usize;
            if p + n > data.len() {
                data.resize(p + n, 0);
            }
            std::ptr::copy_nonoverlapping(src, data.as_mut_ptr().add(p), n);
            *pos = (p + n) as u64;
        }
        if !processed.is_null() {
            *processed = size;
        }
        S_OK
    }
    unsafe fn seek_to(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT {
        let len = self.data.lock().unwrap().len() as i64;
        let mut pos = self.pos.lock().unwrap();
        let base = match origin {
            0 => 0i64,
            1 => *pos as i64,
            2 => len,
            _ => return HRESULT(0x80070057u32 as i32),
        };
        let np = (base + offset).max(0) as u64;
        *pos = np;
        if !newPos.is_null() {
            *newPos = np;
        }
        S_OK
    }
}
impl IOutStream_Impl for SeekableOut_Impl {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        self.write_at(data, size, processed)
    }
    unsafe fn Seek(&self, offset: i64, origin: u32, newPos: *mut u64) -> HRESULT {
        self.seek_to(offset, origin, newPos)
    }
    unsafe fn SetSize(&self, newSize: u64) -> HRESULT {
        self.data.lock().unwrap().resize(newSize as usize, 0);
        S_OK
    }
}
impl ISequentialOutStream_Impl for SeekableOut_Impl {
    unsafe fn Write(&self, data: *const u8, size: u32, processed: *mut u32) -> HRESULT {
        self.write_at(data, size, processed)
    }
}

/// Create an archive fully in memory. Returns (archive_bytes, op_result).
fn create_archive(
    dll: &Dll,
    format_id: u8,
    items: Vec<UpdateItem>,
    level: Option<u32>,
    password: Option<&str>,
    header_encrypt: bool,
) -> Result<(Vec<u8>, i32), String> {
    let clsid = clsid_format(format_id);
    let mut raw: *mut c_void = std::ptr::null_mut();
    let out = unsafe {
        let hr = (dll.create_object)(&clsid, &IID_IOutArchive, &mut raw);
        if hr.0 != 0 || raw.is_null() {
            return Err(format!("CreateObject(IOutArchive) hr=0x{:08X}", hr.0));
        }
        IOutArchive::from_raw(raw)
    };

    if level.is_some() || header_encrypt {
        let setp: ISetProperties = out
            .cast()
            .map_err(|e| format!("QueryInterface(ISetProperties) failed: {e:?}"))?;
        let mut names: Vec<Vec<u16>> = Vec::new();
        let mut values: Vec<PropVariant> = Vec::new();
        if let Some(l) = level {
            names.push(to_wide_nul("x"));
            let mut pv = PropVariant::empty();
            unsafe { pv_set_u32(&mut pv, l); }
            values.push(pv);
        }
        if header_encrypt {
            names.push(to_wide_nul("he"));
            let mut pv = PropVariant::empty();
            unsafe { pv_set_bool(&mut pv, true); }
            values.push(pv);
        }
        let name_ptrs: Vec<*const u16> = names.iter().map(|n| n.as_ptr()).collect();
        let hr = unsafe { setp.SetProperties(name_ptrs.as_ptr(), values.as_ptr(), values.len() as u32) };
        if hr.0 != 0 {
            return Err(format!("SetProperties hr=0x{:08X}", hr.0));
        }
    }

    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let outstream: IOutStream = SeekableOut { data: buf.clone(), pos: Mutex::new(0) }.into();
    let op = Arc::new(Mutex::new(-1i32));
    let num = items.len() as u32;
    let cb: IArchiveUpdateCallback = UpdateCb {
        items,
        password: password.map(to_wide_nul),
        mtime: now_filetime(),
        op_result: op.clone(),
    }
    .into();
    let hr = unsafe { out.UpdateItems(outstream.as_raw(), num, cb.as_raw()) };
    if hr.0 != 0 {
        return Err(format!("UpdateItems hr=0x{:08X}", hr.0));
    }
    let bytes = buf.lock().unwrap().clone();
    let op_val = *op.lock().unwrap();
    Ok((bytes, op_val))
}

/// Run bundled 7z.exe `t` (integrity test) on a produced archive.
fn sevenz_test(work: &PathBuf, path: &PathBuf, password: Option<&str>) -> Result<(), String> {
    let ps = path.to_string_lossy().to_string();
    let pw;
    let mut args: Vec<&str> = vec!["t", &ps];
    if let Some(p) = password {
        pw = format!("-p{p}");
        args.push(&pw);
    }
    run_7z(&args, work)
}

// ---------------------------------------------------------------------------
// High-level read-path helpers
// ---------------------------------------------------------------------------
struct Dll {
    create_object: CreateObjectFn,
    _module: *mut c_void,
}
impl Dll {
    fn load(path: &PathBuf) -> Result<Dll, String> {
        let wide = to_wide_nul(&path.to_string_lossy());
        unsafe {
            let module = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
            if module.is_null() {
                return Err(format!("LoadLibraryExW failed for {}", path.display()));
            }
            let proc = GetProcAddress(module, b"CreateObject\0".as_ptr());
            if proc.is_null() {
                return Err("GetProcAddress(CreateObject) failed".into());
            }
            let create_object: CreateObjectFn = std::mem::transmute(proc);
            Ok(Dll { create_object, _module: module })
        }
    }
    fn create_in_archive(&self, format_id: u8) -> Result<IInArchive, String> {
        let clsid = clsid_format(format_id);
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe {
            let hr = (self.create_object)(&clsid, &IID_IInArchive, &mut raw);
            if hr.0 != 0 || raw.is_null() {
                return Err(format!("CreateObject failed hr=0x{:08X}", hr.0));
            }
            Ok(IInArchive::from_raw(raw))
        }
    }
}

struct Entry {
    index: u32,
    path: String,
    size: u64,
    is_dir: bool,
}

fn open_archive(archive: &IInArchive, bytes: Vec<u8>, password: Option<&str>) -> HRESULT {
    let stream: IInStream = MemInStream { data: bytes, pos: Mutex::new(0) }.into();
    let open_cb: IArchiveOpenCallback =
        OpenCb { password: password.map(to_wide_nul) }.into();
    let max_check: u64 = 1 << 23;
    unsafe { archive.Open(stream.as_raw(), &max_check, open_cb.as_raw()) }
}

fn list_entries(archive: &IInArchive) -> Vec<Entry> {
    let mut out = Vec::new();
    unsafe {
        let mut n: u32 = 0;
        if archive.GetNumberOfItems(&mut n).0 != 0 {
            return out;
        }
        for i in 0..n {
            let mut p = PropVariant::empty();
            let _ = archive.GetProperty(i, KPID_PATH, &mut p);
            let path = p.as_string().unwrap_or_default();
            let mut s = PropVariant::empty();
            let _ = archive.GetProperty(i, KPID_SIZE, &mut s);
            let size = s.as_u64().unwrap_or(0);
            let mut d = PropVariant::empty();
            let _ = archive.GetProperty(i, KPID_IS_DIR, &mut d);
            let is_dir = d.as_bool().unwrap_or(false);
            out.push(Entry { index: i, path, size, is_dir });
        }
    }
    out
}

/// Returns (extract_hr, op_result, bytes). op_result 0 == OK.
fn extract_to_memory(archive: &IInArchive, index: u32, password: Option<&str>) -> (HRESULT, i32, Vec<u8>) {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let op_result = Arc::new(Mutex::new(-1i32));
    let cb: IArchiveExtractCallback = ExtractCb {
        target: index,
        buf: buf.clone(),
        password: password.map(to_wide_nul),
        op_result: op_result.clone(),
    }
    .into();
    let indices = [index];
    let hr = unsafe { archive.Extract(indices.as_ptr(), 1, 0, cb.as_raw()) };
    let op = *op_result.lock().unwrap();
    let bytes = buf.lock().unwrap().clone();
    (hr, op, bytes)
}

// ---------------------------------------------------------------------------
// Fixture creation using the bundled 7z.exe
// ---------------------------------------------------------------------------
fn sevenzip_exe() -> PathBuf {
    binaries_dir().join("7z.exe")
}
fn binaries_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("src-tauri")
        .join("binaries")
}

fn run_7z(args: &[&str], cwd: &PathBuf) -> Result<(), String> {
    let out = Command::new(sevenzip_exe())
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("spawn 7z.exe: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "7z {:?} failed: {}\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn main() {
    println!("=== 7z.dll COM read-path feasibility spike ===\n");

    let dll_path = binaries_dir().join("7z.dll");
    println!("[*] bundled dll : {}", dll_path.display());
    println!("[*] bundled exe : {}", sevenzip_exe().display());

    let dll = match Dll::load(&dll_path) {
        Ok(d) => {
            println!("[OK] LoadLibraryExW + GetProcAddress(\"CreateObject\") succeeded\n");
            d
        }
        Err(e) => {
            eprintln!("[FATAL] {e}");
            std::process::exit(1);
        }
    };

    // --- Build fixtures in a temp dir with 7z.exe ---
    let work = std::env::temp_dir().join(format!("spike7z_{}", std::process::id()));
    let src = work.join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Deterministic-ish content, incl. Korean name, empty file, a binary-ish blob.
    let f_hello = "hello.txt";
    let c_hello = b"Hello, 7-Zip via COM from Rust!\n".to_vec();
    let f_kor = "\u{d55c}\u{ae00}\u{d30c}\u{c77c}.txt"; // 한글파일.txt
    let c_kor = "한국어 내용 with unicode \u{1F980}\n".as_bytes().to_vec();
    let f_empty = "empty.dat";
    let c_empty: Vec<u8> = Vec::new();
    let f_big = "blob.bin";
    let c_big: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2654435761u32) >> 13) as u8).collect();

    for (name, content) in [
        (f_hello, &c_hello),
        (f_kor, &c_kor),
        (f_empty, &c_empty),
        (f_big, &c_big),
    ] {
        std::fs::write(src.join(name), content).unwrap();
    }

    let arc_7z = work.join("archive.7z");
    let arc_zip = work.join("archive.zip");
    let arc_enc = work.join("secret.7z");
    let arc_enc_hdr = work.join("secret_hdr.7z");
    let password = "P@ssw0rd_한글";

    // Solid .7z (7z is solid by default), .zip, encrypted .7z (data only),
    // and encrypted-with-header .7z (-mhe=on) to exercise open-time password.
    let mk = |args: Vec<&str>| run_7z(&args, &src);
    let z7 = arc_7z.to_string_lossy().to_string();
    let zzip = arc_zip.to_string_lossy().to_string();
    let zenc = arc_enc.to_string_lossy().to_string();
    let zench = arc_enc_hdr.to_string_lossy().to_string();
    let pw_arg = format!("-p{password}");

    let fixtures: Vec<(&str, Vec<&str>)> = vec![
        ("7z (solid)", vec!["a", "-t7z", &z7, "hello.txt", f_kor, "empty.dat", "blob.bin"]),
        ("zip", vec!["a", "-tzip", &zzip, "hello.txt", f_kor, "empty.dat", "blob.bin"]),
        ("7z encrypted (data)", vec!["a", "-t7z", &pw_arg, &zenc, "hello.txt", f_kor]),
        ("7z encrypted (+header -mhe=on)", vec!["a", "-t7z", &pw_arg, "-mhe=on", &zench, "hello.txt", f_kor]),
    ];
    for (label, args) in &fixtures {
        match mk(args.clone()) {
            Ok(_) => println!("[fixture] built {label}"),
            Err(e) => {
                eprintln!("[FATAL] fixture {label}: {e}");
                std::process::exit(1);
            }
        }
    }
    println!();

    let originals: std::collections::HashMap<&str, &Vec<u8>> = [
        (f_hello, &c_hello),
        (f_kor, &c_kor),
        (f_empty, &c_empty),
        (f_big, &c_big),
    ]
    .into_iter()
    .collect();

    // Format ids
    const FMT_7Z: u8 = 0x07;
    const FMT_ZIP: u8 = 0x01;

    let mut pass = 0u32;
    let mut fail = 0u32;
    let mut check = |cond: bool, msg: String| {
        if cond {
            pass += 1;
            println!("    [PASS] {msg}");
        } else {
            fail += 1;
            println!("    [FAIL] {msg}");
        }
    };

    // ---- Scenario helper ----
    let run_scenario = |title: &str,
                            path: &PathBuf,
                            fmt: u8,
                            password: Option<&str>,
                            expect_extract_ok: bool,
                            originals: &std::collections::HashMap<&str, &Vec<u8>>,
                            check: &mut dyn FnMut(bool, String)| {
        println!("--- {title} ---");
        let bytes = std::fs::read(path).unwrap();
        let archive = match dll.create_in_archive(fmt) {
            Ok(a) => a,
            Err(e) => {
                check(false, format!("CreateObject: {e}"));
                return;
            }
        };
        let hr = open_archive(&archive, bytes, password);
        check(hr.0 == 0, format!("IInArchive::Open hr=0x{:08X}", hr.0));
        if hr.0 != 0 {
            return;
        }
        let entries = list_entries(&archive);
        println!("    entries: {}", entries.len());
        for e in &entries {
            println!(
                "      [{}] {:<24} size={:<6} dir={}",
                e.index, e.path, e.size, e.is_dir
            );
        }
        // Extract & verify every non-dir entry we know the original of.
        for e in &entries {
            if e.is_dir {
                continue;
            }
            let base = e.path.rsplit(['/', '\\']).next().unwrap_or(&e.path);
            let orig = originals.get(base);
            let (ehr, op, data) = extract_to_memory(&archive, e.index, password);
            if expect_extract_ok {
                let ok_len = data.len() as u64 == e.size;
                let ok_content = orig.map(|o| o.as_slice() == data.as_slice()).unwrap_or(ok_len);
                check(
                    ehr.0 == 0 && op == 0 && ok_content,
                    format!(
                        "extract '{}' -> {} bytes (opResult={}), matches original = {}",
                        base,
                        data.len(),
                        op,
                        ok_content
                    ),
                );
            } else {
                // Expect failure (wrong/no password): extract must NOT reproduce content.
                let reproduced = orig.map(|o| o.as_slice() == data.as_slice() && !o.is_empty()).unwrap_or(false);
                check(
                    !reproduced,
                    format!(
                        "extract '{}' with bad/no password correctly did NOT yield content (hr=0x{:08X}, opResult={}, {} bytes)",
                        base, ehr.0, op, data.len()
                    ),
                );
            }
        }
        unsafe {
            let _ = archive.Close();
        }
    };

    // 1) plain .7z (solid, korean, empty)
    run_scenario("Scenario 1: .7z solid (korean + empty + blob)", &arc_7z, FMT_7Z, None, true, &originals, &mut check);
    println!();
    // 2) plain .zip
    run_scenario("Scenario 2: .zip", &arc_zip, FMT_ZIP, None, true, &originals, &mut check);
    println!();
    // 3) encrypted .7z (data), correct password
    run_scenario("Scenario 3: encrypted .7z, CORRECT password", &arc_enc, FMT_7Z, Some(password), true, &originals, &mut check);
    println!();
    // 4) encrypted .7z (data), wrong password
    run_scenario("Scenario 4: encrypted .7z, WRONG password", &arc_enc, FMT_7Z, Some("totally-wrong"), false, &originals, &mut check);
    println!();
    // 5) encrypted .7z (data), no password
    run_scenario("Scenario 5: encrypted .7z, NO password", &arc_enc, FMT_7Z, None, false, &originals, &mut check);
    println!();
    // 6) header-encrypted .7z, correct password (open-time password via open callback)
    run_scenario("Scenario 6: header-encrypted .7z (-mhe), CORRECT password", &arc_enc_hdr, FMT_7Z, Some(password), true, &originals, &mut check);
    println!();

    // =====================================================================
    // WRITE (creation) path scenarios
    // =====================================================================
    println!("################  WRITE / CREATION PATH  ################\n");

    let mk_items = || {
        vec![
            UpdateItem { name: "hello.txt".into(), data: c_hello.clone(), is_dir: false },
            UpdateItem { name: f_kor.into(), data: c_kor.clone(), is_dir: false },
            UpdateItem { name: "empty.dat".into(), data: c_empty.clone(), is_dir: false },
            UpdateItem { name: "blob.bin".into(), data: c_big.clone(), is_dir: false },
        ]
    };

    // W1: create .7z, then read-spike round-trip + 7z.exe integrity test.
    println!("--- Write 1: create .7z (in-memory IOutArchive::UpdateItems) ---");
    match create_archive(&dll, FMT_7Z, mk_items(), None, None, false) {
        Ok((bytes, op)) => {
            println!("    produced {} bytes, updateOpResult={}", bytes.len(), op);
            let path = work.join("made.7z");
            std::fs::write(&path, &bytes).unwrap();
            check(op == 0, format!("UpdateItems opResult == 0 ({op})"));
            run_scenario("  round-trip read of created .7z", &path, FMT_7Z, None, true, &originals, &mut check);
            let t = sevenz_test(&work, &path, None);
            check(t.is_ok(), format!("bundled 7z.exe `t` accepts created .7z ({:?})", t.err()));
        }
        Err(e) => check(false, format!("create .7z: {e}")),
    }
    println!();

    // W2: create .zip, round-trip + 7z.exe test.
    println!("--- Write 2: create .zip ---");
    match create_archive(&dll, FMT_ZIP, mk_items(), None, None, false) {
        Ok((bytes, op)) => {
            println!("    produced {} bytes, updateOpResult={}", bytes.len(), op);
            let path = work.join("made.zip");
            std::fs::write(&path, &bytes).unwrap();
            check(op == 0, format!("UpdateItems opResult == 0 ({op})"));
            run_scenario("  round-trip read of created .zip", &path, FMT_ZIP, None, true, &originals, &mut check);
            let t = sevenz_test(&work, &path, None);
            check(t.is_ok(), format!("bundled 7z.exe `t` accepts created .zip ({:?})", t.err()));
        }
        Err(e) => check(false, format!("create .zip: {e}")),
    }
    println!();

    // W3: create encrypted .7z (data), verify correct/wrong password on read + 7z.exe test.
    println!("--- Write 3: create encrypted .7z (ICryptoGetTextPassword2) ---");
    match create_archive(&dll, FMT_7Z, mk_items(), None, Some(password), false) {
        Ok((bytes, op)) => {
            println!("    produced {} bytes, updateOpResult={}", bytes.len(), op);
            let path = work.join("made_enc.7z");
            std::fs::write(&path, &bytes).unwrap();
            check(op == 0, format!("UpdateItems opResult == 0 ({op})"));
            run_scenario("  created encrypted .7z, CORRECT password", &path, FMT_7Z, Some(password), true, &originals, &mut check);
            run_scenario("  created encrypted .7z, WRONG password", &path, FMT_7Z, Some("nope-wrong"), false, &originals, &mut check);
            let t = sevenz_test(&work, &path, Some(password));
            check(t.is_ok(), format!("bundled 7z.exe `t -p<pw>` accepts created encrypted .7z ({:?})", t.err()));
        }
        Err(e) => check(false, format!("create encrypted .7z: {e}")),
    }
    println!();

    // W4: create header-encrypted .7z (ISetProperties he=on). Without password the
    // read spike must fail to even open/list; with password it round-trips.
    println!("--- Write 4: create header-encrypted .7z (ISetProperties \"he\"=on) ---");
    match create_archive(&dll, FMT_7Z, mk_items(), None, Some(password), true) {
        Ok((bytes, op)) => {
            println!("    produced {} bytes, updateOpResult={}", bytes.len(), op);
            let path = work.join("made_hdr.7z");
            std::fs::write(&path, &bytes).unwrap();
            check(op == 0, format!("UpdateItems opResult == 0 ({op})"));
            // Open WITHOUT password -> must fail (header is encrypted).
            let arc = dll.create_in_archive(FMT_7Z).unwrap();
            let hr = open_archive(&arc, bytes.clone(), None);
            let entries_no_pw = if hr.0 == 0 { list_entries(&arc).len() } else { 0 };
            check(
                hr.0 != 0 || entries_no_pw == 0,
                format!("header-enc .7z NOT listable without password (openHr=0x{:08X}, entries={})", hr.0, entries_no_pw),
            );
            unsafe { let _ = arc.Close(); }
            // Open WITH password -> round-trip.
            run_scenario("  header-enc created .7z, CORRECT password", &path, FMT_7Z, Some(password), true, &originals, &mut check);
            let t = sevenz_test(&work, &path, Some(password));
            check(t.is_ok(), format!("bundled 7z.exe `t -p<pw>` accepts header-enc .7z ({:?})", t.err()));
        }
        Err(e) => check(false, format!("create header-encrypted .7z: {e}")),
    }
    println!();

    // W5: compression level reflected (x=0 store vs x=9 max) on compressible data.
    println!("--- Write 5: compression level via ISetProperties (\"x\"=0 vs 9) ---");
    let comp: Vec<u8> = (0..40_000u32).map(|i| (i % 61) as u8).collect();
    let one = |d: &Vec<u8>| vec![UpdateItem { name: "data.bin".into(), data: d.clone(), is_dir: false }];
    let sz0 = create_archive(&dll, FMT_7Z, one(&comp), Some(0), None, false).map(|(b, _)| b.len());
    let sz9 = create_archive(&dll, FMT_7Z, one(&comp), Some(9), None, false).map(|(b, _)| b.len());
    match (sz0, sz9) {
        (Ok(s0), Ok(s9)) => {
            println!("    level 0 (store) = {s0} bytes, level 9 (max) = {s9} bytes, raw = {}", comp.len());
            check(s9 < s0, format!("level 9 archive smaller than level 0 ({s9} < {s0})"));
        }
        (a, b) => check(false, format!("level test create failed: {a:?} / {b:?}")),
    }
    println!();

    println!("=== RESULT: {pass} checks passed, {fail} failed ===");

    // best-effort cleanup
    let _ = std::fs::remove_dir_all(&work);

    if fail > 0 {
        std::process::exit(2);
    }
}
