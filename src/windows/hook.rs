//! Hook global do mouse (WH_MOUSE_LL) numa thread própria.
//! Engole o botão lateral (o navegador não vai "voltar/avançar" quando você usa
//! a roleta) e avisa a janela de controle por PostMessage.

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU8, Ordering::Relaxed};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, MSG, MSLLHOOKSTRUCT,
    WH_MOUSE_LL, WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::log;

/// Mensagem enviada para a janela de controle; wParam = um dos EV_*.
pub const WM_ROLETA: u32 = WM_APP + 1;
pub const EV_ABRIR: usize = 1;
pub const EV_SOLTAR: usize = 2;
pub const EV_CLIQUE: usize = 3;
pub const EV_CANCELAR: usize = 4;

/// Roleta aberta esperando clique: o hook captura esquerdo/direito.
pub static MODO_CLIQUE: AtomicBool = AtomicBool::new(false);

static ALVO: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static ACEITOS: AtomicU8 = AtomicU8::new(0b01);
static THREAD: AtomicU32 = AtomicU32::new(0);
// engole o "soltar" do clique que o hook já engoliu
static ENGOLIR_L: AtomicBool = AtomicBool::new(false);
static ENGOLIR_R: AtomicBool = AtomicBool::new(false);

fn avisar(ev: usize) {
    unsafe {
        PostMessageW(ALVO.load(Relaxed), WM_ROLETA, ev, 0);
    }
}

unsafe extern "system" fn proc_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        let msg = wparam as u32;
        match msg {
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let x = (info.mouseData >> 16) & 0xFFFF;
                let aceitos = ACEITOS.load(Relaxed);
                if (x == 1 && aceitos & 0b01 != 0) || (x == 2 && aceitos & 0b10 != 0) {
                    avisar(if msg == WM_XBUTTONDOWN { EV_ABRIR } else { EV_SOLTAR });
                    return 1;
                }
            }
            WM_LBUTTONDOWN if MODO_CLIQUE.load(Relaxed) => {
                ENGOLIR_L.store(true, Relaxed);
                avisar(EV_CLIQUE);
                return 1;
            }
            WM_RBUTTONDOWN if MODO_CLIQUE.load(Relaxed) => {
                ENGOLIR_R.store(true, Relaxed);
                avisar(EV_CANCELAR);
                return 1;
            }
            WM_LBUTTONUP if ENGOLIR_L.swap(false, Relaxed) => return 1,
            WM_RBUTTONUP if ENGOLIR_R.swap(false, Relaxed) => return 1,
            _ => {}
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

pub fn iniciar(alvo: *mut c_void, botao: &str) {
    ALVO.store(alvo, Relaxed);
    ACEITOS.store(
        match botao {
            "x2" => 0b10,
            "ambos" => 0b11,
            _ => 0b01,
        },
        Relaxed,
    );
    std::thread::spawn(|| unsafe {
        THREAD.store(GetCurrentThreadId(), Relaxed);
        // o hook tem que responder rápido, senão o mouse "engasga"
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(proc_mouse), GetModuleHandleW(std::ptr::null()), 0);
        if hook.is_null() {
            log::escrever("ERRO: não deu para instalar o hook do mouse");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
    });
}

pub fn parar() {
    let t = THREAD.load(Relaxed);
    if t != 0 {
        unsafe {
            PostThreadMessageW(t, WM_QUIT, 0, 0);
        }
    }
}
