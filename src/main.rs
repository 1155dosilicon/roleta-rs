//! Roleta de opções (menu radial) acionada pelo botão lateral do mouse - Windows.
//!
//! Como usar:
//!   - SEGURE o botão lateral -> a roleta aparece em volta do cursor.
//!     Mova o mouse na direção da opção e SOLTE para executar.
//!     Soltar no centro ou fora da roleta = cancelar.
//!   - Ou dê um CLIQUE rápido no botão lateral -> a roleta fica aberta;
//!     clique com o botão esquerdo na opção. Botão direito = cancelar.
//!
//! As opções ficam em %APPDATA%\Roleta\roleta_config.json (criado na 1ª execução).
//! Instalado como C:\Program Files\Roleta\Roleta.exe, iniciado no logon por uma
//! tarefa agendada com privilégio de administrador (o hook do mouse funciona
//! até em cima do Gerenciador de Tarefas). Os programas que a roleta abre rodam
//! como usuário normal. Erros vão para %APPDATA%\Roleta\roleta.log.
//!
//! Argumentos:
//!   --teste-usuario arq.txt   (usado pelo instalador)
//!   --previa pasta            salva imagens PNG do visual e sai
//!   --teste                   roda ao lado da roleta instalada (outro mutex)

#![windows_subsystem = "windows"]

mod acoes;
mod config;
mod desenho;
mod hook;
mod janela;
mod log;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

fn main() {
    std::panic::set_hook(Box::new(|info| log::escrever(&format!("ERRO FATAL: {info}"))));
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 3 && args[1] == "--teste-usuario" {
        acoes::teste_usuario(&args[2]);
        return;
    }
    if args.len() == 3 && args[1] == "--previa" {
        janela::previa(&args[2]);
        return;
    }
    let teste = args.iter().any(|a| a == "--teste");

    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    // Instância única: evita duas roletas (ex.: inicialização + abrir na mão).
    let nome = acoes::wide(if teste { r"Local\RoletaMouseTeste" } else { r"Local\RoletaMouse" });
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, nome.as_ptr()) };
    let erro = unsafe { GetLastError() };
    // já existe, ou existe mas foi criado por uma roleta de administrador
    if mutex.is_null() || erro == ERROR_ALREADY_EXISTS || erro == ERROR_ACCESS_DENIED {
        return;
    }

    log::inicio();
    let cfg = config::carregar();
    let reiniciar = janela::rodar(cfg);

    unsafe { CloseHandle(mutex) };
    if reiniciar {
        if let Ok(exe) = std::env::current_exe() {
            let mut c = std::process::Command::new(exe);
            if teste {
                c.arg("--teste");
            }
            if let Err(e) = c.spawn() {
                log::escrever(&format!("Não deu para reiniciar: {e}"));
            }
        }
    }
}
