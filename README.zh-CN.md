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

## 系统要求

* Windows 10 或更高版本
* Microsoft WebView2 Runtime

## 作者

**Kilho.net** · [https://v2.kilho.net](https://v2.kilho.net)

## 许可证

程序本身是**免费软件**。公司、家庭、政府机关、学校均可无限制免费使用，
没有广告，也没有捆绑安装。

源代码采用 **Apache License 2.0**（见 `LICENSE`）。`crates/` 下的可复用
crate 采用 **MIT 或 Apache-2.0** 双许可。“ZipMania”名称、徽标和图标是
Kilho.net 的商标，不在该许可证范围内 — 分发修改版本时请使用其他名称和图标。

ZipMania 捆绑了 7-Zip 的 `7z.dll`（LGPL）等开源组件，
详见 `THIRD-PARTY-NOTICES.txt`。
