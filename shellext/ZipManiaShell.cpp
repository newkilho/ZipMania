// ZipMania 탐색기 셸 확장, 클래식 IContextMenu + IShellExtInit / IExplorerCommand(C++/WinRT, Windows SDK 만)
// IContextMenu = 레거시 메뉴, HKCU 등록, IExplorerCommand = Win11 기본 메뉴, 스파스 MSIX 등록 (D3.7)
// 메뉴 구성은 BuildEntries 한 곳, 두 경로가 같은 목록을 쓴다
//
// 메뉴 = 모두 최상위 평면
//  비-아카이브 "{이름}.zip"(으)로 압축하기 / 집매니아로 압축하기 (여러 개면 각각 압축하기 추가)
//  아카이브 여기에 풀기 / 알아서 풀기 / "{이름}에 풀기" / 집매니아로 압축 풀기 / 집매니아로
//  열기 (여러 개면 각각 알아서 풀기 / 각각 파일명 폴더에 풀기 추가)
// 선택이 모두 아카이브면 압축 항목은 아예 넣지 않는다(진짜 숨김)
//
// 각 항목의 Invoke = 선택 항목 전체를 ZipMania.exe --<스위치> "<경로>"… 로 실행

#include <windows.h>
#include <shlobj.h>
#include <shobjidl_core.h>
#include <shlwapi.h>
#include <shellapi.h>
#include <string>
#include <vector>
#include <cwctype>
#include <cwchar>
#include <cstdio>
#include <winrt/base.h>

// 메뉴 문구(MenuText, kMenuTexts) — 생성물, 정본은 zipmania-i18n 의 strings.rs
#include "strings.generated.h"

using namespace winrt;

#pragma comment(lib, "shlwapi.lib")
#pragma comment(lib, "shell32.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "oleaut32.lib")
#pragma comment(lib, "runtimeobject.lib")
#pragma comment(lib, "advapi32.lib")
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "gdi32.lib")

// 컨텍스트 메뉴 핸들러 CLSID — shell_reg.rs 와 일치, {02BEA257-B0A9-4B99-9A99-F3F61885D771}
static constexpr GUID CLSID_ZipManiaMenu = {
    0x02BEA257, 0xB0A9, 0x4B99, {0x9A, 0x99, 0xF3, 0xF6, 0x18, 0x85, 0xD7, 0x71}};

// IExplorerCommand 루트 CLSID — AppxManifest.xml 과 일치, {F58510CC-5B40-48D9-A9FE-120FD5F23DAE}
static constexpr GUID CLSID_ZipManiaCommand = {
    0xF58510CC, 0x5B40, 0x48D9, {0xA9, 0xFE, 0x12, 0x0F, 0xD5, 0xF2, 0x3D, 0xAE}};

static HMODULE g_module = nullptr;
static HBITMAP g_menuBitmap = nullptr; // 메뉴 항목용 ZipMania.exe 아이콘 비트맵(1회 생성, 캐시)

// 아카이브 확장자(소문자, 점 없음) — 정본 READ_EXTS 의 사본, ext_tests 가 대조
// 추가 절차는 (D3.8)
static const wchar_t* kArchiveExts[] = {
    L"7z",   L"zip",  L"zipx", L"jar",  L"rar",  L"r00",  L"arj",      L"lzh",
    L"lha",  L"cab",  L"tar",  L"ova",  L"gz",   L"gzip", L"tgz",      L"tpz",
    L"bz2",  L"bzip2", L"tbz", L"tbz2", L"xz",   L"txz",  L"zst",      L"tzst",
    L"z",    L"taz",  L"lzma", L"iso",  L"img",  L"udf",  L"wim",      L"swm",
    L"esd",  L"dmg",  L"squashfs",      L"msi",  L"msp",  L"msm",      L"cpio",
    L"rpm",  L"deb",  L"xar",  L"pkg",  L"chm",  L"nsis", L"001",      L"cbz",
    L"cbr",  L"cb7",  L"egg",  L"alz"};

// ── 유틸 ─────────────────────────────────────────────────────────────────────

static std::wstring ModuleDir()
{
    wchar_t buf[MAX_PATH]{};
    GetModuleFileNameW(g_module, buf, MAX_PATH);
    std::wstring p(buf);
    size_t slash = p.find_last_of(L"\\/");
    return slash == std::wstring::npos ? p : p.substr(0, slash);
}

