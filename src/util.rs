use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(unix: u64) -> String {
    match now_unix().saturating_sub(unix) {
        0..=59 => "just now".to_owned(),
        d @ 60..=3599 => format!("{} min ago", d / 60),
        d @ 3600..=86399 => format!("{} h ago", d / 3600),
        d => format!("{} days ago", d / 86400),
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
