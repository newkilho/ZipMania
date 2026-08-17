// 생성물 — 손으로 고치지 말 것, 정본은 zipmania-i18n 의 strings.rs
// 갱신 = cargo run -p zipmania-i18n --bin gen-strings
#pragma once

// 메뉴 문구 한 벌, Pre/Post 는 이름을 사이에 끼우는 앞뒤 조각
struct MenuText
{
    const wchar_t* compressZipPre;
    const wchar_t* compressZipPost;
    const wchar_t* compress;
    const wchar_t* compressEach;
    const wchar_t* extractHere;
    const wchar_t* extractToPre;
    const wchar_t* extractToPost;
    const wchar_t* extract;
    const wchar_t* open;
    const wchar_t* extractEach;
};

// 언어별 메뉴 문구, 코드 순서는 LANGS 와 같다
static const struct
{
    const wchar_t* code;
    MenuText text;
} kMenuTexts[] = {
    {L"ko",
     {L"\"",
      L".zip\"(으)로 압축하기",
      L"집매니아로 압축하기",
      L"각각 압축하기",
      L"여기에 풀기",
      L"\"",
      L"\" 에 풀기",
      L"집매니아로 압축 풀기",
      L"집매니아로 열기",
      L"각각 파일명 폴더에 풀기"}},
    {L"en",
     {L"Compress to ",
      L".zip",
      L"Compress with ZipMania",
      L"Compress each separately",
      L"Extract Here",
      L"Extract to \"",
      L"\"",
      L"Extract with ZipMania",
      L"Open with ZipMania",
      L"Extract each to own folder"}},
    {L"ja",
     {L"「",
      L".zip」に圧縮",
      L"ZipMania で圧縮",
      L"それぞれ個別に圧縮",
      L"ここに展開",
      L"「",
      L"」に展開",
      L"ZipMania で展開",
      L"ZipMania で開く",
      L"それぞれ同名フォルダーに展開"}},
    {L"zh",
     {L"压缩到 “",
      L".zip”",
      L"使用 ZipMania 压缩",
      L"分别单独压缩",
      L"解压到当前文件夹",
      L"解压到 “",
      L"”",
      L"使用 ZipMania 解压",
      L"使用 ZipMania 打开",
      L"分别解压到各自的同名文件夹"}},
    {L"ru",
     {L"Сжать в «",
      L".zip»",
      L"Сжать с помощью ZipMania",
      L"Сжать каждый отдельно",
      L"Извлечь здесь",
      L"Извлечь в «",
      L"»",
      L"Извлечь с помощью ZipMania",
      L"Открыть с помощью ZipMania",
      L"Извлечь каждый в свою папку"}},
    {L"it",
     {L"Comprimi in \"",
      L".zip\"",
      L"Comprimi con ZipMania",
      L"Comprimi ciascuno separatamente",
      L"Estrai qui",
      L"Estrai in \"",
      L"\"",
      L"Estrai con ZipMania",
      L"Apri con ZipMania",
      L"Estrai ciascuno nella propria cartella"}},
    {L"fr",
     {L"Compresser en « ",
      L".zip »",
      L"Compresser avec ZipMania",
      L"Compresser chacun séparément",
      L"Extraire ici",
      L"Extraire dans « ",
      L" »",
      L"Extraire avec ZipMania",
      L"Ouvrir avec ZipMania",
      L"Extraire chacun dans son dossier"}},
    {L"es",
     {L"Comprimir en \"",
      L".zip\"",
      L"Comprimir con ZipMania",
      L"Comprimir cada uno por separado",
      L"Extraer aquí",
      L"Extraer en \"",
      L"\"",
      L"Extraer con ZipMania",
      L"Abrir con ZipMania",
      L"Extraer cada uno en su carpeta"}},
    {L"ar",
     {L"ضغط إلى «",
      L".zip»",
      L"ضغط باستخدام ZipMania",
      L"ضغط كل عنصر على حدة",
      L"استخراج هنا",
      L"استخراج إلى «",
      L"»",
      L"استخراج باستخدام ZipMania",
      L"فتح باستخدام ZipMania",
      L"استخراج كل عنصر إلى مجلد باسمه"}},
};
