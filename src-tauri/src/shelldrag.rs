//! 파일 목록 Shell DnD, 커스텀 IDataObject → DoDragDrop, CF_HDROP 요청 시 임시 루트 하위로 추출
//! FILECONTENTS 아닌 CF_HDROP (D3.5)

/// 드래그로 내보낼 항목 하나(아카이브 내부 파일 → 임시 폴더에 만들 상대 경로)
#[derive(Debug, Clone)]
pub struct DragItem {
    pub rel_name: String,
    pub inner_path: String,
}

/// 선택 내부 경로 → 실제 파일 항목 목록, 파일 = 그 하나, 폴더 = 하위 전부(구조 유지), 중복 제거
pub fn resolve_items(
    entries: &[zipmania_archive::ArchiveEntry],
    selected: &[String],
) -> Vec<DragItem> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for sel in selected {
        let p = sel.replace('\\', "/");
        // 선택 경로의 부모까지 길이(그 아래를 상대 경로로 만들기 위함)
        let parent_len = p.rfind('/').map(|i| i + 1).unwrap_or(0);
        let dir_prefix = format!("{p}/");

        for e in entries {
            if e.is_dir {
                continue;
            }
            let ep = e.path.replace('\\', "/");
            let is_target = ep == p || ep.starts_with(&dir_prefix);
            if !is_target {
                continue;
            }
            if !seen.insert(ep.clone()) {
                continue;
            }
            let rel = &ep[parent_len.min(ep.len())..];
            out.push(DragItem {
                rel_name: rel.replace('/', "\\"),
                inner_path: ep.clone(),
            });
        }
    }
    out
}

#[cfg(windows)]
pub fn do_shell_drag(
    app: tauri::AppHandle,
    archive: String,
    dll: std::path::PathBuf,
    password: Option<String>,
    items: Vec<DragItem>,
) -> windows::core::HRESULT {
    if items.is_empty() {
        return windows::core::HRESULT(0);
    }
    imp::run(app, archive, dll, password, items)
}

/// 비-Windows 스텁(현재 begin_shell_drag 가 비-Windows 에선 호출하지 않는다)
#[cfg(not(windows))]
pub fn do_shell_drag(
    _app: tauri::AppHandle,
    _archive: String,
    _dll: std::path::PathBuf,
    _password: Option<String>,
    _items: Vec<DragItem>,
) -> i32 {
    -1
}

#[cfg(windows)]
mod imp {
    use std::cell::Cell;
    use std::mem::ManuallyDrop;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use windows::core::{implement, Error, Result, BOOL, HRESULT};
    use windows::Win32::Foundation::{HGLOBAL, POINT};
    use windows::Win32::System::Com::{
        IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumFORMATETC_Impl,
        IEnumSTATDATA, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0,
        TYMED_HGLOBAL,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
    use windows::Win32::System::Ole::{
        DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, DROPEFFECT, DROPEFFECT_COPY,
    };
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows::Win32::UI::Shell::DROPFILES;

    use zipmania_archive::Router;

    use super::DragItem;

    // 표준 HRESULT 상수(윈도우 헤더 값)
    const S_OK: HRESULT = HRESULT(0);
    const S_FALSE: HRESULT = HRESULT(1);
    const E_NOTIMPL: HRESULT = HRESULT(0x8000_4001u32 as i32);
    const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007_000Eu32 as i32);
    const E_FAIL: HRESULT = HRESULT(0x8000_4005u32 as i32);
    const DV_E_FORMATETC: HRESULT = HRESULT(0x8004_0064u32 as i32);
    const OLE_E_ADVISENOTSUPPORTED: HRESULT = HRESULT(0x8004_0003u32 as i32);
    const DRAGDROP_S_DROP: HRESULT = HRESULT(0x0004_0100u32 as i32);
    const DRAGDROP_S_CANCEL: HRESULT = HRESULT(0x0004_0101u32 as i32);
    const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = HRESULT(0x0004_0102u32 as i32);
    const MK_LBUTTON: u32 = 0x0001;

    /// CF_HDROP(파일 경로 목록), 등록 불필요
    const CF_HDROP: u16 = 15;

    /// 열거자용 포맷 사양(작고 Copy 가능)
    #[derive(Clone, Copy)]
    struct FmtSpec {
        cf: u16,
        tymed: u32,
    }

    // ── 커스텀 데이터 오브젝트(CF_HDROP, 드롭 시 임시 추출) ──
    #[implement(IDataObject)]
    struct HdropData {
        app: tauri::AppHandle,
        archive: String,
        dll: PathBuf,
        password: Option<String>,
        items: Vec<DragItem>,
        temp_dir: Mutex<Option<PathBuf>>,
    }

