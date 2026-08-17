//! 확장자, 폴더 → Windows 시스템 아이콘 PNG, SHGetFileInfoW + GDI → RGBA8 PNG, 비-Windows = None 스텁

/// 아이콘 추출 직렬화 잠금, SHGetFileInfoW 동시 호출 경합 방지
#[cfg(windows)]
static ICON_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 확장자(소문자, 점 없음) + 폴더 여부 → 16x16 시스템 아이콘 RGBA8 PNG, 실패 = None
#[cfg(windows)]
pub fn icon_png(ext: &str, is_dir: bool) -> Option<Vec<u8>> {
    use windows::core::PCWSTR;

    // 동시 접근 경합 방지, 잠금 오염돼도 그대로 진행
    let _guard = ICON_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    // SHGetFileInfoW 는 호출 스레드에 COM 초기화 필요, 이 함수가 초기화한 경우에만 해제
    let com_hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let com_owned = com_hr.is_ok();

    // USEFILEATTRIBUTES 사용 → 실제 파일 불필요, 폴더 = x, 파일 = x.<ext>
    let dummy = if is_dir || ext.is_empty() {
        "x".to_string()
    } else {
        format!("x.{ext}")
    };
    let attrs = if is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };

    // 널 종료 UTF-16 문자열
    let wide: Vec<u16> = dummy.encode_utf16().chain(std::iter::once(0)).collect();

    let mut info = SHFILEINFOW::default();
    let flags = SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES;

    // SHGetFileInfoW 로 작은(16x16) 아이콘 핸들을 얻는다
    let result = unsafe {
        let ret = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            attrs,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        if ret == 0 || info.hIcon.0.is_null() {
            None
        } else {
            // 아이콘을 PNG 로 변환(성공/실패 무관하게 반드시 DestroyIcon)
            let png = icon_to_png(info.hIcon);
            let _ = DestroyIcon(info.hIcon);
            png
        }
    };

    if com_owned {
        unsafe { CoUninitialize() };
    }
    result
}

/// HICON → RGBA8 PNG, GDI 객체(hbmColor/hbmMask) 해제 필수
#[cfg(windows)]
fn icon_to_png(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut icon_info = ICONINFO::default();
    if unsafe { GetIconInfo(hicon, &mut icon_info) }.is_err() {
        return None;
    }
    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    let png = extract_and_encode(hbm_color, hbm_mask);

    // GetIconInfo 가 생성한 비트맵 해제
    unsafe {
        if !hbm_color.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        }
        if !hbm_mask.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        }
    }
    png
}

/// 컬러, 마스크 비트맵 → PNG
#[cfg(windows)]
fn extract_and_encode(
    hbm_color: windows::Win32::Graphics::Gdi::HBITMAP,
    hbm_mask: windows::Win32::Graphics::Gdi::HBITMAP,
) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, GetObjectW, BITMAP, HGDIOBJ,
    };

    if hbm_color.0.is_null() {
        return None;
    }

    // 아이콘 크기(폭, 높이) 확인
    let mut bmp = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(hbm_color.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut _ as *mut c_void),
        )
    };
    if got == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
        return None;
    }
    let width = bmp.bmWidth;
    let height = bmp.bmHeight;

    // 메모리 DC(화면 호환) 생성 — GetDIBits 에 필요
    let hdc = unsafe { CreateCompatibleDC(None) };
    if hdc.0.is_null() {
        return None;
    }

    let pixels = extract_rgba(hdc, hbm_color, hbm_mask, width, height);

    unsafe {
        let _ = DeleteDC(hdc);
    }

    let pixels = pixels?;
    encode_png(width as u32, height as u32, &pixels)
}

/// GetDIBits(32bpp, top-down) → BGRA → RGBA, 알파가 전부 0(구형 아이콘)이면 AND 마스크로 복원
#[cfg(windows)]
fn extract_rgba(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    hbm_color: windows::Win32::Graphics::Gdi::HBITMAP,
    hbm_mask: windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    use windows::Win32::Graphics::Gdi::{
        GetDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };

    let pixel_count = (width as usize) * (height as usize);
    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = width;
    // 음수 높이 = top-down(첫 행이 이미지 상단)
    bi.bmiHeader.biHeight = -height;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB.0 as u32;

    // 컬러 비트맵의 BGRA 픽셀 추출
    let mut pixels = vec![0u8; pixel_count * 4];
    let scan = unsafe {
        GetDIBits(
            hdc,
            hbm_color,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut c_void),
            &mut bi,
            DIB_RGB_COLORS,
        )
    };
    if scan == 0 {
        return None;
    }

    // BGRA → RGBA (B 와 R 스왑)
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    // 알파 전부 0(구형 32bpp) 시 AND 마스크로 알파 복원
    let all_zero_alpha = pixels.chunks_exact(4).all(|px| px[3] == 0);
    if all_zero_alpha {
        let mut mask = vec![0u8; pixel_count * 4];
        let mut bim = BITMAPINFO::default();
        bim.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bim.bmiHeader.biWidth = width;
        bim.bmiHeader.biHeight = -height;
        bim.bmiHeader.biPlanes = 1;
        bim.bmiHeader.biBitCount = 32;
        bim.bmiHeader.biCompression = BI_RGB.0 as u32;

        let scan_mask = unsafe {
            GetDIBits(
                hdc,
                hbm_mask,
                0,
                height as u32,
                Some(mask.as_mut_ptr() as *mut c_void),
                &mut bim,
                DIB_RGB_COLORS,
            )
        };
        if scan_mask != 0 && !hbm_mask.0.is_null() {
            // AND 마스크: 검정(0)=불투명, 흰색(!=0)=투명
            for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
                px[3] = if mask[i * 4] == 0 { 255 } else { 0 };
            }
        } else {
            // 마스크도 못 얻으면 전부 불투명 처리
            for px in pixels.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
    }

    Some(pixels)
}

/// RGBA8 → PNG 바이트
#[cfg(windows)]
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// 비-Windows 스텁, 항상 None
#[cfg(not(windows))]
pub fn icon_png(_ext: &str, _is_dir: bool) -> Option<Vec<u8>> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// PNG 시그니처 8바이트
    const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn txt_확장자_아이콘_png_로_인코딩() {
        let png = icon_png("txt", false).expect("txt 아이콘을 얻지 못했습니다");
        assert!(!png.is_empty(), "PNG 바이트가 비어 있습니다");
        assert!(
            png.starts_with(PNG_SIG),
            "PNG 시그니처로 시작하지 않습니다: {:?}",
            &png[..png.len().min(8)]
        );
    }

    #[test]
    fn 폴더_아이콘_png_로_인코딩() {
        let png = icon_png("", true).expect("폴더 아이콘을 얻지 못했습니다");
        assert!(!png.is_empty(), "폴더 PNG 바이트가 비어 있습니다");
        assert!(png.starts_with(PNG_SIG), "폴더 PNG 시그니처 불일치");
    }

    #[test]
    fn 확장자없는_파일_아이콘_png() {
        // 확장자 부재 시에도 기본 파일 아이콘 PNG 획득 필요
        let png = icon_png("", false).expect("확장자 없는 파일 아이콘 실패");
        assert!(png.starts_with(PNG_SIG));
    }
}
