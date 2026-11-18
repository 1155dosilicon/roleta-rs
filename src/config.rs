//! Configuração em roleta_config.json: %APPDATA%\Roleta (Windows, mesmo arquivo
//! da versão Python) ou ~/.config/roleta (Linux).
//!
//! O JSON é lido em structs "brutas" (só texto, tudo opcional) e depois
//! convertido para os tipos usados pelo resto do programa. Valores errados
//! viram avisos no log em vez de derrubar a configuração inteira.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;

use serde::Deserialize;

use crate::log;

pub const RAIO_PADRAO: f32 = 130.0;
const RAIO_MIN: f32 = 60.0;
const RAIO_MAX: f32 = 400.0;

/// Botão lateral que abre a roleta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Botao {
    /// Botão de trás ("voltar"; botão 8 no X11).
    X1,
    /// Botão da frente ("avançar"; botão 9 no X11).
    X2,
    Ambos,
}

/// O que uma opção faz.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Acao {
    /// Abre arquivo, pasta, site ou programa.
    Abrir(String),
    /// Roda uma linha de comando (sh no Linux, cmd no Windows).
    Comando(String),
    /// Atalho de teclado, ex.: "<ctrl>+<shift>+s".
    Teclas(String),
    /// Digita um texto.
    Texto(String),
    /// Fecha a roleta.
    Sair,
    /// `tipo` desconhecido: a opção aparece, mas só registra um erro no log.
    Desconhecida(String),
}

#[derive(Clone, Debug)]
pub struct Opcao {
    pub nome: String,
    /// Nome de um ícone embutido ("pasta", "print", "terminal", "chrome",
    /// "tarefas"), caminho de um .svg, ou código de glifo (Segoe Fluent Icons
    /// no Windows, Nerd Font no Linux).
    pub icone: String,
    pub acao: Acao,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub botao: Botao,
    pub raio: f32,
    /// Nunca vazia.
    pub opcoes: Vec<Opcao>,
}

// ------------------------------------------------------------ JSON

#[derive(Deserialize)]
struct OpcaoBruta {
    #[serde(default)]
    nome: Option<String>,
    #[serde(default)]
    icone: String,
    #[serde(default)]
    tipo: String,
    #[serde(default)]
    valor: String,
    #[serde(flatten)]
    resto: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct ConfigBruta {
    #[serde(default)]
    botao: Option<String>,
    #[serde(default)]
    raio: Option<f32>,
    #[serde(default)]
    opcoes: Vec<OpcaoBruta>,
    #[serde(flatten)]
    resto: BTreeMap<String, serde_json::Value>,
}

fn avisar_resto(onde: &str, resto: &BTreeMap<String, serde_json::Value>, avisos: &mut Vec<String>) {
    for chave in resto.keys() {
        avisos.push(format!("{onde}: campo desconhecido {chave:?} (ignorado)"));
    }
}

impl OpcaoBruta {
    fn converter(self, i: usize, avisos: &mut Vec<String>) -> Opcao {
        let onde = format!("opção {}", i + 1);
        avisar_resto(&onde, &self.resto, avisos);
        let acao = match self.tipo.as_str() {
            "abrir" => Acao::Abrir(self.valor),
            "comando" => Acao::Comando(self.valor),
            "teclas" => Acao::Teclas(self.valor),
            "texto" => Acao::Texto(self.valor),
            "sair" => Acao::Sair,
            t => {
                avisos.push(format!("{onde}: tipo {t:?} desconhecido (use abrir, comando, teclas, texto ou sair)"));
                Acao::Desconhecida(t.to_string())
            }
        };
        Opcao { nome: self.nome.unwrap_or_else(|| "?".into()), icone: self.icone, acao }
    }
}

impl ConfigBruta {
    fn converter(self, avisos: &mut Vec<String>) -> Config {
        avisar_resto("configuração", &self.resto, avisos);
        let botao = match self.botao.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("x1") => Botao::X1,
            Some("x2") => Botao::X2,
            Some("ambos") => Botao::Ambos,
            Some(_) => {
                let b = self.botao.as_deref().unwrap_or_default();
                avisos.push(format!("botao {b:?} desconhecido (use x1, x2 ou ambos); usando x1"));
                Botao::X1
            }
        };
        let raio = match self.raio {
            None => RAIO_PADRAO,
            Some(r) if !r.is_finite() => RAIO_PADRAO,
            Some(r) => {
                let c = r.clamp(RAIO_MIN, RAIO_MAX);
                if c != r {
                    avisos.push(format!("raio {r} fora de {RAIO_MIN}..{RAIO_MAX}; usando {c}"));
                }
                c
            }
        };
        let opcoes = self.opcoes.into_iter().enumerate().map(|(i, o)| o.converter(i, avisos)).collect();
        Config { botao, raio, opcoes }
    }
}

