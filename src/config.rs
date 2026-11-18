//! Configuração em roleta_config.json: %APPDATA%\Roleta (Windows, mesmo arquivo
//! da versão Python) ou ~/.config/roleta (Linux).

use serde::Deserialize;
use std::path::PathBuf;

use crate::log;

#[derive(Clone, Debug, Deserialize)]
pub struct Opcao {
    #[serde(default = "nome_padrao")]
    pub nome: String,
    /// Nome de um ícone embutido ("pasta", "print", "terminal", "chrome",
    /// "tarefas"), caminho de um .svg, ou código de glifo (Segoe Fluent Icons
    /// no Windows, Nerd Font no Linux).
    #[serde(default)]
    pub icone: String,
    #[serde(default)]
    pub tipo: String,
    #[serde(default)]
    pub valor: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    #[serde(default = "botao_padrao")]
    pub botao: String,
    #[serde(default = "raio_padrao")]
    pub raio: f32,
    #[serde(default)]
    pub opcoes: Vec<Opcao>,
}

fn nome_padrao() -> String {
    "?".into()
}
fn botao_padrao() -> String {
    "x1".into()
}
fn raio_padrao() -> f32 {
    RAIO_PADRAO
}

pub const RAIO_PADRAO: f32 = 130.0;

/// Onde ficam a configuração e os ícones próprios.
#[cfg(windows)]
pub fn pasta_dados() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(pasta_usuario);
    let p = base.join("Roleta");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Onde ficam a configuração e os ícones próprios.
#[cfg(not(windows))]
pub fn pasta_dados() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| pasta_usuario().join(".config"));
    let p = base.join("roleta");
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(windows)]
pub fn pasta_usuario() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

#[cfg(not(windows))]
pub fn pasta_usuario() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn caminho() -> PathBuf {
    // ROLETA_CONFIG permite testar com outra configuração
    match std::env::var_os("ROLETA_CONFIG") {
        Some(p) => PathBuf::from(p),
        None => pasta_dados().join("roleta_config.json"),
    }
}

#[cfg(windows)]
fn texto_padrao() -> String {
    let proj = pasta_usuario().join("proj").to_string_lossy().into_owned();
    let v = serde_json::json!({
        "botao": "x1",
        "raio": RAIO_PADRAO,
        "opcoes": [
            {"nome": "Pasta proj", "icone": "pasta",   "tipo": "abrir",   "valor": proj},
            {"nome": "Print",      "icone": "print",   "tipo": "abrir",   "valor": "ms-screenclip:"},
            {"nome": "Terminal",   "icone": "terminal","tipo": "comando", "valor": format!("wt -d \"{proj}\"")},
            {"nome": "Chrome",     "icone": "chrome",  "tipo": "abrir",   "valor": "chrome"},
            {"nome": "Gerenciador de tarefas", "icone": "tarefas", "tipo": "abrir", "valor": "taskmgr"},
        ]
    });
    serde_json::to_string_pretty(&v).unwrap()
}

/// Mesmas opções do Windows com os programas daqui. A pasta ~/ai é criada se
/// não existir; o "Claude" abre o kitty nela já rodando o claude (se estiver
/// instalado) e volta para o shell quando ele termina.
#[cfg(not(windows))]
fn texto_padrao() -> String {
    let v = serde_json::json!({
        "botao": "x1",
        "raio": RAIO_PADRAO,
        "opcoes": [
            {"nome": "Pasta ai", "icone": "pasta",    "tipo": "comando", "valor": "mkdir -p ~/ai && thunar ~/ai"},
            {"nome": "Print",    "icone": "print",    "tipo": "comando", "valor": "xfce4-screenshooter -r"},
            {"nome": "Claude",   "icone": "terminal", "tipo": "comando",
             "valor": "mkdir -p ~/ai && kitty --directory ~/ai sh -c 'command -v claude >/dev/null && claude; exec \"${SHELL:-bash}\"'"},
            {"nome": "Chrome",   "icone": "chrome",   "tipo": "abrir",   "valor": "google-chrome"},
            {"nome": "Gerenciador de tarefas", "icone": "tarefas", "tipo": "comando", "valor": "kitty -e aerotop"},
        ]
    });
    serde_json::to_string_pretty(&v).unwrap()
}

fn padrao() -> Config {
    serde_json::from_str(&texto_padrao()).unwrap()
}

pub fn carregar() -> Config {
    let p = caminho();
    let mut cfg = match std::fs::read_to_string(&p) {
        Err(_) => {
            if let Err(e) = std::fs::write(&p, texto_padrao()) {
                log::escrever(&format!("Não deu para criar {}: {e}", p.display()));
            }
            padrao()
        }
        Ok(txt) => match serde_json::from_str::<Config>(txt.trim_start_matches('\u{feff}')) {
            Ok(c) => c,
            Err(e) => {
                log::escrever(&format!("Erro em {} ({e}); usando o padrão.", p.display()));
                padrao()
            }
        },
    };
    if cfg.opcoes.is_empty() {
        log::escrever("Nenhuma opção na configuração; usando as padrão.");
        cfg.opcoes = padrao().opcoes;
    }
    if !cfg.raio.is_finite() {
        cfg.raio = RAIO_PADRAO;
    }
    cfg.raio = cfg.raio.clamp(60.0, 400.0);
    cfg
}
