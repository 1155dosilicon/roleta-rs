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
    CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
    PROCESS_CREATE_PROCESS, PROCESS_INFORMATION, STARTUPINFOEXW, UpdateProcThreadAttribute,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
    VkKeyScanW,
};
use windows_sys::Win32::UI::Shell::{IsUserAnAdmin, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId, SW_SHOWNORMAL};

use crate::atalho;
use crate::config::{Acao, Opcao, pasta_usuario};
use crate::log;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn eh_admin() -> bool {
    // SAFETY: sem argumentos nem pré-condições.
    unsafe { IsUserAnAdmin() != 0 }
}

fn cmd_exe() -> PathBuf {
    let raiz = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(raiz).join("System32").join("cmd.exe")
}

fn erro_win(onde: &str) -> String {
    // SAFETY: sem pré-condições; lê o erro da última chamada desta thread.
    format!("{onde} falhou (erro {})", unsafe { GetLastError() })
}

/// Handle fechado no Drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: handle válido, aberto por nós e fechado só aqui.
        unsafe { CloseHandle(self.0) };
    }
}

/// Lista de atributos de processo com um atributo só (o processo pai).
struct ListaAtributos {
    // Vec<usize> em vez de Vec<u8>: a lista guarda ponteiros e precisa de
    // alinhamento de ponteiro
    _buf: Vec<usize>,
    lista: LPPROC_THREAD_ATTRIBUTE_LIST,
    /// O UpdateProcThreadAttribute guarda o endereço do handle, que precisa
    /// existir até o CreateProcessW: fica numa Box (endereço fixo).
    _pai: Box<HANDLE>,
}

impl ListaAtributos {
    fn com_pai(pai: HANDLE) -> Result<Self, String> {
        let mut tam = 0usize;
        // SAFETY: com lista nula a chamada só informa o tamanho necessário.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut tam) };
        let mut buf = vec![0usize; tam.div_ceil(size_of::<usize>())];
        let lista = buf.as_mut_ptr().cast();
        // SAFETY: `buf` tem pelo menos `tam` bytes, alinhado a ponteiro.
        if unsafe { InitializeProcThreadAttributeList(lista, 1, 0, &mut tam) } == 0 {
            return Err(erro_win("InitializeProcThreadAttributeList"));
        }
        let pai = Box::new(pai);
        let ok = {
            // SAFETY: lista inicializada; o valor aponta para um HANDLE que vive
            // (na Box) tanto quanto a lista.
            unsafe {
                UpdateProcThreadAttribute(
                    lista,
                    0,
                    PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
                    std::ptr::from_ref::<HANDLE>(&pai).cast(),
                    size_of::<HANDLE>(),
                    null_mut(),
                    null(),
                )
            }
        };
        let l = ListaAtributos { _buf: buf, lista, _pai: pai };
        if ok == 0 {
            return Err(erro_win("UpdateProcThreadAttribute"));
        }
        Ok(l)
    }
}

impl Drop for ListaAtributos {
    fn drop(&mut self) {
        // SAFETY: a lista foi inicializada no com_pai e o buffer ainda existe.
        unsafe { DeleteProcThreadAttributeList(self.lista) };
    }
}

