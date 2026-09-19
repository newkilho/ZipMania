//! 콘솔 명령의 출력 층, 출력 자리 판정(부모 콘솔 / 파이프 / 새 콘솔)과 진행 줄 그리기
//! 고정 줄 = line, 진행 줄 = progress(콘솔이면 \r 로 같은 자리에 다시 그림, 파이프면 10% 단위 줄)
//! 창은 어떤 경우에도 띄우지 않음 (D3.18)

use std::io::Write;
use std::time::{Duration, Instant};

use zipmania_archive::Progress;

/// 명령 실행부가 쓰는 보고 채널, 테스트는 Vec 로 받음
pub trait Report {
    fn line(&mut self, s: &str);
    fn progress(&mut self, p: &Progress);
    fn progress_end(&mut self);
}

/// 줄 수집기(테스트, 진행 줄은 버림)
#[cfg(test)]
#[derive(Default)]
pub struct Lines(pub Vec<String>);

#[cfg(test)]
impl Report for Lines {
    fn line(&mut self, s: &str) {
        self.0.push(s.to_string());
    }
    fn progress(&mut self, _p: &Progress) {}
    fn progress_end(&mut self) {}
}

/// 출력이 가는 곳, Parent = 부모 콘솔(zm 의 것), Redirected = 파이프/파일 또는 없음(그대로 흘려보냄), AllocConsole 금지 (D3.18)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Console {
    Parent,
    Redirected,
}

/// 실제 콘솔 출력, 진행 줄은 100ms 간격, 속도는 지수이동평균
pub struct Term {
    kind: Console,
    width: usize,
    last_draw: Option<Instant>,
    last_sample: Option<(Instant, u64)>,
    rate: f64,
    last_pct_line: u32,
    dirty: bool,
}

impl Term {
    pub fn open() -> Term {
        let kind = attach_console();
        Term {
            kind,
            width: console_width().unwrap_or(80).max(40),
            last_draw: None,
            last_sample: None,
            rate: 0.0,
            last_pct_line: u32::MAX,
            dirty: false,
        }
    }

    /// 종료 처리, 부모 콘솔이면 프롬프트 복원
    pub fn close(&mut self) {
        self.progress_end();
        let _ = std::io::stdout().flush();
        // zm.exe 를 거치면 그쪽이 기다리므로 프롬프트 복원 불필요
        if self.kind == Console::Parent && std::env::var_os("ZIPMANIA_CONSOLE_HOST").is_none() {
            release_console();
        }
    }

    fn clear_progress(&mut self) {
        if self.dirty {
            print!("\r{}\r", " ".repeat(self.width - 1));
            self.dirty = false;
        }
    }

    fn draw(&mut self, p: &Progress) {
        let now = Instant::now();
        if let Some((t, b)) = self.last_sample {
            let dt = now.duration_since(t).as_secs_f64();
            if dt > 0.0 && p.done >= b {
                let inst = (p.done - b) as f64 / dt;
                self.rate = if self.rate == 0.0 { inst } else { self.rate * 0.7 + inst * 0.3 };
            }
        }
        self.last_sample = Some((now, p.done));
        let pct = percent(p);
        let bar_w = 20usize;
        let filled = (pct as usize * bar_w / 100).min(bar_w);
        let bar = format!("[{}{}]", "=".repeat(filled), " ".repeat(bar_w - filled));
        let mut s = if p.total > 0 {
            let eta = if self.rate > 0.0 && p.total > p.done {
                fmt_dur(Duration::from_secs_f64((p.total - p.done) as f64 / self.rate))
            } else {
                "--:--".to_string()
            };
            format!(
                "{bar} {pct:3}%  {} / {}  {}/s  ETA {eta}",
                fmt_size(p.done),
                fmt_size(p.total),
                fmt_size(self.rate as u64)
            )
        } else {
            format!("{bar} {pct:3}%")
        };
        if let Some(f) = &p.current_file {
            let room = self.width.saturating_sub(s.chars().count() + 3);
            if room > 8 {
                s.push_str("  ");
                s.push_str(&tail(f, room));
            }
        }
        let s: String = s.chars().take(self.width - 1).collect();
        print!("\r{s:<w$}", w = self.width - 1);
        let _ = std::io::stdout().flush();
        self.dirty = true;
    }
}

impl Report for Term {
    fn line(&mut self, s: &str) {
        self.clear_progress();
        println!("{s}");
    }

