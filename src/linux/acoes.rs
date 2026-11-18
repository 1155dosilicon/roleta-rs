//! O que cada opção faz no Linux: abrir (xdg-open ou o programa), comando
//! (sh -c), teclas e texto (teclado falso do XTest).

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Keysym, KEY_PRESS_EVENT, KEY_RELEASE_EVENT};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use crate::config::{pasta_usuario, Opcao};
use crate::log;

// ------------------------------------------------------------ programas

/// Roda solto da roleta (grupo de processos próprio), na pasta do usuário.
/// Uma thread espera o fim (sem processo zumbi) e registra no log se falhar.
fn lancar(mut c: Command, desc: String) {
    c.current_dir(pasta_usuario())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut filho = match c.spawn() {
        Ok(f) => f,
        Err(e) => {
            log::escrever(&format!("Erro ao rodar {desc:?}: {e}"));
            return;
        }
    };
    std::thread::spawn(move || {
        // lê o stderr até o fim (senão o programa trava com o cano cheio),
        // guardando só o final
        let mut fim: Vec<u8> = Vec::new();
        if let Some(mut err) = filho.stderr.take() {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                fim.extend_from_slice(&buf[..n]);
                if fim.len() > 2000 {
                    fim.drain(..fim.len() - 2000);
                }
            }
        }
        match filho.wait() {
            Ok(s) if !s.success() => {
                let msg = String::from_utf8_lossy(&fim);
                log::escrever(&format!("{desc:?} terminou com {s}: {}", msg.trim()));
            }
            Err(e) => log::escrever(&format!("{desc:?}: {e}")),
            _ => {}
        }
    });
}

fn expandir(s: &str) -> String {
    match s.strip_prefix('~') {
        Some(r) if r.is_empty() || r.starts_with('/') => format!("{}{r}", pasta_usuario().display()),
        _ => s.to_string(),
    }
}

/// "https://...", "mailto:...", "file:///..."
fn eh_uri(s: &str) -> bool {
    match s.split_once(':') {
        Some((esq, _)) => {
            esq.len() >= 2
                && esq.starts_with(|c: char| c.is_ascii_alphabetic())
                && esq.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        }
        None => false,
    }
}

