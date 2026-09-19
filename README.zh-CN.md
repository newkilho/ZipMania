[한국어](README.ko.md) | [English](README.md) | [日本語](README.ja.md) | **简体中文**

# ZipMania

![Platform](https://img.shields.io/badge/platform-Windows-blue)
![Formats](https://img.shields.io/badge/read-50%2B_formats-green)
![License](https://img.shields.io/badge/license-Apache_2.0-lightgrey)

可打开 50 多种压缩格式，并创建 **7Z · ZIP · TAR** 的 Windows 压缩软件。
支持快速 ZIP 处理、预览以及资源管理器右键菜单。

## 功能

* 打开 ZIP、RAR、7Z、EGG、ALZ、ISO 等 **50 多种格式**
* 创建 **7Z · ZIP · TAR** 压缩文件
* 无需解压即可预览图片和文本
* 只解压所选择的文件
* 直接打开压缩文件中的压缩文件
* 密码保护与分卷压缩
* 命令行压缩、解压（窗口或控制台，Total Commander 等外部调用）
* 压缩后完整性检测、删除源文件
* 添加/删除压缩文件内容及完整性检查
* Windows 安全扫描（AMSI）
* 资源管理器右键菜单与文件关联
* 深色/浅色主题
* **支持 9 种语言**

## 快速 ZIP 处理

使用 ZIP 专用引擎直接处理。

| 操作        |     相比 7z.dll |
| --------- | -----------: |
| 大文件压缩     | **最高 7.6×** |
| 多个小文件压缩   | **最高 6.5×** |
| 解压        | **最高 2.2×** |

## 支持的格式

**打开：** 7Z, ZIP, ZIPX, JAR, RAR, EGG, ALZ, TAR, GZ, BZ2, XZ, ZST, ISO, IMG, WIM, DMG, MSI, RPM, DEB, CBZ, CBR 等 50 多种

**压缩：** `7z` · `zip` · `tar`

## 命令行

通过命令行进行压缩、解压、列表和检测。同时接受 Bandizip 和 7-Zip 的选项写法,并有两个可执行文件 — 与 Bandizip 的 `Bandizip.exe`/`bc.exe`、7-Zip 的 `7zG.exe`/`7z.exe` 相同的划分。

* **`ZipMania.exe <命令> …`** — 以**窗口**显示进度。适合 Total Commander、快捷方式等没有控制台的场合。压缩、解压显示进度窗口;列表、检测则在主窗口中打开该压缩包。
* **`zm.exe <命令> …`** — 输出到**控制台**。cmd 和脚本会等待它结束并获得退出码(0/1/2)。给 `zm` 传入命令以外的内容会报错。

`ZipMania.exe <文件>` 会在窗口中打开该文件。

### 用法

```
<exe> a|c [选项] <压缩包> <输入...>    压缩 / 追加
<exe> x|e [选项] <压缩包> [项目...]    解压(x 保留路径,e 不保留)
<exe> bx  [选项] <压缩包...>           把每个压缩包解压到以它命名的目录
<exe> l   [选项] <压缩包>              列表
<exe> t   [选项] <压缩包>              完整性检测
```

`<exe>` 是 `ZipMania.exe`(进度窗口)或 `zm`(控制台),两者参数相同。选项同时支持 `-名称:值`(Bandizip)和 `-名称值`(7-Zip)两种写法。压缩包之后的项目在压缩时是输入文件/目录,在解压时是要取出的项目(省略 = 全部)。只输入 `zm` 会显示帮助。

### 示例

```
ZipMania.exe c -l:9 -fmt:7z -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"        # 进度窗口,检测后删除源文件
zm c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"   # 同样的操作在控制台,分卷 7z
zm a -mx9 -psecret backup.7z D:\Work                                              # 7-Zip 写法,追加到已有压缩包
zm x -o:D:\Out -target:auto backup.7z                                             # 解压到以压缩包命名的目录
zm e -y backup.zip docs\readme.txt                                                  # 仅一个项目,不保留路径,覆盖
zm bx a.zip b.7z                                                                       # 分别解压到 a\ 和 b\ 目录
zm l backup.7z.001                                                                     # 列出分卷的第一卷
```

| 动词 | 含义 |
| --- | --- |
| `a` / `c` | 压缩(`a` 在已有压缩包时追加) |
| `x` / `e` | 解压(`e` 不保留目录结构) |
| `bx` | 把多个压缩包分别解压到以各自命名的目录(Bandizip 的 `bx`,默认 `-target:name`) |
| `l` / `t` | 列表 / 完整性检测 |

| 选项 | 含义 |
| --- | --- |
| `-l:0..9` `-mx9` | 压缩级别 |
| `-fmt:zip\|7z\|tar` `-t7z` | 格式(省略则按压缩包扩展名) |
| `-v:700M` `-v700m` | 分卷大小(K/M/G) |
| `-p:密码` `-p密码` | 密码 |
| `-t:N` `-mmt=N` | 线程数(7z) |
| `-o:目录` `-o目录` | 解压目标目录(默认当前目录) |
| `-target:auto\|name\|none` | 解压到以压缩包命名的子目录(`auto` = 仅当顶层项目多于一个时) |
| `-aoa` `-y` / `-aos` / `-aou` | 同名已存在时:覆盖 / 跳过 / 改名为 `名称 (2)`(压缩默认覆盖,解压默认跳过) |
| `-testdst` | 压缩后进行完整性检测 |
| `-delsrc` `-sdel` | 检测通过后删除源文件 |
| `-date` | 将文件名中的 `%Y %y %m %d %H %M %S` 替换为当前时间 |
| `-r` `-bd` `-bb*` `-bs*` `-cp:*` | 忽略(兼容 7-Zip / Bandizip) |

用 `ZipMania.exe` 打开窗口时,`-t:<n>`(线程)会被忽略,解压时的覆盖(`-aoa`/`-aos`/`-aou`)和密码由窗口询问。向已有压缩包追加(`a`)仅限 `zm`。

退出码为 0(成功)/ 1(警告:缺失项目、检测失败、跳过)/ 2(错误)。在 `.bat` 文件中请把 `%` 写成 `%%`(cmd 提示符、PowerShell 和 Total Commander 会原样传递)。

Total Commander 用户命令示例 — 命令 `ZipMania.exe`(进度窗口)或 `zm.exe`(控制台),参数 `c -l:9 -fmt:7z -aou -testdst -delsrc -date "%T%S %y%m%d_%H%M".7z "%P%S"`(把所选项目压缩为目标面板目录下的日期命名 7z)。

## 系统要求

* Windows 10 或更高版本
* Microsoft WebView2 Runtime

## 作者

**Kilho.net** · [https://v2.kilho.net/zipmania](https://v2.kilho.net/zipmania)

这是我人生第一次用「vibe coding」做出的程序。欢迎随时提出意见和建议。

## 许可证

程序本身是**免费软件**。公司、家庭、政府机关、学校均可无限制免费使用，
没有广告，也没有捆绑安装。

源代码采用 **Apache License 2.0**（见 `LICENSE`）。`crates/` 下的可复用
crate 采用 **MIT 或 Apache-2.0** 双许可。“ZipMania”名称、徽标和图标是
Kilho.net 的商标，不在该许可证范围内 — 分发修改版本时请使用其他名称和图标。

ZipMania 捆绑了 7-Zip 的 `7z.dll`（LGPL）等开源组件，
详见 `THIRD-PARTY-NOTICES.txt`。
