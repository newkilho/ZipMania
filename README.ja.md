[한국어](README.ko.md) | [English](README.md) | **日本語** | [简体中文](README.zh-CN.md)

# ZipMania

![Platform](https://img.shields.io/badge/platform-Windows-blue)
![Formats](https://img.shields.io/badge/read-50%2B_formats-green)
![License](https://img.shields.io/badge/license-Apache_2.0-lightgrey)

50 種類以上の書庫を開き、**7Z · ZIP · TAR** で圧縮する Windows 用アーカイバ。
高速な ZIP 処理とプレビュー、エクスプローラーの右クリックメニューに対応しています。

## 機能

* ZIP、RAR、7Z、EGG、ALZ、ISO など **50 種類以上の形式を開く**
* **7Z · ZIP · TAR** で圧縮
* 画像・テキストを展開せずにその場でプレビュー
* 必要なファイルだけ選んで展開
* 書庫の中の書庫をそのまま開く
* パスワード付き圧縮と分割圧縮
* コマンドライン圧縮（Total Commander などの外部呼び出し）
* 書庫へのファイル追加・削除と整合性チェック
* Windows セキュリティスキャン（AMSI）
* エクスプローラーの右クリックメニューとファイルの関連付け
* ダーク/ライトテーマ
* **9 言語対応**

## 高速な ZIP 処理

ZIP 専用エンジンで高速に処理します。

| 処理                |    7z.dll 比 |
| ----------------- | ----------: |
| 大きいファイルの圧縮        | **最大 7.6×** |
| 小さいファイル多数の圧縮      | **最大 6.5×** |
| 展開                | **最大 2.2×** |

## 対応形式

**開く:** 7Z, ZIP, ZIPX, JAR, RAR, EGG, ALZ, TAR, GZ, BZ2, XZ, ZST, ISO, IMG, WIM, DMG, MSI, RPM, DEB, CBZ, CBR など 50 種類以上

**圧縮:** `7z` · `zip` · `tar`

## コマンドライン

ウィンドウを開かずに圧縮だけを行うコンソールコマンドです（Total Commander などの外部呼び出し用）。オプションは Bandizip と同じです。

```
ZipMania.exe c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"
```

| オプション | 意味 |
| --- | --- |
| `-l:0..9` | 圧縮レベル |
| `-fmt:zip\|7z\|tar` | 形式（省略時は出力の拡張子） |
| `-v:700M` | 分割サイズ（K/M/G） |
| `-p:パスワード` | パスワード |
| `-t:N` | スレッド数（7z） |
| `-aou` | 同名があれば `名前 (2)` に |
| `-aos` | 同名があればスキップ |
| `-testdst` | 圧縮後に整合性チェック |
| `-delsrc` | チェックに合格したら元ファイルを削除 |
| `-date` | ファイル名の `%Y %y %m %d %H %M %S` を現在時刻に置換 |

終了コードは 0（成功）/ 1（警告: 欠落項目、チェック失敗、スキップ）/ 2（エラー）です。オプション一覧は `ZipMania.exe --help` で表示できます。

## 動作環境

* Windows 10 以降
* Microsoft WebView2 Runtime

## 作者

**Kilho.net** · [https://v2.kilho.net/zipmania](https://v2.kilho.net/zipmania)

生まれて初めてのバイブコーディングで作ったプログラムです。ご意見・ご提案はいつでも歓迎します。

## ライセンス

プログラム自体は**フリーウェア**です。会社、家庭、官公庁、学校を問わず制限なく
無料で使用でき、広告やバンドルインストールはありません。

ソースコードは **Apache License 2.0** です(`LICENSE`)。`crates/` の再利用
クレートは **MIT または Apache-2.0** のデュアルライセンスです。「ZipMania」の
名称、ロゴ、アイコンは Kilho.net の商標であり、このライセンスには含まれません —
改変版を配布する場合は別の名称とアイコンを使用してください。

7-Zip の `7z.dll`(LGPL)をはじめとするオープンソース構成要素は
`THIRD-PARTY-NOTICES.txt` にまとめてあります。
