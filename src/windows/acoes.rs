//! O que cada opção faz: abrir, comando, teclas, texto.
//!
//! Quando a roleta roda como administrador, tudo que ela abrir herdaria isso
//! (Chrome como admin, terminal como admin...). Para evitar, o processo novo é
//! criado com o Explorer (que roda como usuário normal) como "pai", herdando o
//! acesso dele em vez do nosso.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, OpenProcess,
    UpdateProcThreadAttribute, CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATE_PROCESS, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, STARTUPINFOEXW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId, SW_SHOWNORMAL};

use crate::config::{pasta_usuario, Opcao};
use crate::log;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn eh_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

fn cmd_exe() -> PathBuf {
    let raiz = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(raiz).join("System32").join("cmd.exe")
}

fn erro_win(onde: &str) -> String {
    format!("{onde} falhou (erro {})", unsafe { GetLastError() })
}

fn rodar_como_usuario(linha: &str) -> Result<(), String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetShellWindow(), &mut pid);
        if pid == 0 {
            return Err("Explorer não encontrado".into());
        }
        let hpai = OpenProcess(PROCESS_CREATE_PROCESS, 0, pid);
        if hpai.is_null() {
            return Err(erro_win("OpenProcess"));
        }
        let mut tam = 0usize;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut tam);
        let mut buf = vec![0u8; tam];
        let lista = buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        let r = (|| {
            if InitializeProcThreadAttributeList(lista, 1, 0, &mut tam) == 0 {
                return Err(erro_win("InitializeProcThreadAttributeList"));
            }
            let mut hp: HANDLE = hpai;
            let ok = UpdateProcThreadAttribute(
                lista,
                0,
                PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
                &mut hp as *mut HANDLE as *const _,
                std::mem::size_of::<HANDLE>(),
                null_mut(),
                null(),
            );
            if ok == 0 {
                DeleteProcThreadAttributeList(lista);
                return Err(erro_win("UpdateProcThreadAttribute"));
            }
            let mut si: STARTUPINFOEXW = std::mem::zeroed();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            si.lpAttributeList = lista;
            let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
            let mut cmd = wide(linha);
            let dir = wide(&pasta_usuario().to_string_lossy());
            let ok = CreateProcessW(
                null(),
                cmd.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                null(),
                dir.as_ptr(),
                &si.StartupInfo,
                &mut pi,
            );
            DeleteProcThreadAttributeList(lista);
            if ok == 0 {
                return Err(erro_win("CreateProcessW"));
            }
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            Ok(())
        })();
        CloseHandle(hpai);
        r
    }
}