fn rodar_como_usuario(linha: &str) -> Result<(), String> {
    let mut pid = 0u32;
    // SAFETY: `pid` é um u32 válido para escrita; janela nula = pid fica 0.
    unsafe { GetWindowThreadProcessId(GetShellWindow(), &mut pid) };
    if pid == 0 {
        return Err("Explorer não encontrado".into());
    }
    // SAFETY: sem ponteiros; falha devolve nulo.
    let pai = unsafe { OpenProcess(PROCESS_CREATE_PROCESS, 0, pid) };
    if pai.is_null() {
        return Err(erro_win("OpenProcess"));
    }
    let pai = Handle(pai);
    let atributos = ListaAtributos::com_pai(pai.0)?;

    // SAFETY: STARTUPINFOEXW e PROCESS_INFORMATION são POD (zero é válido).
    let (mut si, mut pi): (STARTUPINFOEXW, PROCESS_INFORMATION) = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = atributos.lista;
    let mut cmd = wide(linha);
    let dir = wide(&pasta_usuario().to_string_lossy());
    // SAFETY: `cmd` é gravável e termina em 0 (o CreateProcessW pode mexer
    // nele); `dir` termina em 0; `si` e `atributos` vivem até o fim da chamada.
    let ok = unsafe {
        CreateProcessW(
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
        )
    };
    if ok == 0 {
        return Err(erro_win("CreateProcessW"));
    }
    drop((Handle(pi.hThread), Handle(pi.hProcess)));
    Ok(())
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
    // SAFETY: as strings terminam em 0 e vivem até o fim da chamada.
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

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12; // Alt

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
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn unicode(u: u16, solta: bool) -> INPUT {
    let flags = KEYEVENTF_UNICODE | if solta { KEYEVENTF_KEYUP } else { 0 };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: u, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn enviar(entradas: &[INPUT]) {
    if entradas.is_empty() {
        return;
    }
    // SAFETY: o ponteiro e o tamanho vêm de um slice válido de INPUT.
    let n = unsafe { SendInput(entradas.len() as u32, entradas.as_ptr(), size_of::<INPUT>() as i32) };
    if n as usize != entradas.len() {
        // ex.: a janela ativa é de administrador e a roleta não (UIPI)
        log::escrever(&format!("SendInput mandou {n} de {} eventos", entradas.len()));
    }
}

/// Modificadores que um caractere precisa no layout atual (bits do
/// VkKeyScanW: 1 = Shift, 2 = Ctrl, 4 = Alt; Ctrl+Alt = AltGr).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Mods(u8);

impl Mods {
    fn vks(self) -> impl Iterator<Item = u16> {
        [(1, VK_SHIFT), (2, VK_CONTROL), (4, VK_MENU)]
            .into_iter()
            .filter(move |&(b, _)| self.0 & b != 0)
            .map(|(_, v)| v)
    }
}

/// Nome de tecla (já normalizado por [`atalho::teclas`]) -> código virtual e
/// modificadores que ela precisa (ex.: "?" no layout americano = Shift + "/").
fn codigo_tecla(n: &str) -> Option<(u16, Mods)> {
    let vk = match n {
        "ctrl" | "ctrl_l" | "control" => VK_CONTROL,
        "ctrl_r" => 0xA3,
        "shift" | "shift_l" => VK_SHIFT,
        "shift_r" => 0xA1,
        "alt" | "alt_l" => VK_MENU,
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
            if let Some(f) = atalho::numero_f(n) {
                return Some((0x70 + u16::from(f) - 1, Mods::default()));
            }
            let c = atalho::caractere(n)?;
            if c.is_ascii_alphanumeric() {
                return Some((c.to_ascii_uppercase() as u16, Mods::default()));
            }
            let mut u = [0u16; 2];
            let [u] = c.encode_utf16(&mut u) else { return None };
            // SAFETY: sem ponteiros; devolve -1 se o caractere não existe no layout.
            let r = unsafe { VkKeyScanW(*u) };
            if r == -1 {
                return None;
            }
            let [vk, mods] = r.to_le_bytes();
            return Some((u16::from(vk), Mods(mods & 0b111)));
        }
    };
    Some((vk, Mods::default()))
}

fn eh_modificador(vk: u16, m: u16) -> bool {
    let (l, r) = match m {
        VK_SHIFT => (0xA0, 0xA1),
        VK_CONTROL => (0xA2, 0xA3),
        _ => (0xA4, 0xA5),
    };
    vk == m || vk == l || vk == r
}

/// Teclas na ordem de apertar: os modificadores que algum caractere precisa
/// e que o atalho não tem vêm primeiro.
fn sequencia(teclas: &[(u16, Mods)]) -> Vec<u16> {
    let todos = Mods(teclas.iter().fold(0, |a, &(_, m)| a | m.0));
    let mut vks: Vec<u16> = todos.vks().filter(|&m| !teclas.iter().any(|&(vk, _)| eh_modificador(vk, m))).collect();
    vks.extend(teclas.iter().map(|&(vk, _)| vk));
    vks
}

fn atalho(valor: &str) -> Result<(), String> {
    let teclas = atalho::teclas(valor)?
        .iter()
        .map(|n| codigo_tecla(n).ok_or_else(|| format!("tecla desconhecida: {n:?}")))
        .collect::<Result<Vec<_>, _>>()?;
    let vks = sequencia(&teclas);
    let mut ent: Vec<INPUT> = vks.iter().map(|&v| tecla(v, false)).collect();
    ent.extend(vks.iter().rev().map(|&v| tecla(v, true)));
    enviar(&ent);
    Ok(())
}

fn digitar(texto: &str) {
    let mut ent = Vec::with_capacity(texto.len() * 2);
    for u in texto.encode_utf16() {
        ent.push(unicode(u, false));
        ent.push(unicode(u, true));
    }
    enviar(&ent);
}

/// Executa a opção. Devolve `true` se a opção pede para sair da roleta.
pub fn executar(op: &Opcao) -> bool {
    let r = match &op.acao {
        Acao::Abrir(v) => {
            abrir_como_usuario(v);
            Ok(())
        }
        Acao::Comando(v) => {
            comando_como_usuario(v);
            Ok(())
        }
        Acao::Teclas(v) => atalho(v),
        Acao::Texto(v) => {
            digitar(v);
            Ok(())
        }
        Acao::Sair => return true,
        Acao::Desconhecida(t) => Err(format!("tipo desconhecido: {t:?}")),
    };
    if let Err(e) = r {
        log::escrever(&format!("Erro ao executar {:?}: {e}", op.nome));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEM: Mods = Mods(0);

    #[test]
    fn modificador_que_falta_vem_antes() {
        // "<ctrl>+?" no layout americano: ? = Shift + VK_OEM_2 (0xBF)
        assert_eq!(sequencia(&[(VK_CONTROL, SEM), (0xBF, Mods(1))]), [VK_SHIFT, VK_CONTROL, 0xBF]);
    }

    #[test]
    fn modificador_que_ja_tem_nao_repete() {
        assert_eq!(sequencia(&[(VK_CONTROL, SEM), (0xA1, SEM), (0xBF, Mods(1))]), [VK_CONTROL, 0xA1, 0xBF]);
        assert_eq!(sequencia(&[(VK_CONTROL, SEM), (0x53, SEM)]), [VK_CONTROL, 0x53]);
    }

    #[test]
    fn altgr_vira_ctrl_alt() {
        assert_eq!(sequencia(&[(0x51, Mods(6))]), [VK_CONTROL, VK_MENU, 0x51]);
    }

    #[test]
    fn teclas_com_nome() {
        assert_eq!(codigo_tecla("ctrl"), Some((VK_CONTROL, SEM)));
        assert_eq!(codigo_tecla("f5"), Some((0x74, SEM)));
        assert_eq!(codigo_tecla("s"), Some((0x53, SEM)));
        assert_eq!(codigo_tecla("nada"), None);
    }
}