fn no_path(nome: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if nome.is_empty() || nome.contains('/') || nome.contains(char::is_whitespace) {
        return false;
    }
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            std::fs::metadata(d.join(nome)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// Abre arquivo, pasta, site ou URI com o programa padrão (xdg-open), ou roda
/// o programa com esse nome se estiver no PATH (ex.: "google-chrome").
pub fn abrir(alvo: &str) {
    let alvo = expandir(alvo.trim());
    if !eh_uri(&alvo) && !Path::new(&alvo).exists() && no_path(&alvo) {
        lancar(Command::new(&alvo), alvo.clone());
        return;
    }
    let mut c = Command::new("xdg-open");
    c.arg(&alvo);
    lancar(c, format!("xdg-open {alvo}"));
}

/// Roda uma linha de comando no sh (aceita ~, &&, aspas...).
pub fn comando(linha: &str) {
    let mut c = Command::new("sh");
    c.arg("-c").arg(linha);
    lancar(c, linha.to_string());
}

// ------------------------------------------------------------ teclado

/// Nome de tecla no formato do pynput ("ctrl", "shift", "f5", "a"...) -> keysym do X.
fn keysym_tecla(nome: &str) -> Option<Keysym> {
    let n = nome.trim().trim_start_matches('<').trim_end_matches('>').to_lowercase();
    let ks = match n.as_str() {
        "ctrl" | "ctrl_l" | "control" => 0xffe3,
        "ctrl_r" => 0xffe4,
        "shift" | "shift_l" => 0xffe1,
        "shift_r" => 0xffe2,
        "alt" | "alt_l" => 0xffe9,
        "alt_r" | "alt_gr" => 0xfe03, // ISO_Level3_Shift
        "cmd" | "cmd_l" | "win" | "super" => 0xffeb,
        "cmd_r" => 0xffec,
        "enter" | "return" => 0xff0d,
        "esc" | "escape" => 0xff1b,
        "tab" => 0xff09,
        "space" => 0x20,
        "backspace" => 0xff08,
        "delete" | "del" => 0xffff,
        "insert" => 0xff63,
        "home" => 0xff50,
        "end" => 0xff57,
        "page_up" => 0xff55,
        "page_down" => 0xff56,
        "up" => 0xff52,
        "down" => 0xff54,
        "left" => 0xff51,
        "right" => 0xff53,
        "print_screen" => 0xff61,
        "caps_lock" => 0xffe5,
        "num_lock" => 0xff7f,
        "menu" => 0xff67,
        "pause" => 0xff13,
        "media_play_pause" => 0x1008ff14,
        "media_next" => 0x1008ff17,
        "media_previous" => 0x1008ff16,
        "media_volume_up" => 0x1008ff13,
        "media_volume_down" => 0x1008ff11,
        "media_volume_mute" => 0x1008ff12,
        _ => {
            if let Some(f) = n.strip_prefix('f').and_then(|x| x.parse::<u32>().ok())
                && (1..=24).contains(&f) {
                    return Some(0xffbe + f - 1);
                }
            let mut cs = n.chars();
            let c = cs.next()?;
            if cs.next().is_some() {
                return None;
            }
            return Some(keysym_char(c));
        }
    };
    Some(ks)
}

fn keysym_char(c: char) -> Keysym {
    match c {
        '\n' => 0xff0d,
        '\t' => 0xff09,
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => c as u32,
        _ => 0x0100_0000 | c as u32,
    }
}

const SHIFT: Keysym = 0xffe1;

/// Teclado falso: acha a tecla (keycode) de cada keysym no mapa atual e,
/// se não houver, empresta temporariamente uma tecla sem uso.
struct Teclado<'a> {
    conn: &'a RustConnection,
    min: u8,
    por_tecla: usize,
    syms: Vec<Keysym>,
    emprestadas: Vec<Keycode>,
}

impl<'a> Teclado<'a> {
    fn new(conn: &'a RustConnection) -> Result<Self, String> {
        let s = conn.setup();
        let (min, max) = (s.min_keycode, s.max_keycode);
        let r = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        Ok(Teclado { conn, min, por_tecla: r.keysyms_per_keycode as usize, syms: r.keysyms, emprestadas: Vec::new() })
    }

    /// (tecla, precisa de shift)
    fn achar(&self, ks: Keysym) -> Option<(Keycode, bool)> {
        for nivel in 0..self.por_tecla.min(2) {
            for (i, t) in self.syms.chunks(self.por_tecla).enumerate() {
                if t[nivel] == ks {
                    return Some((self.min + i as u8, nivel == 1));
                }
            }
        }
        None
    }

    fn emprestar(&mut self, ks: Keysym) -> Option<Keycode> {
        // de trás para frente: as últimas teclas quase nunca existem no teclado
        let i = self.syms.chunks(self.por_tecla).rposition(|t| t.iter().all(|&s| s == 0))?;
        let kc = self.min + i as u8;
        let novo = vec![ks; self.por_tecla];
        self.conn.change_keyboard_mapping(1, kc, self.por_tecla as u8, &novo).ok()?;
        self.syms[i * self.por_tecla..(i + 1) * self.por_tecla].copy_from_slice(&novo);
        self.emprestadas.push(kc);
        // dá tempo dos programas lerem o mapa novo
        let _ = self.conn.sync();
        std::thread::sleep(Duration::from_millis(20));
        Some(kc)
    }

    fn tecla(&mut self, ks: Keysym) -> Option<(Keycode, bool)> {
        self.achar(ks).or_else(|| self.emprestar(ks).map(|kc| (kc, false)))
    }

    fn enviar(&self, kc: Keycode, aperta: bool) {
        let tipo = if aperta { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT };
        let _ = self.conn.xtest_fake_input(tipo, kc, CURRENT_TIME, NONE, 0, 0, 0);
    }

    fn devolver(&mut self) {
        if self.emprestadas.is_empty() {
            return;
        }
        let _ = self.conn.sync();
        std::thread::sleep(Duration::from_millis(20));
        let vazio = vec![0; self.por_tecla];
        for kc in self.emprestadas.drain(..) {
            let _ = self.conn.change_keyboard_mapping(1, kc, self.por_tecla as u8, &vazio);
            let i = (kc - self.min) as usize;
            self.syms[i * self.por_tecla..(i + 1) * self.por_tecla].copy_from_slice(&vazio);
        }
        let _ = self.conn.sync();
    }
}

fn atalho(conn: &RustConnection, valor: &str) -> Result<(), String> {
    // "<ctrl>+<shift>+s" (formato do pynput). "+" sozinho no fim = tecla "+".
    let mut partes: Vec<&str> = valor.split('+').collect();
    if valor.ends_with("++") {
        partes.truncate(partes.len().saturating_sub(2));
        partes.push("+");
    }
    let mut syms = Vec::new();
    for p in partes.iter().filter(|p| !p.trim().is_empty()) {
        syms.push(keysym_tecla(p).ok_or_else(|| format!("tecla desconhecida: {p:?}"))?);
    }
    let mut tec = Teclado::new(conn)?;
    let mut teclas = Vec::new();
    let mut shift = false;
    for &ks in &syms {
        let (kc, s) = tec.tecla(ks).ok_or_else(|| format!("sem tecla livre para o keysym {ks:#x}"))?;
        shift |= s;
        teclas.push(kc);
    }
    // ex.: "+" no teclado americano é shift + "="
    if shift && !syms.contains(&SHIFT)
        && let Some((kc, _)) = tec.achar(SHIFT) {
            teclas.insert(0, kc);
        }
    for &kc in &teclas {
        tec.enviar(kc, true);
    }
    for &kc in teclas.iter().rev() {
        tec.enviar(kc, false);
    }
    tec.devolver();
    Ok(())
}

fn digitar(conn: &RustConnection, texto: &str) -> Result<(), String> {
    let mut tec = Teclado::new(conn)?;
    let shift = tec.achar(SHIFT).map(|(kc, _)| kc);
    for c in texto.chars() {
        let Some((kc, s)) = tec.tecla(keysym_char(c)) else {
            log::escrever(&format!("Sem tecla livre para digitar {c:?}"));
            continue;
        };
        let sh = shift.filter(|_| s);
        if let Some(sh) = sh {
            tec.enviar(sh, true);
        }
        tec.enviar(kc, true);
        tec.enviar(kc, false);
        if let Some(sh) = sh {
            tec.enviar(sh, false);
        }
        // devolve a tecla emprestada antes de emprestar outra
        tec.devolver();
    }
    let _ = conn.flush();
    Ok(())
}

/// Executa a opção. Devolve `true` se a opção pede para sair da roleta.
pub fn executar(op: &Opcao, conn: &RustConnection) -> bool {
    let r = match op.tipo.as_str() {
        "abrir" => {
            abrir(&op.valor);
            Ok(())
        }
        "comando" => {
            comando(&op.valor);
            Ok(())
        }
        "teclas" => atalho(conn, &op.valor),
        "texto" => digitar(conn, &op.valor),
        "sair" => return true,
        t => Err(format!("tipo desconhecido: {t:?}")),
    };
    let _ = conn.flush();
    if let Err(e) = r {
        log::escrever(&format!("Erro ao executar {:?}: {e}", op.nome));
    }
    false
}
