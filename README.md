[한국어](README.ko.md) | **English** | [日本語](README.ja.md) | [简体中文](README.zh-CN.md)

# ZipMania

![Platform](https://img.shields.io/badge/platform-Windows-blue)
![Formats](https://img.shields.io/badge/read-50%2B_formats-green)
![License](https://img.shields.io/badge/license-Apache_2.0-lightgrey)

A Windows archive manager that opens 50+ archive formats and creates **7Z · ZIP · TAR**.
Fast ZIP processing, built-in preview, and Explorer right-click integration.

## Features

* Open **50+ formats** including ZIP, RAR, 7Z, EGG, ALZ and ISO
* Create **7Z · ZIP · TAR** archives
* Preview images and text without extracting
* Extract only the files you select
* Open an archive nested inside another archive
* Password protection and split archives
* Command-line compression (Total Commander and other external callers)
* Add/remove entries and run integrity checks
* Windows security scanning (AMSI)
* Explorer right-click menu and file associations
* Dark/light theme
* **9 languages**

## Fast ZIP Processing

A dedicated ZIP engine handles ZIP archives directly.

| Operation                   |   vs. 7z.dll |
| --------------------------- | -----------: |
| Large-file compression      | **up to 7.6×** |
| Many small files            | **up to 6.5×** |
| Extraction                  | **up to 2.2×** |

## Supported Formats

**Read:** 7Z, ZIP, ZIPX, JAR, RAR, EGG, ALZ, TAR, GZ, BZ2, XZ, ZST, ISO, IMG, WIM, DMG, MSI, RPM, DEB, CBZ, CBR and 50+ more

**Write:** `7z` · `zip` · `tar`

## Command Line

A console command that compresses without opening a window (for external callers such as Total Commander). Options follow Bandizip.

```
ZipMania.exe c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"
```

| Option | Meaning |
| --- | --- |
| `-l:0..9` | Compression level |
| `-fmt:zip\|7z\|tar` | Format (defaults to the output extension) |
| `-v:700M` | Volume size for split archives (K/M/G) |
| `-p:password` | Password |
| `-t:N` | Thread count (7z) |
| `-aou` | Rename to `name (2)` if the target exists |
| `-aos` | Skip if the target exists |
| `-testdst` | Verify the archive after creation |
| `-delsrc` | Delete the sources when verification passes |
| `-date` | Replace `%Y %y %m %d %H %M %S` in the file name with the current time |

Exit code is 0 (success) / 1 (warning: missing items, failed check, skipped) / 2 (error). Run `ZipMania.exe --help` for the full option list.

## Requirements

* Windows 10 or later
* Microsoft WebView2 Runtime

## Author

**Kilho.net** · [https://v2.kilho.net/zipmania](https://v2.kilho.net/zipmania)

My very first vibe-coded project. Thoughts and suggestions are always welcome.

## License

The program itself is **freeware** - free for anyone, at home, at work, at
school or in government, with no ads and no bundled installers.

The source code is licensed under the **Apache License 2.0** (see `LICENSE`).
The reusable crates under `crates/` are dual licensed as **MIT OR Apache-2.0**.
The "ZipMania" name, logos and icons are trademarks of Kilho.net and are not
covered by that license - distribute modified versions under a different name.

ZipMania bundles 7-Zip's `7z.dll` (LGPL) and other open-source components.
See `THIRD-PARTY-NOTICES.txt`.
