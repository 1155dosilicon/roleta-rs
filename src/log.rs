//! Log simples em %APPDATA%\Roleta\roleta.log.

use std::io::Write;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

fn caminho() -> std::path::PathBuf {
    crate::config::pasta_dados().join("roleta.log")
}

fn agora() -> String {
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

pub fn escrever(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(caminho()) {
        let _ = writeln!(f, "{} {msg}", agora());
    }
}

pub fn inicio() {
    let p = caminho();
    // não deixa o log crescer para sempre
    if std::fs::metadata(&p).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    escrever("--- roleta iniciada (Rust) ---");
}
