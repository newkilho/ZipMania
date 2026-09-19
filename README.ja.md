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
* コマンドライン圧縮・解凍（ウィンドウまたはコンソール、Total Commander などの外部呼び出し）
* 圧縮後の整合性検査・元ファイル削除
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

圧縮・解凍・一覧・検査をコマンドラインから行う機能です。Bandizip と 7-Zip のオプション表記を両方受け付け、実行ファイルは 2 つあります — Bandizip の `Bandizip.exe`/`bc.exe`、7-Zip の `7zG.exe`/`7z.exe` と同じ分け方です。

* **`ZipMania.exe <コマンド> …`** — 進行を**ウィンドウ**で表示します。Total Commander やショートカットなど、コンソールのない場所向けです。圧縮・解凍は進行ウィンドウ、一覧・検査はメインウィンドウにそのアーカイブが開きます。
* **`zm.exe <コマンド> …`** — **コンソール**に出力します。cmd やスクリプトは終了まで待ち、終了コード(0/1/2)を受け取れます。`zm` にコマンド以外を渡すとエラーです。

`ZipMania.exe <ファイル>` はそのファイルをウィンドウで開きます。

### 使い方

```
<exe> a|c [オプション] <アーカイブ> <入力...>    圧縮 / 追加
<exe> x|e [オプション] <アーカイブ> [項目...]    解凍(x はパス保持、e はフラット)
<exe> bx  [オプション] <アーカイブ...>           各アーカイブをその名前のフォルダーに解凍
<exe> l   [オプション] <アーカイブ>              一覧
<exe> t   [オプション] <アーカイブ>              整合性検査
```

`<exe>` は `ZipMania.exe`(進行ウィンドウ)または `zm`(コンソール)で、パラメーターは両方同じです。オプションは `-名前:値`(Bandizip)と `-名前値`(7-Zip)の両方が使えます。アーカイブの後の項目は、圧縮では入力ファイル・フォルダー、解凍では取り出す項目(省略 = すべて)です。`zm` だけを実行するとヘルプが表示されます。

### 例

```
ZipMania.exe c -l:9 -fmt:7z -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"        # 進行ウィンドウで、検査後に元を削除
zm c -l:9 -fmt:7z -v:4GB -aou -testdst -delsrc -date "backup_%y%m%d_%H%M.7z" "D:\Work"   # 同じことをコンソールで、分割 7z
zm a -mx9 -psecret backup.7z D:\Work                                              # 7-Zip 表記、既存アーカイブに追加
zm x -o:D:\Out -target:auto backup.7z                                             # アーカイブ名のフォルダーに解凍
zm e -y backup.zip docs\readme.txt                                                  # 1 項目のみ、パスなし、上書き
zm bx a.zip b.7z                                                                       # それぞれ a\、b\ フォルダーに解凍
zm l backup.7z.001                                                                     # 分割の先頭ボリュームを一覧
```

| 動詞 | 意味 |
| --- | --- |
| `a` / `c` | 圧縮(`a` は既存のアーカイブがあれば追加) |
| `x` / `e` | 解凍(`e` はフォルダー構造なし) |
| `bx` | 複数のアーカイブをそれぞれ自分の名前のフォルダーに解凍(Bandizip の `bx`、既定 `-target:name`) |
| `l` / `t` | 一覧 / 整合性検査 |

| オプション | 意味 |
| --- | --- |
| `-l:0..9` `-mx9` | 圧縮レベル |
| `-fmt:zip\|7z\|tar` `-t7z` | 形式(省略時はアーカイブの拡張子) |
| `-v:700M` `-v700m` | 分割サイズ(K/M/G) |
| `-p:パスワード` `-pパスワード` | パスワード |
| `-t:N` `-mmt=N` | スレッド数(7z) |
| `-o:フォルダー` `-oフォルダー` | 解凍先フォルダー(既定は現在のフォルダー) |
| `-target:auto\|name\|none` | アーカイブ名のサブフォルダーに解凍(`auto` = 最上位項目が 2 つ以上のときだけ) |
| `-aoa` `-y` / `-aos` / `-aou` | 同名があれば上書き / スキップ / `名前 (2)` に(圧縮の既定は上書き、解凍の既定はスキップ) |
| `-testdst` | 圧縮後に整合性検査 |
| `-delsrc` `-sdel` | 検査に合格したら元ファイルを削除 |
| `-date` | ファイル名の `%Y %y %m %d %H %M %S` を現在時刻に |
| `-r` `-bd` `-bb*` `-bs*` `-cp:*` | 無視(7-Zip / Bandizip 互換用) |

`ZipMania.exe` でウィンドウを開く場合、`-t:<n>`(スレッド)は無視され、解凍時の上書き(`-aoa`/`-aos`/`-aou`)とパスワードはウィンドウが尋ねます。既存アーカイブへの追加(`a`)は `zm` のみです。

終了コードは 0(成功)/ 1(警告: 欠落項目、検査失敗、スキップ)/ 2(エラー)です。`.bat` ファイル内では `%` を `%%` と書きます(cmd プロンプト・PowerShell・Total Commander ではそのまま)。

Total Commander のユーザーコマンド例 — コマンド `ZipMania.exe`(進行ウィンドウ)または `zm.exe`(コンソール)、パラメーター `c -l:9 -fmt:7z -aou -testdst -delsrc -date "%T%S %y%m%d_%H%M".7z "%P%S"`(選択項目を反対側パネルのフォルダーに日付名の 7z として圧縮)。

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