// 아래 ExePath 가 먼저 쓴다
static std::wstring ParentDir(const std::wstring& path);

// 앱 실행 파일 경로, HKCU\Software\ZipMania\ShellExt\ExePath 우선
// 폴백은 DLL 옆이 아니라 두 단계 위, DLL 자리가 <설치 루트>\shell\x64
static std::wstring ExePath()
{
    wchar_t buf[MAX_PATH]{};
    DWORD cb = sizeof(buf);
    if (RegGetValueW(HKEY_CURRENT_USER, L"Software\\ZipMania\\ShellExt", L"ExePath", RRF_RT_REG_SZ,
                     nullptr, buf, &cb) == ERROR_SUCCESS &&
        buf[0])
    {
        return buf;
    }
    return ParentDir(ParentDir(ModuleDir())) + L"\\ZipMania.exe";
}

static std::wstring ToLower(std::wstring s)
{
    for (auto& c : s) c = (wchar_t)towlower(c);
    return s;
}

static std::wstring FileName(const std::wstring& path)
{
    size_t slash = path.find_last_of(L"\\/");
    return slash == std::wstring::npos ? path : path.substr(slash + 1);
}

static std::wstring ExtOf(const std::wstring& path)
{
    std::wstring name = FileName(path);
    size_t dot = name.find_last_of(L'.');
    return dot == std::wstring::npos ? L"" : ToLower(name.substr(dot + 1));
}

// 확장자 하나 제거한 이름(예: photo.png → photo, backup.tar.gz → backup.tar)
static std::wstring Stem(const std::wstring& path)
{
    std::wstring name = FileName(path);
    size_t dot = name.find_last_of(L'.');
    return (dot == std::wstring::npos || dot == 0) ? name : name.substr(0, dot);
}

// 폴더 여부, 경로가 없거나 읽지 못하면 false
static bool IsDirectory(const std::wstring& path)
{
    DWORD attr = GetFileAttributesW(path.c_str());
    return attr != INVALID_FILE_ATTRIBUTES && (attr & FILE_ATTRIBUTE_DIRECTORY) != 0;
}

// 아카이브 여부, 폴더는 이름이 확장자처럼 보여도 아니다
static bool IsArchive(const std::wstring& path)
{
    if (IsDirectory(path)) return false;
    std::wstring ext = ExtOf(path);
    for (auto* e : kArchiveExts)
        if (ext == e) return true;
    return false;
}

// 압축 결과 이름의 바탕, 폴더는 이름 그대로, 파일은 확장자 하나 제거
static std::wstring ArchiveStem(const std::wstring& path)
{
    return IsDirectory(path) ? FileName(path) : Stem(path);
}

// 경로의 부모 폴더 경로
static std::wstring ParentDir(const std::wstring& path)
{
    size_t slash = path.find_last_of(L"\\/");
    return slash == std::wstring::npos ? L"" : path.substr(0, slash);
}

// 경로가 든 폴더의 이름(다중 선택 시 "현재 폴더명")
static std::wstring ParentFolderName(const std::wstring& path)
{
    return FileName(ParentDir(path));
}

// OS UI 언어 → 언어 코드, 표에 없는 것은 영어
static const wchar_t* OsLanguage()
{
    switch (PRIMARYLANGID(GetUserDefaultUILanguage()))
    {
    case LANG_KOREAN: return L"ko";
    case LANG_JAPANESE: return L"ja";
    case LANG_CHINESE: return L"zh";
    case LANG_RUSSIAN: return L"ru";
    case LANG_ITALIAN: return L"it";
    case LANG_FRENCH: return L"fr";
    case LANG_SPANISH: return L"es";
    case LANG_ARABIC: return L"ar";
    default: return L"en";
    }
}