    fn progress(&mut self, p: &Progress) {
        match self.kind {
            Console::Redirected => {
                let pct = percent(p);
                if pct / 10 != self.last_pct_line / 10 {
                    self.last_pct_line = pct;
                    println!("  {pct}%");
                }
            }
            _ => {
                let due = self
                    .last_draw
                    .map(|t| t.elapsed() >= Duration::from_millis(100))
                    .unwrap_or(true);
                if due || percent(p) >= 100 {
                    self.last_draw = Some(Instant::now());
                    self.draw(p);
                }
            }
        }
    }

    fn progress_end(&mut self) {
        self.clear_progress();
        self.last_draw = None;
        self.last_sample = None;
        self.rate = 0.0;
        self.last_pct_line = u32::MAX;
    }
}

/// 백분율, 바이트가 있으면 바이트로, 없으면 엔진 값
pub fn percent(p: &Progress) -> u32 {
    if p.total > 0 {
        ((p.done.min(p.total) * 100) / p.total) as u32
    } else {
        p.percent as u32
    }
}

/// 3,149,962 형식
pub fn fmt_bytes(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 3.0 MB 형식(1024 단위)
pub fn fmt_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// 0.4 s / 1:05 / 1:02:03
pub fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{:.1} s", d.as_secs_f64())
    } else if s < 3600 {
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    }
}

/// 긴 경로의 뒷부분만, 앞에 …
fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        let skip = n - max + 1;
        format!("…{}", s.chars().skip(skip).collect::<String>())
    }
}

/// 부모 콘솔(zm 이 물려준 것) → 그 외는 Redirected, 코드 페이지 UTF-8(cp949 콘솔 깨짐 방지)
/// 콘솔이 없어도 만들지 않음, ZipMania.exe 직접 실행은 창으로 가므로 여기 오지 않음
#[cfg(windows)]
fn attach_console() -> Console {
    use windows::Win32::System::Console::{AttachConsole, SetConsoleOutputCP, ATTACH_PARENT_PROCESS};
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            let _ = SetConsoleOutputCP(65001);
            return Console::Parent;
        }
    }
    Console::Redirected
}

/// 콘솔 열 수, 콘솔이 아니면 None
#[cfg(windows)]
fn console_width() -> Option<usize> {
    use windows::Win32::System::Console::{
        GetConsoleScreenBufferInfo, GetStdHandle, CONSOLE_SCREEN_BUFFER_INFO, STD_OUTPUT_HANDLE,
    };
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        GetConsoleScreenBufferInfo(h, &mut info).ok()?;
        (info.dwSize.X > 0).then_some(info.dwSize.X as usize)
    }
}

/// 콘솔 입력에 Enter 1회, cmd 는 GUI exe 를 기다리지 않아 프롬프트가 출력 위에 먼저 찍힘
/// → 끝에 Enter 를 넣어 프롬프트를 다시 그리게 함(멈춘 것처럼 보이는 것 방지)
#[cfg(windows)]
fn release_console() {
    use windows::Win32::System::Console::{
        GetStdHandle, WriteConsoleInputW, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT,
        KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, STD_INPUT_HANDLE,
    };
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        let Ok(h) = GetStdHandle(STD_INPUT_HANDLE) else { return };
        let key = |down: bool| INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: down.into(),
                    wRepeatCount: 1,
                    wVirtualKeyCode: 0x0D,
                    wVirtualScanCode: 0x1C,
                    uChar: KEY_EVENT_RECORD_0 { UnicodeChar: 0x0D },
                    dwControlKeyState: 0,
                },
            },
        };
        let recs = [key(true), key(false)];
        let mut n = 0u32;
        let _ = WriteConsoleInputW(h, &recs, &mut n);
    }
}

#[cfg(not(windows))]
fn attach_console() -> Console {
    Console::Redirected
}

#[cfg(not(windows))]
fn console_width() -> Option<usize> {
    None
}

#[cfg(not(windows))]
fn release_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 크기_시간_표기() {
        assert_eq!(fmt_bytes(0), "0");
        assert_eq!(fmt_bytes(999), "999");
        assert_eq!(fmt_bytes(3149962), "3,149,962");
        assert_eq!(fmt_size(16), "16 B");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(3 << 20), "3.0 MB");
        assert_eq!(fmt_dur(Duration::from_millis(400)), "0.4 s");
        assert_eq!(fmt_dur(Duration::from_secs(65)), "1:05");
        assert_eq!(fmt_dur(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(tail("abcdefgh", 5), "…efgh");
        assert_eq!(tail("abc", 5), "abc");
    }
}
