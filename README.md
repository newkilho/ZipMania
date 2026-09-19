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
* Command-line compression and extraction (window or console; Total Commander and other external callers)
* Verify the archive and delete the sources after compression
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

Compress, extract, list and test from the command line. Both Bandizip and 7-Zip option spellings are accepted, and there are two executables — the same split as Bandizip's `Bandizip.exe`/`bc.exe` and 7-Zip's `7zG.exe`/`7z.exe`.

* **`ZipMania.exe <command> …`** — shows the progress in a **window**. Use it from places without a console (Total Commander, shortcuts). Compression and extraction get a progress window; list and test open the archive in the main window.
* **`zm.exe <command> …`** — prints to the **console**. cmd and scripts wait for it and receive the exit code (0/1/2). Giving `zm` anything but a command is an error.

`ZipMania.exe <file>` opens that file in the window.

### Usage

```
<exe> a|c [options] <archive> <input...>    add / create
<exe> x|e [options] <archive> [entry...]    extract (x keeps paths, e flattens)
<exe> bx  [options] <archive...>            extract each archive into a folder named after it
<exe> l   [options] <archive>               list
<exe> t   [options] <archive>               test
```

`<exe>` is `ZipMania.exe` (progress window) or `zm` (console); the parameters are the same for both. Options take both `-name:value` (Bandizip) and `-namevalue` (7-Zip). Items after the archive are the inputs when creating, or the entries to extract (none = all). `zm` alone prints the help.

### Examples

```
ZipMania.exe c -l:9 -fmt:7z -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"        # progress window, verify then delete sources
zm c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"   # the same in the console, split 7z
zm a -mx9 -psecret backup.7z D:\Work                                              # 7-Zip spelling, add to an existing archive
zm x -o:D:\Out -target:auto backup.7z                                             # extract into a folder named after the archive
zm e -y backup.zip docs\readme.txt                                                  # one entry, no paths, overwrite
zm bx a.zip b.7z                                                                       # extract into a\ and b\ respectively
zm l backup.7z.001                                                                     # list the first volume of a split set
```

| Verb | Meaning |
| --- | --- |
| `a` / `c` | Compress (`a` adds to an existing archive) |
| `x` / `e` | Extract (`e` flattens the folder structure) |
| `bx` | Extract several archives, each into a folder named after it (Bandizip `bx`; `-target:name` by default) |
| `l` / `t` | List / integrity test |

| Option | Meaning |
| --- | --- |
| `-l:0..9` `-mx9` | Compression level |
| `-fmt:zip\|7z\|tar` `-t7z` | Format (defaults to the archive extension) |
| `-v:700M` `-v700m` | Volume size for split archives (K/M/G) |
| `-p:password` `-ppassword` | Password |
| `-t:N` `-mmt=N` | Thread count (7z) |
| `-o:dir` `-odir` | Extraction folder (default: current folder) |
| `-target:auto\|name\|none` | Extract into a subfolder named after the archive (`auto` = only when there is more than one top-level item) |
| `-aoa` `-y` / `-aos` / `-aou` | If the target exists: overwrite / skip / rename to `name (2)` (create defaults to overwrite, extract to skip) |
| `-testdst` | Verify the archive after creation |
| `-delsrc` `-sdel` | Delete the sources when verification passes |
| `-date` | Replace `%Y %y %m %d %H %M %S` in the file name with the current time |
| `-r` `-bd` `-bb*` `-bs*` `-cp:*` | Ignored (7-Zip / Bandizip compatibility) |

When `ZipMania.exe` shows a window, `-t:<n>` (threads) is ignored and the window asks about overwriting (`-aoa`/`-aos`/`-aou`) and passwords on extraction. Adding to an existing archive (`a`) is `zm` only.

Exit code is 0 (success) / 1 (warning: missing items, failed check, skipped) / 2 (error). Inside a `.bat` file write `%` as `%%` (the cmd prompt, PowerShell and Total Commander pass it through as is).

Total Commander user command example — command `ZipMania.exe` (progress window) or `zm.exe` (console window), parameters `c -l:9 -fmt:7z -aou -testdst -delsrc -date "%T%S %y%m%d_%H%M".7z "%P%S"` (packs the selection into a dated 7z in the target panel folder).

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