// 설정(settings.toml)의 언어, 지원 코드면 그것, 그 외(system/누락)는 OS 기본으로 추정
static const MenuText& MenuTextForUi()
{
    std::wstring lang;
    {
        // 설정 = 앱 실행 파일 옆(settings.rs 의 settings_file 과 같은 규칙 — 한쪽만 변경 금지)
        std::wstring path = ParentDir(ExePath()) + L"\\settings.toml";
        FILE* f = _wfopen(path.c_str(), L"rb");
        if (f)
        {
            char line[512];
            while (fgets(line, sizeof(line), f))
            {
                std::string s(line);
                auto pos = s.find("language");
                if (pos == std::string::npos) continue;
                auto q1 = s.find('"', pos);
                auto q2 = (q1 == std::string::npos) ? std::string::npos : s.find('"', q1 + 1);
                if (q1 != std::string::npos && q2 != std::string::npos)
                {
                    std::string v = s.substr(q1 + 1, q2 - q1 - 1);
                    lang.assign(v.begin(), v.end());
                }
                break;
            }
            fclose(f);
        }
    }

    for (const auto& e : kMenuTexts)
        if (lang == e.code) return e.text;

    // system/누락/모르는 값 = OS 기본
    const wchar_t* os = OsLanguage();
    for (const auto& e : kMenuTexts)
        if (wcscmp(os, e.code) == 0) return e.text;

    // 표의 첫 줄은 ko 이므로 폴백은 이름으로 찾는다
    for (const auto& e : kMenuTexts)
        if (wcscmp(L"en", e.code) == 0) return e.text;

    return kMenuTexts[0].text;
}

// ZipMania.exe 를 스위치 + 경로들로 실행
static void Launch(const std::wstring& verb, const std::vector<std::wstring>& files)
{
    std::wstring cmd = L"\"" + ExePath() + L"\" " + verb;
    for (const auto& p : files)
        cmd += L" \"" + p + L"\"";

    std::vector<wchar_t> buf(cmd.begin(), cmd.end());
    buf.push_back(L'\0');

    STARTUPINFOW si{};
    si.cb = sizeof(si);
    PROCESS_INFORMATION pi{};
    std::wstring dir = ModuleDir();
    if (CreateProcessW(nullptr, buf.data(), nullptr, nullptr, FALSE, 0, nullptr, dir.c_str(), &si,
                       &pi))
    {
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
}

// HICON → 32bpp HBITMAP(작은 아이콘 크기), 메뉴 항목 비트맵용
static HBITMAP IconToBitmap(HICON icon)
{
    int cx = GetSystemMetrics(SM_CXSMICON);
    int cy = GetSystemMetrics(SM_CYSMICON);

    BITMAPINFO bi{};
    bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bi.bmiHeader.biWidth = cx;
    bi.bmiHeader.biHeight = -cy; // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;

    HDC dc = CreateCompatibleDC(nullptr);
    void* bits = nullptr;
    HBITMAP bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &bits, nullptr, 0);
    if (bmp)
    {
        HGDIOBJ old = SelectObject(dc, bmp);
        DrawIconEx(dc, 0, 0, icon, cx, cy, 0, nullptr, DI_NORMAL);
        SelectObject(dc, old);
    }
    DeleteDC(dc);
    return bmp;
}

// ZipMania.exe 아이콘을 메뉴용 비트맵으로(1회 생성 후 캐시), 실패하면 nullptr
static HBITMAP MenuBitmap()
{
    if (!g_menuBitmap)
    {
        std::wstring exe = ExePath();
        HICON icon = ExtractIconW(g_module, exe.c_str(), 0);
        if (icon && icon != reinterpret_cast<HICON>(1))
        {
            g_menuBitmap = IconToBitmap(icon);
            DestroyIcon(icon);
        }
    }
    return g_menuBitmap;
}

// ── 메뉴 목록(두 경로 공용) ──────────────────────────────────────────────────

// 메뉴 항목 하나, verb = ZipMania.exe 에 넘길 CLI 스위치
struct MenuEntry
{
    std::wstring label;
    const wchar_t* verb;
};

