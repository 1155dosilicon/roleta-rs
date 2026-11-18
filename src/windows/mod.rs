//! Windows: hook global do mouse (WH_MOUSE_LL), janela em camadas com
//! UpdateLayeredWindow, bandeja e ações com re-paiamento para o Explorer.

pub mod acoes;
mod hook;
mod janela;

pub use janela::rodar;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

pub fn preparar_processo() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Mutex de instância única; solto ao sair do escopo.
pub struct Instancia(HANDLE);

impl Drop for Instancia {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Evita duas roletas (ex.: inicialização + abrir na mão).
pub fn instancia_unica(teste: bool) -> Option<Instancia> {
    let nome = acoes::wide(if teste { r"Local\RoletaMouseTeste" } else { r"Local\RoletaMouse" });
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, nome.as_ptr()) };
    let erro = unsafe { GetLastError() };
    // já existe, ou existe mas foi criado por uma roleta de administrador
    if mutex.is_null() {
        return None;
    }
    if erro == ERROR_ALREADY_EXISTS || erro == ERROR_ACCESS_DENIED {
        unsafe { CloseHandle(mutex) };
        return None;
    }
    Some(Instancia(mutex))
}