/// Lê o texto do JSON. Devolve a configuração e os avisos (campos e valores
/// desconhecidos, raio fora do limite, lista de opções vazia...).
fn interpretar(txt: &str) -> Result<(Config, Vec<String>), serde_json::Error> {
    let bruta: ConfigBruta = serde_json::from_str(txt.trim_start_matches('\u{feff}'))?;
    let mut avisos = Vec::new();
    let mut cfg = bruta.converter(&mut avisos);
    if cfg.opcoes.is_empty() {
        avisos.push("nenhuma opção na configuração; usando as padrão".into());
        cfg.opcoes = padrao().opcoes;
    }
    Ok((cfg, avisos))
}

/// UTF-8; senão Latin-1 (Bloco de Notas antigo salvando em "ANSI").
fn decodificar(bytes: Vec<u8>) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(s) => (s, true),
        Err(e) => (e.into_bytes().iter().map(|&b| char::from(b)).collect(), false),
    }
}

// ------------------------------------------------------------ pastas

/// Onde ficam a configuração e os ícones próprios.
#[cfg(windows)]
pub fn pasta_dados() -> PathBuf {
    let base = std::env::var_os("APPDATA").map_or_else(pasta_usuario, PathBuf::from);
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
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from(r"C:\"), PathBuf::from)
}

#[cfg(not(windows))]
pub fn pasta_usuario() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

pub fn caminho() -> PathBuf {
    // ROLETA_CONFIG permite testar com outra configuração
    match std::env::var_os("ROLETA_CONFIG") {
        Some(p) => PathBuf::from(p),
        None => pasta_dados().join("roleta_config.json"),
    }
}

// ------------------------------------------------------------ padrão

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
    serde_json::to_string_pretty(&v).expect("JSON montado com json! sempre serializa")
}

/// Mesmas opções do Windows com os programas daqui. A pasta ~/ai é criada se
/// não existir; o "Claude" abre o kitty nela já rodando o claude (se estiver
/// instalado, já em modo bypass de permissões) e volta para o shell quando
/// ele termina.
#[cfg(not(windows))]
fn texto_padrao() -> String {
    let v = serde_json::json!({
        "botao": "x1",
        "raio": RAIO_PADRAO,
        "opcoes": [
            {"nome": "Pasta ai", "icone": "pasta",    "tipo": "comando", "valor": "mkdir -p ~/ai && thunar ~/ai"},
            {"nome": "Print",    "icone": "print",    "tipo": "comando", "valor": "xfce4-screenshooter -r"},
            {"nome": "Claude",   "icone": "terminal", "tipo": "comando",
             "valor": "mkdir -p ~/ai && kitty --directory ~/ai sh -c 'command -v claude >/dev/null && claude --dangerously-skip-permissions; exec \"${SHELL:-bash}\"'"},
            {"nome": "Chrome",   "icone": "chrome",   "tipo": "abrir",   "valor": "google-chrome"},
            {"nome": "Gerenciador de tarefas", "icone": "tarefas", "tipo": "comando", "valor": "kitty -e aerotop"},
        ]
    });
    serde_json::to_string_pretty(&v).expect("JSON montado com json! sempre serializa")
}

fn padrao() -> Config {
    let bruta: ConfigBruta = serde_json::from_str(&texto_padrao()).expect("configuração padrão é válida");
    bruta.converter(&mut Vec::new())
}

// ------------------------------------------------------------ carregar

