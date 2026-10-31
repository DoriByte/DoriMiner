//! What the miner shows in its own console window: the banner, the header, the
//! log lines with colour and local time, the report every 30 s and the window
//! title. Only when the output is a console a person reads: under a manager
//! (output piped), with NO_COLOR set or TERM=dumb, every line stays the plain
//! "[time LEVEL] message" it always was. Managers read the miner by its API.

use crate::sensors::Reading;
use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

/// Log target for the lines that matter to the person mining (shown in green).
pub const GOOD: &str = "good";
/// Log target for the technical lines (shown dimmed).
pub const DETAIL: &str = "detail";
/// Log target for each new job: a pool may send one every 2 s, so a console leaves them out.
pub const JOB: &str = "job";

/// Core and memory temperatures from which the report warns.
const CORE_WARN_C: u32 = 80;
const CORE_HOT_C: u32 = 87;
const MEMORY_WARN_C: u32 = 90;
const MEMORY_HOT_C: u32 = 100;

pub fn fancy() -> bool {
    static FANCY: OnceLock<bool> = OnceLock::new();
    *FANCY.get_or_init(|| {
        std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map_or(true, |t| t != "dumb")
            && enable_colours()
    })
}

/// Windows consoles show escape sequences only once asked to.
#[cfg(windows)]
fn enable_colours() -> bool {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(handle: *mut std::ffi::c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut std::ffi::c_void, mode: u32) -> i32;
    }
    // SAFETY: plain calls of the console API on this process's own output.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        GetConsoleMode(handle, &mut mode) != 0 && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}
#[cfg(not(windows))]
fn enable_colours() -> bool {
    true
}

fn paint(code: &str, text: &str) -> String {
    if fancy() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}
pub fn green(text: &str) -> String { paint("32", text) }
pub fn yellow(text: &str) -> String { paint("33", text) }
pub fn red(text: &str) -> String { paint("1;31", text) }
pub fn dim(text: &str) -> String { paint("2", text) }
pub fn bold(text: &str) -> String { paint("1", text) }
pub fn cyan(text: &str) -> String { paint("36", text) }

/// Local time as HH:MM:SS.
fn local_time() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct SystemTime { year: u16, month: u16, weekday: u16, day: u16, hour: u16, minute: u16, second: u16, millis: u16 }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(time: *mut SystemTime);
        }
        let mut t = SystemTime { year: 0, month: 0, weekday: 0, day: 0, hour: 0, minute: 0, second: 0, millis: 0 };
        // SAFETY: fills the struct it is given.
        unsafe { GetLocalTime(&mut t) };
        format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
    }
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm { sec: i32, min: i32, hour: i32, mday: i32, mon: i32, year: i32, wday: i32, yday: i32, isdst: i32, gmtoff: i64, zone: *const std::ffi::c_char }
        extern "C" {
            fn time(t: *mut i64) -> i64;
            fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
        }
        let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
        // SAFETY: the C library's own time functions, with valid pointers.
        unsafe {
            let now = time(std::ptr::null_mut());
            localtime_r(&now, &mut tm);
        }
        format!("{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec)
    }
}

/// One log line. Plain: the format the miner always had. In a console: local
/// time, and the message coloured by what it is.
pub fn log_line(out: &mut impl Write, timestamp: impl std::fmt::Display, level: log::Level, target: &str, message: &str) -> std::io::Result<()> {
    if !fancy() {
        return writeln!(out, "[{} {:<5}] {}", timestamp, level, message);
    }
    if target == JOB && level > log::Level::Warn {
        return Ok(());
    }
    let text = match (level, target) {
        (log::Level::Error, _) => red(&format!("✗ {message}")),
        (log::Level::Warn, _) => yellow(&format!("⚠ {message}")),
        (_, GOOD) => green(&format!("✓ {message}")),
        (_, DETAIL) => dim(message),
        // the engine's own lines (kernel loaded, registers): technical
        (_, t) if t.starts_with("qtc_cuda") => dim(message),
        _ => message.to_string(),
    };
    writeln!(out, "{}  {}", dim(&local_time()), text)
}

const BANNER: [&str; 6] = [
    "██████╗  ██████╗ ██████╗ ██╗███╗   ███╗██╗███╗   ██╗███████╗██████╗ ",
    "██╔══██╗██╔═══██╗██╔══██╗██║████╗ ████║██║████╗  ██║██╔════╝██╔══██╗",
    "██║  ██║██║   ██║██████╔╝██║██╔████╔██║██║██╔██╗ ██║█████╗  ██████╔╝",
    "██║  ██║██║   ██║██╔══██╗██║██║╚██╔╝██║██║██║╚██╗██║██╔══╝  ██╔══██╗",
    "██████╔╝╚██████╔╝██║  ██║██║██║ ╚═╝ ██║██║██║ ╚████║███████╗██║  ██║",
    "╚═════╝  ╚═════╝ ╚═╝  ╚═╝╚═╝╚═╝     ╚═╝╚═╝╚═╝  ╚═══╝╚══════╝╚═╝  ╚═╝",
];

/// The wallet as the header shows it: the start and the end, enough to recognise it.
pub fn short_wallet(wallet: &str) -> String {
    let chars: Vec<char> = wallet.chars().collect();
    if chars.len() <= 16 {
        return wallet.to_string();
    }
    format!("{}…{}", chars[..8].iter().collect::<String>(), chars[chars.len() - 6..].iter().collect::<String>())
}

pub struct Header<'a> {
    pub version: &'a str,
    pub coin: &'a str,
    pub pool: &'a str,
    pub wallet: &'a str,
    pub worker: Option<&'a str>,
    pub dev_fee: u64,
    pub api: Option<String>,
    pub gpus: Vec<String>,
}