    impl HdropData {
        /// 선택 항목을 임시 루트 하위에 1회 추출 → 폴더 경로 반환
        fn ensure_extracted(&self) -> Result<PathBuf> {
            let mut guard = self.temp_dir.lock().unwrap();
            if guard.is_none() {
                let dir = extract_to_temp(
                    &self.app,
                    &self.archive,
                    &self.dll,
                    self.password.as_deref(),
                    &self.items,
                )
                .map_err(|_| Error::from(E_FAIL))?;
                *guard = Some(dir);
            }
            Ok(guard.clone().unwrap())
        }
    }

    impl IDataObject_Impl for HdropData_Impl {
        fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
            let fmt = unsafe { &*pformatetcin };

            // CF_HDROP 요청 → 임시 폴더 해제 후 실제 경로 목록(DROPFILES) 반환
            if fmt.cfFormat == CF_HDROP && (fmt.tymed & TYMED_HGLOBAL.0 as u32) != 0 {
                let dir = self.ensure_extracted()?;
                let paths = top_level_wide_paths(&dir, &self.items);
                if paths.is_empty() {
                    return Err(Error::from(DV_E_FORMATETC));
                }
                let hglobal = unsafe { build_hdrop(&paths) }?;
                return Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: hglobal },
                    pUnkForRelease: ManuallyDrop::new(None),
                });
            }

            Err(Error::from(DV_E_FORMATETC))
        }

        fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> Result<()> {
            Err(Error::from(E_NOTIMPL))
        }

        fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
            let fmt = unsafe { &*pformatetc };
            if fmt.cfFormat == CF_HDROP && (fmt.tymed & TYMED_HGLOBAL.0 as u32) != 0 {
                S_OK
            } else {
                S_FALSE
            }
        }

        fn GetCanonicalFormatEtc(
            &self,
            _pformatectin: *const FORMATETC,
            _pformatetcout: *mut FORMATETC,
        ) -> HRESULT {
            E_NOTIMPL
        }

        fn SetData(
            &self,
            _pformatetc: *const FORMATETC,
            _pmedium: *const STGMEDIUM,
            _frelease: BOOL,
        ) -> Result<()> {
            Err(Error::from(E_NOTIMPL))
        }

        fn EnumFormatEtc(&self, dwdirection: u32) -> Result<IEnumFORMATETC> {
            if dwdirection != DATADIR_GET.0 as u32 {
                return Err(Error::from(E_NOTIMPL));
            }
            let specs = vec![FmtSpec {
                cf: CF_HDROP,
                tymed: TYMED_HGLOBAL.0 as u32,
            }];
            let en: IEnumFORMATETC = FormatEnum {
                specs,
                index: Cell::new(0),
            }
            .into();
            Ok(en)
        }

        fn DAdvise(
            &self,
            _pformatetc: *const FORMATETC,
            _advf: u32,
            _padvsink: windows_core::Ref<'_, IAdviseSink>,
        ) -> Result<u32> {
            Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
        }

        fn DUnadvise(&self, _dwconnection: u32) -> Result<()> {
            Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
        }

        fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
            Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
        }
    }

    // ── 포맷 열거자 ──
    #[implement(IEnumFORMATETC)]
    struct FormatEnum {
        specs: Vec<FmtSpec>,
        index: Cell<usize>,
    }

    impl IEnumFORMATETC_Impl for FormatEnum_Impl {
        fn Next(&self, celt: u32, rgelt: *mut FORMATETC, pceltfetched: *mut u32) -> HRESULT {
            let mut fetched = 0u32;
            let mut idx = self.index.get();
            while fetched < celt && idx < self.specs.len() {
                let spec = self.specs[idx];
                let fmt = FORMATETC {
                    cfFormat: spec.cf,
                    ptd: std::ptr::null_mut(),
                    dwAspect: DVASPECT_CONTENT.0 as u32,
                    lindex: -1,
                    tymed: spec.tymed,
                };
                unsafe { std::ptr::write(rgelt.add(fetched as usize), fmt) };
                idx += 1;
                fetched += 1;
            }
            self.index.set(idx);
            if !pceltfetched.is_null() {
                unsafe { *pceltfetched = fetched };
            }
            if fetched == celt {
                S_OK
            } else {
                S_FALSE
            }
        }

        fn Skip(&self, celt: u32) -> Result<()> {
            let target = self.index.get() + celt as usize;
            let clamped = target.min(self.specs.len());
            self.index.set(clamped);
            if target <= self.specs.len() {
                Ok(())
            } else {
                Err(Error::from(S_FALSE))
            }
        }

        fn Reset(&self) -> Result<()> {
            self.index.set(0);
            Ok(())
        }

        fn Clone(&self) -> Result<IEnumFORMATETC> {
            let en: IEnumFORMATETC = FormatEnum {
                specs: self.specs.clone(),
                index: Cell::new(self.index.get()),
            }
            .into();
            Ok(en)
        }
    }

    // ── 드래그 소스 ──
    #[implement(IDropSource)]
    struct DropSource;

    impl IDropSource_Impl for DropSource_Impl {
        fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
            if fescapepressed.as_bool() {
                return DRAGDROP_S_CANCEL;
            }
            if (grfkeystate.0 & MK_LBUTTON) == 0 {
                return DRAGDROP_S_DROP;
            }
            S_OK
        }

        fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// 선택 항목을 아카이브 전용 폴더에 내부 경로 구조대로 추출 → 폴더 반환
    /// 경로가 결정적이라 이미 풀린 것은 재사용(더블클릭 실행과 공유)
    fn extract_to_temp(
        app: &tauri::AppHandle,
        archive: &str,
        dll: &PathBuf,
        password: Option<&str>,
        items: &[DragItem],
    ) -> std::result::Result<PathBuf, zipmania_archive::ZipManiaError> {
        let base = crate::commands::archive_temp_dir(app, archive);
        let router = Router::new(dll.clone());
        for item in items {
            // 안전한 경로로 만들 수 없는 항목은 건너뛴다, 임시 루트 하위인지까지 확인
            let Some(dest) = crate::commands::inner_dest_path(&base, &item.inner_path) else {
                continue;
            };
            if dest.is_file() {
                continue; // 이미 풀린 항목은 다시 풀지 않는다
            }
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = router.for_archive(archive).extract_entry_to_file(
                archive,
                &item.inner_path,
                &dest,
                password,
            ) {
                // 대상을 지우지 않는다 — 그 자리의 파일은 우리 것이 아니다(정상 캐시)
                return Err(e);
            }
        }
        Ok(base)
    }

    /// CF_HDROP 에 담을 최상위 경로 UTF-16. 항목별 선택 최상위 구해 중복 제거
    fn top_level_wide_paths(base: &Path, items: &[DragItem]) -> Vec<Vec<u16>> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for item in items {
            let inner = item.inner_path.as_str(); // "/" 정규화
            let rel = item.rel_name.replace('\\', "/");
            // 선택된 최상위 내부 경로 = inner 에서 rel 을 뗀 부모 + rel 의 첫 조각
            let parent = inner.strip_suffix(rel.as_str()).unwrap_or("");
            let first = rel.split('/').next().unwrap_or(rel.as_str());
            let top_inner = format!("{parent}{first}");
            if top_inner.is_empty() || !seen.insert(top_inner.clone()) {
                continue;
            }
            // 추출에서 건너뛴 항목은 CF_HDROP 에도 담지 않는다, 판정은 추출 쪽과 같은 함수로
            let Some(full) = crate::commands::inner_dest_path(&base, &top_inner) else {
                continue;
            };
            let wide: Vec<u16> = full.as_os_str().encode_wide().collect();
            out.push(wide);
        }
        out
    }

    /// DROPFILES + 이중 널 종료 경로 목록(HGLOBAL), paths = 널 종료 없는 UTF-16
    unsafe fn build_hdrop(paths: &[Vec<u16>]) -> Result<HGLOBAL> {
        let header = std::mem::size_of::<DROPFILES>();
        let mut wide_count = 0usize;
        for p in paths {
            wide_count += p.len() + 1; // 경로 + 널
        }
        wide_count += 1; // 이중 널 종료
        let total = header + wide_count * 2;

        let hglobal = GlobalAlloc(GHND, total)?;
        let base = GlobalLock(hglobal) as *mut u8;
        if base.is_null() {
            return Err(Error::from(E_OUTOFMEMORY));
        }

        let df = DROPFILES {
            pFiles: header as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: BOOL(0),
            fWide: BOOL(1),
        };
        std::ptr::write(base as *mut DROPFILES, df);

        let mut w = base.add(header) as *mut u16;
        for p in paths {
            for &c in p {
                std::ptr::write(w, c);
                w = w.add(1);
            }
            std::ptr::write(w, 0);
            w = w.add(1);
        }
        std::ptr::write(w, 0); // 이중 널 종료

        let _ = GlobalUnlock(hglobal);
        Ok(hglobal)
    }

    pub fn run(
        app: tauri::AppHandle,
        archive: String,
        dll: PathBuf,
        password: Option<String>,
        items: Vec<DragItem>,
    ) -> HRESULT {
        unsafe {
            let _ = OleInitialize(None);

            let data: IDataObject = HdropData {
                app,
                archive,
                dll,
                password,
                items,
                temp_dir: Mutex::new(None),
            }
            .into();
            let source: IDropSource = DropSource.into();
            let mut effect = DROPEFFECT::default();
            // 아카이브에서 꺼내는 동작 → 복사(Copy)만 허용
            DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect)
        }
    }
}
