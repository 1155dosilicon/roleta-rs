//! Configuração em %APPDATA%\Roleta\roleta_config.json (mesmo arquivo da versão Python).

use serde::Deserialize;
use std::path::PathBuf;

use crate::log;

#[derive(Clone, Debug, Deserialize)]
pub struct Opcao {
    #[serde(default = "nome_padrao")]
    pub nome: String,
    /// Nome de um ícone embutido ("pasta", "print", "terminal", "chrome",
    /// "tarefas"), caminho de um .svg, ou código de glifo da Segoe Fluent Icons.
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

pub fn pasta_dados() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(pasta_usuario);
    let p = base.join("Roleta");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn pasta_usuario() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

pub fn caminho() -> PathBuf {
    // ROLETA_CONFIG permite testar com outra configuração
    match std::env::var_os("ROLETA_CONFIG") {
        Some(p) => PathBuf::from(p),
        None => pasta_dados().join("roleta_config.json"),
    }
}

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