/// The big name and the header, once at start. Only in a console.
pub fn print_start(h: &Header) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out);
    // a gradient from cyan to blue, one colour a line
    let colours = [(0, 214, 255), (0, 190, 255), (20, 165, 255), (40, 140, 255), (60, 115, 255), (80, 95, 240)];
    for (line, (r, g, b)) in BANNER.iter().zip(colours) {
        let _ = writeln!(out, "  \x1b[1;38;2;{r};{g};{b}m{line}\x1b[0m");
    }
    let by = format!("by DoriByte · v{}", h.version);
    let _ = writeln!(out, "  {:>width$}", cyan(&by), width = BANNER[0].chars().count() + 9);
    let _ = writeln!(out);
    let rows = {
        let mut rows = vec![("Coin", h.coin.to_string()), ("Pool", h.pool.to_string()), ("Wallet", short_wallet(h.wallet))];
        if let Some(worker) = h.worker {
            rows.push(("Worker", worker.to_string()));
        }
        rows.push(("Dev fee", match h.dev_fee {
            0 => "off".to_string(),
            p => format!("{p}% (optional: --dev-fee 0 turns it off)"),
        }));
        if let Some(api) = &h.api {
            rows.push(("API", api.clone()));
        }
        for gpu in &h.gpus {
            rows.push(("GPU", gpu.clone()));
        }
        rows
    };
    let inner = rows.iter().map(|(k, v)| k.len().max(7) + 2 + v.chars().count()).max().unwrap_or(40).max(50);
    let _ = writeln!(out, "  {}", dim(&format!("╭{}╮", "─".repeat(inner + 2))));
    for (k, v) in &rows {
        let pad = inner - k.len().max(7) - 2 - v.chars().count();
        let _ = writeln!(out, "  {} {}  {}{} {}", dim("│"), bold(&format!("{k:<7}")), v, " ".repeat(pad), dim("│"));
    }
    let _ = writeln!(out, "  {}", dim(&format!("╰{}╯", "─".repeat(inner + 2))));
    let _ = writeln!(out);
}

/// The window's title (the taskbar shows it). Only in a console.
pub fn set_title(title: &str) {
    if fancy() {
        let mut out = std::io::stdout().lock();
        // no newline follows, so the line buffer has to be flushed for the title to change now
        let _ = write!(out, "\x1b]0;{title}\x07");
        let _ = out.flush();
    }
}

fn uptime(seconds: u64) -> String {
    let (h, m) = (seconds / 3600, seconds / 60 % 60);
    if h >= 24 { format!("{}d{:02}h{:02}m", h / 24, h % 24, m) } else { format!("{h}h{m:02}m") }
}

pub struct Totals {
    pub hashrate: String,
    pub accepted: u64,
    pub rejected: u64,
    pub stale: u64,
    pub uptime_s: u64,
}

pub struct GpuLine {
    pub ordinal: usize,
    pub name: String,
    pub hashrate: String,
    pub failed: bool,
    pub sensors: Reading,
}

/// The report every 30 s, in a console: totals, then a line a card.
pub fn print_report(t: &Totals, gpus: &[GpuLine]) {
    let count = |n: u64, paint: fn(&str) -> String| if n > 0 { paint(&n.to_string()) } else { n.to_string() };
    let mut lines = vec![format!(
        "{} {} {}  accepted {} · rejected {} · stale {}  {} up {}",
        dim("──"),
        bold(&cyan(&t.hashrate)),
        dim("──"),
        count(t.accepted, green),
        count(t.rejected, red),
        count(t.stale, yellow),
        dim("──"),
        uptime(t.uptime_s)
    )];
    for g in gpus {
        let mut line = format!("          GPU {}  {:<24} {:>12}", g.ordinal, g.name, if g.failed { red("down") } else { g.hashrate.clone() });
        let s = g.sensors;
        if let Some(c) = s.core_c {
            let text = format!("{c} °C");
            line += &format!("  {}", if c >= CORE_HOT_C { red(&text) } else if c >= CORE_WARN_C { yellow(&text) } else { text });
        }
        if let Some(c) = s.memory_c {
            let text = format!("mem {c} °C");
            line += &format!("  {}", if c >= MEMORY_HOT_C { red(&text) } else if c >= MEMORY_WARN_C { yellow(&text) } else { text });
        }
        if let Some(w) = s.watts {
            line += &format!("  {w:.0} W");
        }
        if let Some(f) = s.fan_pct {
            line += &format!("  {}", dim(&format!("fan {f}%")));
        }
        if s.core_c.is_some_and(|c| c >= CORE_HOT_C) || s.memory_c.is_some_and(|c| c >= MEMORY_HOT_C) {
            line += &format!("  {}", red("TOO HOT: check the fans and the power limit"));
        }
        lines.push(line);
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}  {}", dim(&local_time()), lines[0]);
    for line in &lines[1..] {
        let _ = writeln!(out, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_wallet_keeps_both_ends() {
        assert_eq!(short_wallet("qzExampLeWa11etAddressForTheTestOnly0123456789ab"), "qzExampL…6789ab");
        assert_eq!(short_wallet("short"), "short");
    }

    #[test]
    fn uptime_reads_in_hours_and_days() {
        assert_eq!(uptime(59), "0h00m");
        assert_eq!(uptime(3_725), "1h02m");
        assert_eq!(uptime(90_000), "1d01h00m");
    }

    #[test]
    fn banner_lines_are_one_width() {
        let w = BANNER[0].chars().count();
        assert!(BANNER.iter().all(|l| l.chars().count() == w));
    }
}