/// Abre arquivo/pasta/site/programa sem herdar o modo administrador.
pub fn abrir_como_usuario(alvo: &str) {
    if eh_admin() {
        let linha = format!("\"{}\" /c start \"\" \"{alvo}\"", cmd_exe().display());
        match rodar_como_usuario(&linha) {
            Ok(()) => return,
            Err(e) => log::escrever(&format!("Não deu para abrir como usuário ({e}); abrindo direto.")),
        }
    }
    let op = wide("open");
    let arq = wide(alvo);
    let r = unsafe { ShellExecuteW(null_mut(), op.as_ptr(), arq.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    if (r as isize) <= 32 {
        log::escrever(&format!("Não deu para abrir {alvo:?} (código {})", r as isize));
    }
}

/// Roda uma linha de comando sem herdar o modo administrador.
pub fn comando_como_usuario(linha: &str) {
    if eh_admin() {
        let l = format!("\"{}\" /c \"{linha}\"", cmd_exe().display());
        match rodar_como_usuario(&l) {
            Ok(()) => return,
            Err(e) => log::escrever(&format!("Não deu para rodar como usuário ({e}); rodando direto.")),
        }
    }
    let r = std::process::Command::new(cmd_exe())
        // aspas em volta de tudo: o cmd tira o primeiro e o último " da linha
        .raw_arg(format!("/c \"{linha}\""))
        .current_dir(pasta_usuario())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(e) = r {
        log::escrever(&format!("Erro ao rodar {linha:?}: {e}"));
    }
}

/// Uso: Roleta.exe --teste-usuario arquivo.txt (o instalador confere que o
/// que a roleta abre roda como usuário normal).
pub fn teste_usuario(saida: &str) {
    let whoami = cmd_exe().with_file_name("whoami.exe");
    comando_como_usuario(&format!("\"{}\" /groups > \"{saida}\"", whoami.display()));
}

// ---------------------------------------------------------------- teclado

fn tecla(vk: u16, solta: bool) -> INPUT {
    let estendida = matches!(vk, 0x21..=0x2E | 0x5B | 0x5C | 0x5D | 0xA3 | 0xA5);
    let mut flags = 0;
    if estendida {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if solta {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn unicode(u: u16, solta: bool) -> INPUT {
    let flags = KEYEVENTF_UNICODE | if solta { KEYEVENTF_KEYUP } else { 0 };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: 0, wScan: u, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn enviar(entradas: &[INPUT]) {
    if entradas.is_empty() {
        return;
    }
    unsafe {
        SendInput(entradas.len() as u32, entradas.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

/// Nome de tecla no formato do pynput ("ctrl", "shift", "f5", "a"...) -> código virtual.
fn codigo_tecla(nome: &str) -> Option<u16> {
    let n = nome.trim().trim_start_matches('<').trim_end_matches('>').to_lowercase();
    let vk = match n.as_str() {
        "ctrl" | "ctrl_l" | "control" => 0x11,
        "ctrl_r" => 0xA3,
        "shift" | "shift_l" => 0x10,
        "shift_r" => 0xA1,
        "alt" | "alt_l" => 0x12,
        "alt_r" | "alt_gr" => 0xA5,
        "cmd" | "cmd_l" | "win" | "super" => 0x5B,
        "cmd_r" => 0x5C,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "page_up" => 0x21,
        "page_down" => 0x22,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "print_screen" => 0x2C,
        "caps_lock" => 0x14,
        "num_lock" => 0x90,
        "menu" => 0x5D,
        "pause" => 0x13,
        "media_play_pause" => 0xB3,
        "media_next" => 0xB0,
        "media_previous" => 0xB1,
        "media_volume_up" => 0xAF,
        "media_volume_down" => 0xAE,
        "media_volume_mute" => 0xAD,
        _ => {
            if let Some(f) = n.strip_prefix('f').and_then(|x| x.parse::<u16>().ok()) {
                if (1..=24).contains(&f) {
                    return Some(0x70 + f - 1);
                }
            }
            let mut cs = n.chars();
            let c = cs.next()?;
            if cs.next().is_some() {
                return None;
            }
            if c.is_ascii_alphanumeric() {
                return Some(c.to_ascii_uppercase() as u16);
            }
            let r = unsafe { VkKeyScanW(c as u16) };
            if r == -1 {
                return None;
            }
            (r as u16) & 0xFF
        }
    };
    Some(vk)
}

fn atalho(valor: &str) -> Result<(), String> {
    // "<ctrl>+<shift>+s" (formato do pynput). "+" sozinho no fim = tecla "+".
    let mut partes: Vec<&str> = valor.split('+').collect();
    if valor.ends_with("++") {
        partes.truncate(partes.len().saturating_sub(2));
        partes.push("+");
    }
    let mut vks = Vec::new();
    for p in partes.iter().filter(|p| !p.trim().is_empty()) {
        vks.push(codigo_tecla(p).ok_or_else(|| format!("tecla desconhecida: {p:?}"))?);
    }
    let mut ent: Vec<INPUT> = vks.iter().map(|&v| tecla(v, false)).collect();
    ent.extend(vks.iter().rev().map(|&v| tecla(v, true)));
    enviar(&ent);
    Ok(())
}

fn digitar(texto: &str) {
    let mut ent = Vec::new();
    for u in texto.encode_utf16() {
        ent.push(unicode(u, false));
        ent.push(unicode(u, true));
    }
    enviar(&ent);
}

/// Executa a opção. Devolve `true` se a opção pede para sair da roleta.
pub fn executar(op: &Opcao) -> bool {
    let r = match op.tipo.as_str() {
        "abrir" => {
            abrir_como_usuario(&op.valor);
            Ok(())
        }
        "comando" => {
            comando_como_usuario(&op.valor);
            Ok(())
        }
        "teclas" => atalho(&op.valor),
        "texto" => {
            digitar(&op.valor);
            Ok(())
        }
        "sair" => return true,
        t => Err(format!("tipo desconhecido: {t:?}")),
    };
    if let Err(e) = r {
        log::escrever(&format!("Erro ao executar {:?}: {e}", op.nome));
    }
    false
}
