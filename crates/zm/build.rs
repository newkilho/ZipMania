// zm.exe 리소스 = 앱 아이콘 + 버전 정보(파일 설명, 제품명), 앱과 같은 icon.ico
fn main() {
    #[cfg(windows)]
    {
        let mut res = tauri_winres::WindowsResource::new();
        res.set_icon("../../src-tauri/icons/icon.ico");
        res.set("FileDescription", "ZipMania console launcher");
        res.set("ProductName", "ZipMania");
        res.set("OriginalFilename", "zm.exe");
        res.set("CompanyName", "Kilho.net");
        res.set("LegalCopyright", "Kilho.net");
        res.compile().expect("zm resource");
        println!("cargo:rerun-if-changed=../../src-tauri/icons/icon.ico");
    }
}
