//! Log simples: %APPDATA%\Roleta\roleta.log (Windows) ou
//! ~/.local/state/roleta/roleta.log (Linux, também vai para o stderr).

use std::io::Write;
use std::path::PathBuf;

#[cfg(windows)]
fn caminho() -> PathBuf {
    crate::config::pasta_dados().join("roleta.log")
}

#[cfg(not(windows))]
fn caminho() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::config::pasta_usuario().join(".local/state"));
    let p = base.join("roleta");
    let _ = std::fs::create_dir_all(&p);
    p.join("roleta.log")
}

#[cfg(windows)]
fn agora() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: SYSTEMTIME é POD (zero é válido) e GetLocalTime só escreve nela.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
fn agora() -> String {
    // SAFETY: time aceita ponteiro nulo; tm é POD (zero é válido) e o
    // localtime_r só escreve nela.
    let t = unsafe {
        let agora = libc::time(std::ptr::null_mut());
        let mut t: libc::tm = std::mem::zeroed();
        libc::localtime_r(&agora, &mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.tm_year + 1900,
        t.tm_mon + 1,
        t.tm_mday,
        t.tm_hour,
        t.tm_min,
        t.tm_sec
    )
}

pub fn escrever(msg: &str) {
    let linha = format!("{} {msg}", agora());
    #[cfg(not(windows))]
    eprintln!("{linha}");
    // os testes não mexem no log de verdade
    if cfg!(test) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(caminho()) {
        let _ = writeln!(f, "{linha}");
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