pub fn carregar() -> Config {
    let p = caminho();
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            if let Err(e) = std::fs::write(&p, texto_padrao()) {
                log::escrever(&format!("Não deu para criar {}: {e}", p.display()));
            }
            return padrao();
        }
        Err(e) => {
            // não sobrescreve: o arquivo existe, só não deu para ler agora
            log::escrever(&format!("Não deu para ler {} ({e}); usando o padrão.", p.display()));
            return padrao();
        }
    };
    let (txt, utf8) = decodificar(bytes);
    if !utf8 {
        log::escrever(&format!("{} não está em UTF-8; lendo como Latin-1.", p.display()));
    }
    match interpretar(&txt) {
        Ok((cfg, avisos)) => {
            for a in avisos {
                log::escrever(&format!("Configuração: {a}"));
            }
            cfg
        }
        Err(e) => {
            log::escrever(&format!("Erro em {} ({e}); usando o padrão.", p.display()));
            padrao()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ler(txt: &str) -> (Config, Vec<String>) {
        interpretar(txt).expect("JSON válido")
    }

    #[test]
    fn padrao_e_valido_e_sem_avisos() {
        let (cfg, avisos) = ler(&texto_padrao());
        assert!(avisos.is_empty(), "{avisos:?}");
        assert_eq!(cfg.botao, Botao::X1);
        assert_eq!(cfg.raio, RAIO_PADRAO);
        assert_eq!(cfg.opcoes.len(), 5);
    }

    #[test]
    fn campos_ausentes_usam_padrao() {
        let (cfg, avisos) = ler(r#"{"opcoes": [{}]}"#);
        assert_eq!(cfg.botao, Botao::X1);
        assert_eq!(cfg.raio, RAIO_PADRAO);
        assert_eq!(cfg.opcoes[0].nome, "?");
        assert_eq!(cfg.opcoes[0].acao, Acao::Desconhecida(String::new()));
        assert_eq!(avisos.len(), 1);
    }

    #[test]
    fn le_todos_os_tipos() {
        let (cfg, avisos) = ler(r#"{"botao": "ambos", "opcoes": [
                {"tipo": "abrir", "valor": "a"}, {"tipo": "comando", "valor": "b"},
                {"tipo": "teclas", "valor": "c"}, {"tipo": "texto", "valor": "d"}, {"tipo": "sair"}]}"#);
        assert!(avisos.is_empty(), "{avisos:?}");
        assert_eq!(cfg.botao, Botao::Ambos);
        let acoes: Vec<_> = cfg.opcoes.into_iter().map(|o| o.acao).collect();
        assert_eq!(
            acoes,
            [
                Acao::Abrir("a".into()),
                Acao::Comando("b".into()),
                Acao::Teclas("c".into()),
                Acao::Texto("d".into()),
                Acao::Sair
            ]
        );
    }

    #[test]
    fn valores_e_campos_desconhecidos_viram_avisos() {
        let (cfg, avisos) = ler(r#"{"botao": "X3", "cor": 1, "opcoes": [{"nome": "A", "icon": "x", "tipo": "abrr"}]}"#);
        assert_eq!(cfg.botao, Botao::X1);
        assert_eq!(cfg.opcoes[0].acao, Acao::Desconhecida("abrr".into()));
        assert_eq!(avisos.len(), 4, "{avisos:?}");
        assert!(avisos.iter().any(|a| a.contains("\"X3\"")));
        assert!(avisos.iter().any(|a| a.contains("\"cor\"")));
        assert!(avisos.iter().any(|a| a.contains("\"icon\"")));
        assert!(avisos.iter().any(|a| a.contains("\"abrr\"")));
    }

    #[test]
    fn botao_ignora_maiusculas() {
        assert_eq!(ler(r#"{"botao": "X2", "opcoes": [{"tipo": "sair"}]}"#).0.botao, Botao::X2);
    }

    #[test]
    fn raio_fica_dentro_do_limite() {
        let (cfg, avisos) = ler(r#"{"raio": 1000, "opcoes": [{"tipo": "sair"}]}"#);
        assert_eq!(cfg.raio, RAIO_MAX);
        assert_eq!(avisos.len(), 1);
        assert_eq!(ler(r#"{"raio": 10, "opcoes": [{"tipo": "sair"}]}"#).0.raio, RAIO_MIN);
    }

    #[test]
    fn sem_opcoes_usa_as_padrao() {
        let (cfg, avisos) = ler(r#"{"opcoes": []}"#);
        assert_eq!(cfg.opcoes.len(), padrao().opcoes.len());
        assert_eq!(avisos.len(), 1);
    }

    #[test]
    fn aceita_bom() {
        assert!(interpretar("\u{feff}{\"opcoes\": []}").is_ok());
    }

    #[test]
    fn json_invalido_e_erro() {
        assert!(interpretar("{\"opcoes\": [").is_err());
        assert!(interpretar(r#"{"raio": "grande"}"#).is_err());
    }

    #[test]
    fn decodifica_latin1() {
        assert_eq!(decodificar("Opção".as_bytes().to_vec()), ("Opção".into(), true));
        assert_eq!(decodificar(b"Op\xe7\xe3o".to_vec()), ("Opção".into(), false));
    }
}