// 선택 항목 → 메뉴 목록, 선택이 단일 아카이브면 압축 항목 제외
static std::vector<MenuEntry> BuildEntries(const std::vector<std::wstring>& files)
{
    std::vector<MenuEntry> out;
    if (files.empty()) return out;

    const MenuText& tx = MenuTextForUi();
    const size_t count = files.size();
    bool anyArchive = false;
    for (const auto& f : files)
        if (IsArchive(f)) anyArchive = true;

    const std::wstring stem = Stem(files.front());
    // 다중 선택 = 현재 폴더명, 단일 = ArchiveStem(폴더는 이름 그대로)
    const std::wstring name =
        count > 1 ? ParentFolderName(files.front()) : ArchiveStem(files.front());
    // 압축은 단일 아카이브일 때만 숨김(아카이브 여러 개면 하나로 묶어 압축 가능)
    const bool singleArchive = (count == 1 && anyArchive);

    auto add = [&](std::wstring label, const wchar_t* verb) {
        out.push_back({std::move(label), verb});
    };

    if (!singleArchive)
    {
        add(tx.compressZipPre + name + tx.compressZipPost, L"--compress-zip");
        add(tx.compress, L"--compress");
        if (count > 1) add(tx.compressEach, L"--compress-each");
    }

    if (anyArchive)
    {
        add(tx.extractHere, L"--extract-here");
        // 단일 선택 전용: {파일명}에 풀기 / 압축 풀기(폼) / 열기, (다중은 아래 "각각 …" 사용)
        if (count == 1)
        {
            add(tx.extractToPre + stem + tx.extractToPost, L"--extract-newfolder");
            add(tx.extract, L"--extract");
            add(tx.open, L"--open");
        }
        if (count > 1) add(tx.extractEach, L"--extract-each-newfolder");
    }

    return out;
}

// IShellItemArray → 파일 시스템 경로 목록, 경로가 없는 항목(가상 폴더)은 건너뜀
static std::vector<std::wstring> PathsOf(IShellItemArray* items)
{
    std::vector<std::wstring> out;
    if (!items) return out;

    DWORD count = 0;
    if (FAILED(items->GetCount(&count))) return out;
    for (DWORD i = 0; i < count; ++i)
    {
        com_ptr<IShellItem> item;
        if (FAILED(items->GetItemAt(i, item.put()))) continue;
        PWSTR path = nullptr;
        if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &path)) && path)
        {
            out.emplace_back(path);
            CoTaskMemFree(path);
        }
    }
    return out;
}

// ── IContextMenu + IShellExtInit 핸들러 ──────────────────────────────────────

struct MenuHandler : implements<MenuHandler, IShellExtInit, IContextMenu>
{
    // IShellExtInit — 선택 항목(CF_HDROP)을 읽어 둔다
    HRESULT __stdcall Initialize(PCIDLIST_ABSOLUTE, IDataObject* pdtobj, HKEY) noexcept override
    {
        m_files.clear();
        if (!pdtobj) return E_INVALIDARG;

        FORMATETC fe{};
        fe.cfFormat = CF_HDROP;
        fe.dwAspect = DVASPECT_CONTENT;
        fe.lindex = -1;
        fe.tymed = TYMED_HGLOBAL;

        STGMEDIUM stg{};
        if (FAILED(pdtobj->GetData(&fe, &stg))) return E_INVALIDARG;

        if (HDROP hdrop = static_cast<HDROP>(GlobalLock(stg.hGlobal)))
        {
            UINT n = DragQueryFileW(hdrop, 0xFFFFFFFF, nullptr, 0);
            wchar_t path[MAX_PATH];
            for (UINT i = 0; i < n; ++i)
                if (DragQueryFileW(hdrop, i, path, MAX_PATH))
                    m_files.emplace_back(path);
            GlobalUnlock(stg.hGlobal);
        }
        ReleaseStgMedium(&stg);
        return m_files.empty() ? E_INVALIDARG : S_OK;
    }

    // IContextMenu — 선택 항목 검사 후 조건부 메뉴 구성(모두 최상위 평면)
    HRESULT __stdcall QueryContextMenu(HMENU hmenu, UINT indexMenu, UINT idCmdFirst, UINT idCmdLast,
                                       UINT uFlags) noexcept override
    {
        if (uFlags & CMF_DEFAULTONLY)
            return MAKE_HRESULT(SEVERITY_SUCCESS, FACILITY_NULL, 0);

        m_verbs.clear();
        auto entries = BuildEntries(m_files);

        UINT pos = indexMenu;
        UINT cmd = 0;
        HBITMAP icon = MenuBitmap();
        for (const auto& e : entries)
        {
            // 배정된 ID 범위를 넘기면 그 자리에서 멈춘다
            if (idCmdFirst + cmd > idCmdLast) break;
            InsertMenuW(hmenu, pos, MF_BYPOSITION | MF_STRING, idCmdFirst + cmd, e.label.c_str());
            if (icon) SetMenuItemBitmaps(hmenu, pos, MF_BYPOSITION, icon, icon);
            m_verbs.emplace_back(e.verb);
            ++pos;
            ++cmd;
        }

        return MAKE_HRESULT(SEVERITY_SUCCESS, FACILITY_NULL, cmd);
    }

