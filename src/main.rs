//! Roleta de opções (menu radial) acionada pelo botão lateral do mouse.
//! Windows e Linux (X11).
//!
//! Como usar:
//!   - SEGURE o botão lateral -> a roleta aparece em volta do cursor.
//!     Mova o mouse na direção da opção e SOLTE para executar.
//!     Soltar no centro ou fora da roleta = cancelar.
//!   - Ou dê um CLIQUE rápido no botão lateral -> a roleta fica aberta;
//!     clique com o botão esquerdo na opção. Botão direito = cancelar.
//!
//! Windows: opções em %APPDATA%\Roleta\roleta_config.json, log em
//! %APPDATA%\Roleta\roleta.log. Instalado como C:\Program Files\Roleta\Roleta.exe,
//! iniciado no logon por uma tarefa agendada com privilégio de administrador
//! (o hook do mouse funciona até em cima do Gerenciador de Tarefas). Os
//! programas que a roleta abre rodam como usuário normal.
//!
//! Linux: opções em ~/.config/roleta/roleta_config.json, log em
//! ~/.local/state/roleta/roleta.log. Instalado em ~/.local/bin/roleta e
//! iniciado com a sessão (~/.config/autostart/roleta.desktop).
//!
//! Argumentos:
//!   --previa pasta            salva imagens PNG do visual e sai
//!   --teste                   roda ao lado da roleta instalada (outra trava)
//!   --teste-usuario arq.txt   (Windows, usado pelo instalador)

#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod desenho;
mod log;
mod roleta;

#[cfg_attr(windows, path = "windows/mod.rs")]
#[cfg_attr(target_os = "linux", path = "linux/mod.rs")]
mod plataforma;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("a roleta só funciona no Windows e no Linux (X11)");

fn main() {
    std::panic::set_hook(Box::new(|info| log::escrever(&format!("ERRO FATAL: {info}"))));
    let args: Vec<String> = std::env::args().collect();

    #[cfg(windows)]
    if args.len() == 3 && args[1] == "--teste-usuario" {
        plataforma::acoes::teste_usuario(&args[2]);
        return;
    }
    if args.len() == 3 && args[1] == "--previa" {
        roleta::previa(&args[2]);
        return;
    }
    let teste = args.iter().any(|a| a == "--teste");

    plataforma::preparar_processo();
    let Some(trava) = plataforma::instancia_unica(teste) else {
        return;
    };

    log::inicio();
    let cfg = config::carregar();
    let reiniciar = plataforma::rodar(cfg);

    drop(trava);
    if reiniciar
        && let Ok(exe) = std::env::current_exe() {
            // Linux: se o binário foi trocado (reinstalação), o link vem com " (deleted)"
            let exe = match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
                Some(s) => std::path::PathBuf::from(s),
                None => exe,
            };
            let mut c = std::process::Command::new(exe);
            if teste {
                c.arg("--teste");
            }
            if let Err(e) = c.spawn() {
                log::escrever(&format!("Não deu para reiniciar: {e}"));
            }
        }
}
