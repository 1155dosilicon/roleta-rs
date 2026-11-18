//! Linux (X11): grab passivo do botão lateral na janela raiz, janela ARGB
//! sem moldura (override-redirect) pintada com PutImage, ações com
//! xdg-open/sh/XTest e ícone na bandeja via StatusNotifierItem.

pub mod acoes;
mod bandeja;
mod janela;

pub use janela::rodar;

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;

use crate::config::pasta_usuario;
use crate::log;

/// A sessão gráfica (autostart do XFCE etc.) costuma não ter ~/.local/bin no
/// PATH; sem isso os comandos da roleta não acham kitty, claude...
pub fn preparar_processo() {
    limpar_ambiente_claude();
    let casa = pasta_usuario();
    let atual: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let mut novo: Vec<PathBuf> = [".local/bin", ".cargo/bin", ".bun/bin"]
        .iter()
        .map(|d| casa.join(d))
        .filter(|d| d.is_dir() && !atual.contains(d))
        .collect();
    if novo.is_empty() {
        return;
    }
    novo.extend(atual);
    if let Ok(p) = std::env::join_paths(novo) {
        // SAFETY: chamado no início do main, antes de existir outra thread.
        unsafe { std::env::set_var("PATH", p) };
    }
}

/// Se a roleta foi (re)iniciada de dentro de uma sessão do Claude Code, ela
/// herda CLAUDECODE=1, CLAUDE_CODE_* etc. e repassa tudo aos programas que
/// abre: o claude aberto pela roleta se acha "aninhado" e desliga o
/// salvamento do transcript. Tira essas variáveis antes de lançar qualquer coisa.
fn limpar_ambiente_claude() {
    let nomes: Vec<_> = std::env::vars_os()
        .filter_map(|(k, _)| {
            let n = k.to_str()?;
            (n == "CLAUDECODE"
                || n == "CLAUDE_PID"
                || n == "CLAUDE_EFFORT"
                || n == "AI_AGENT"
                || n.starts_with("CLAUDE_CODE_"))
            .then_some(k)
        })
        .collect();
    for k in nomes {
        // SAFETY: chamado no início do main, antes de existir outra thread.
        unsafe { std::env::remove_var(k) };
    }
}

/// Trava (flock) de instância única; solta ao fechar o arquivo.
pub struct Instancia(#[allow(dead_code)] File);

pub fn instancia_unica(teste: bool) -> Option<Instancia> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let p = dir.join(if teste { "roleta-teste.lock" } else { "roleta.lock" });
    let f = match OpenOptions::new().create(true).truncate(false).write(true).open(&p) {
        Ok(f) => f,
        Err(e) => {
            log::escrever(&format!("Não deu para abrir a trava {}: {e}", p.display()));
            return None;
        }
    };
    match f.try_lock() {
        Ok(()) => Some(Instancia(f)),
        Err(TryLockError::WouldBlock) => {
            eprintln!("A roleta já está rodando.");
            None
        }
        Err(TryLockError::Error(e)) => {
            log::escrever(&format!("Trava {} falhou ({e}); seguindo assim mesmo.", p.display()));
            Some(Instancia(f))
        }
    }
}