    HRESULT __stdcall InvokeCommand(CMINVOKECOMMANDINFO* pici) noexcept override
    {
        if (!pici) return E_INVALIDARG;
        // 문자열 verb(HIWORD!=0) 미지원, 오프셋만 처리
        if (HIWORD(pici->lpVerb) != 0) return E_FAIL;
        UINT id = LOWORD(reinterpret_cast<UINT_PTR>(pici->lpVerb));
        if (id >= m_verbs.size()) return E_INVALIDARG;
        Launch(m_verbs[id], m_files);
        return S_OK;
    }

    HRESULT __stdcall GetCommandString(UINT_PTR, UINT, UINT*, LPSTR, UINT) noexcept override
    {
        return E_NOTIMPL;
    }

  private:
    std::vector<std::wstring> m_files;
    std::vector<std::wstring> m_verbs; // 메뉴 오프셋 → CLI 스위치
};

// ── IExplorerCommand 핸들러(Win11 기본 메뉴) ─────────────────────────────────

// 하위 항목 하나, 루트의 하위 메뉴에 평면으로 놓인다
struct SubCommand : implements<SubCommand, IExplorerCommand>
{
    SubCommand(MenuEntry entry, std::vector<std::wstring> files)
        : m_entry(std::move(entry)), m_files(std::move(files))
    {
    }

    HRESULT __stdcall GetTitle(IShellItemArray*, LPWSTR* name) noexcept override
    {
        return SHStrDupW(m_entry.label.c_str(), name);
    }
    HRESULT __stdcall GetIcon(IShellItemArray*, LPWSTR* icon) noexcept override
    {
        // 하위 항목은 아이콘 없음, 루트에만 둔다
        *icon = nullptr;
        return E_NOTIMPL;
    }
    HRESULT __stdcall GetToolTip(IShellItemArray*, LPWSTR* tip) noexcept override
    {
        *tip = nullptr;
        return E_NOTIMPL;
    }
    HRESULT __stdcall GetCanonicalName(GUID* guid) noexcept override
    {
        *guid = GUID_NULL;
        return S_OK;
    }
    HRESULT __stdcall GetState(IShellItemArray*, BOOL, EXPCMDSTATE* state) noexcept override
    {
        *state = ECS_ENABLED;
        return S_OK;
    }
    HRESULT __stdcall Invoke(IShellItemArray* items, IBindCtx*) noexcept override
    {
        // 넘어온 항목 우선, 비어 있으면 루트가 떠 둔 목록
        std::vector<std::wstring> files = PathsOf(items);
        Launch(m_entry.verb, files.empty() ? m_files : files);
        return S_OK;
    }
    HRESULT __stdcall GetFlags(EXPCMDFLAGS* flags) noexcept override
    {
        *flags = ECF_DEFAULT;
        return S_OK;
    }
    HRESULT __stdcall EnumSubCommands(IEnumExplorerCommand** e) noexcept override
    {
        *e = nullptr;
        return E_NOTIMPL;
    }

  private:
    MenuEntry m_entry;
    std::vector<std::wstring> m_files;
};

struct CommandEnum : implements<CommandEnum, IEnumExplorerCommand>
{
    CommandEnum(std::vector<MenuEntry> entries, std::vector<std::wstring> files)
        : m_entries(std::move(entries)), m_files(std::move(files))
    {
    }

    HRESULT __stdcall Next(ULONG celt, IExplorerCommand** out, ULONG* fetched) noexcept override
    {
        ULONG n = 0;
        for (; n < celt && m_index < m_entries.size(); ++n, ++m_index)
        {
            try
            {
                out[n] = make<SubCommand>(m_entries[m_index], m_files)
                             .as<IExplorerCommand>()
                             .detach();
            }
            catch (...)
            {
                if (fetched) *fetched = n;
                return to_hresult();
            }
        }
        if (fetched) *fetched = n;
        return n == celt ? S_OK : S_FALSE;
    }
    HRESULT __stdcall Skip(ULONG celt) noexcept override
    {
        m_index += celt;
        return m_index <= m_entries.size() ? S_OK : S_FALSE;
    }
    HRESULT __stdcall Reset() noexcept override
    {
        m_index = 0;
        return S_OK;
    }
    HRESULT __stdcall Clone(IEnumExplorerCommand** out) noexcept override
    {
        *out = nullptr;
        return E_NOTIMPL;
    }

  private:
    std::vector<MenuEntry> m_entries;
    std::vector<std::wstring> m_files;
    size_t m_index = 0;
};

// 루트 항목, Win11 기본 메뉴에 "집매니아" 하나로 뜨고 하위에 실제 명령을 편다
// EnumSubCommands 에는 선택 항목이 넘어오지 않으므로 GetState/GetTitle 에서 떠 둔다
struct RootCommand : implements<RootCommand, IExplorerCommand>
{
    HRESULT __stdcall GetTitle(IShellItemArray* items, LPWSTR* name) noexcept override
    {
        Capture(items);
        return SHStrDupW(MenuTextForUi().appName, name);
    }
    HRESULT __stdcall GetIcon(IShellItemArray*, LPWSTR* icon) noexcept override
    {
        std::wstring path = ExePath() + L",0";
        return SHStrDupW(path.c_str(), icon);
    }
    HRESULT __stdcall GetToolTip(IShellItemArray*, LPWSTR* tip) noexcept override
    {
        *tip = nullptr;
        return E_NOTIMPL;
    }
    HRESULT __stdcall GetCanonicalName(GUID* guid) noexcept override
    {
        *guid = GUID_NULL;
        return S_OK;
    }
    HRESULT __stdcall GetState(IShellItemArray* items, BOOL, EXPCMDSTATE* state) noexcept override
    {
        Capture(items);
        // 낼 항목이 없으면 루트 자체를 숨긴다(빈 하위 메뉴 방지)
        *state = m_entries.empty() ? ECS_HIDDEN : ECS_ENABLED;
        return S_OK;
    }
    HRESULT __stdcall Invoke(IShellItemArray*, IBindCtx*) noexcept override { return S_OK; }
    HRESULT __stdcall GetFlags(EXPCMDFLAGS* flags) noexcept override
    {
        *flags = ECF_HASSUBCOMMANDS;
        return S_OK;
    }
    HRESULT __stdcall EnumSubCommands(IEnumExplorerCommand** e) noexcept override
    {
        *e = nullptr;
        try
        {
            *e = make<CommandEnum>(m_entries, m_files).as<IEnumExplorerCommand>().detach();
            return S_OK;
        }
        catch (...)
        {
            return to_hresult();
        }
    }

  private:
    void Capture(IShellItemArray* items)
    {
        if (!items || m_captured) return;
        m_captured = true;
        m_files = PathsOf(items);
        m_entries = BuildEntries(m_files);
    }

    std::vector<std::wstring> m_files;
    std::vector<MenuEntry> m_entries;
    bool m_captured = false;
};

// ── COM 클래스 팩토리 + DLL 진입점 ───────────────────────────────────────────

template <typename T> struct ClassFactory : implements<ClassFactory<T>, IClassFactory>
{
    HRESULT __stdcall CreateInstance(IUnknown* outer, REFIID riid, void** obj) noexcept override
    {
        if (obj) *obj = nullptr;
        if (outer) return CLASS_E_NOAGGREGATION;
        try
        {
            return make<T>()->QueryInterface(riid, obj);
        }
        catch (...)
        {
            return to_hresult();
        }
    }
    HRESULT __stdcall LockServer(BOOL lock) noexcept override
    {
        if (lock) ++get_module_lock();
        else --get_module_lock();
        return S_OK;
    }
};

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, void** instance)
{
    try
    {
        *instance = nullptr;
        if (rclsid == CLSID_ZipManiaMenu)
            return make<ClassFactory<MenuHandler>>()->QueryInterface(riid, instance);
        if (rclsid == CLSID_ZipManiaCommand)
            return make<ClassFactory<RootCommand>>()->QueryInterface(riid, instance);
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    catch (...)
    {
        return to_hresult();
    }
}

STDAPI DllCanUnloadNow()
{
    if (get_module_lock()) return S_FALSE;
    clear_factory_cache();
    return S_OK;
}

BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID)
{
    if (reason == DLL_PROCESS_ATTACH)
    {
        g_module = module;
        DisableThreadLibraryCalls(module);
    }
    else if (reason == DLL_PROCESS_DETACH)
    {
        if (g_menuBitmap)
        {
            DeleteObject(g_menuBitmap);
            g_menuBitmap = nullptr;
        }
    }
    return TRUE;
}
