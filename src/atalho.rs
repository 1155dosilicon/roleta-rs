//! Texto de atalho no formato do pynput, igual nos dois sistemas:
//! "<ctrl>+<shift>+s", "<alt>+<f4>", "<ctrl>++" (a tecla "+").

/// Separa o atalho em nomes de tecla já normalizados (sem `<>`, minúsculos).
/// Um "+" no fim depois do separador, ou sozinho, é a própria tecla "+".
pub fn teclas(valor: &str) -> Result<Vec<String>, String> {
    let v = valor.trim();
    let (corpo, mais) = if v == "+" {
        ("", true)
    } else if let Some(c) = v.strip_suffix("++") {
        (c, true)
    } else {
        (v, false)
    };
    let mut nomes: Vec<String> = Vec::new();
    if !corpo.is_empty() {
        for parte in corpo.split('+') {
            let n = normalizar(parte);
            if n.is_empty() {
                return Err(format!("atalho mal formado: {valor:?}"));
            }
            nomes.push(n);
        }
    }
    if mais {
        nomes.push("+".into());
    }
    if nomes.is_empty() {
        return Err("atalho vazio".into());
    }
    Ok(nomes)
}

fn normalizar(nome: &str) -> String {
    nome.trim().trim_start_matches('<').trim_end_matches('>').trim().to_lowercase()
}

/// "f1" a "f24" -> 1 a 24.
pub fn numero_f(nome: &str) -> Option<u8> {
    nome.strip_prefix('f').and_then(|x| x.parse::<u8>().ok()).filter(|f| (1..=24).contains(f))
}

/// Um caractere só ("a", "+", "ç").
pub fn caractere(nome: &str) -> Option<char> {
    let mut cs = nome.chars();
    let c = cs.next()?;
    cs.next().is_none().then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: &str) -> Vec<String> {
        teclas(v).unwrap_or_else(|e| panic!("{v:?}: {e}"))
    }

    #[test]
    fn separa_e_normaliza() {
        assert_eq!(t("<ctrl>+<shift>+s"), ["ctrl", "shift", "s"]);
        assert_eq!(t(" <Ctrl> + <ALT> + Delete "), ["ctrl", "alt", "delete"]);
        assert_eq!(t("<f5>"), ["f5"]);
        assert_eq!(t("a"), ["a"]);
    }

    #[test]
    fn tecla_mais() {
        assert_eq!(t("<ctrl>++"), ["ctrl", "+"]);
        assert_eq!(t("+"), ["+"]);
        assert_eq!(t("<ctrl>+<shift>++"), ["ctrl", "shift", "+"]);
    }

    #[test]
    fn mal_formado_e_erro() {
        for v in ["", "  ", "<ctrl>+", "+<ctrl>", "<ctrl>++<shift>", "<>"] {
            assert!(teclas(v).is_err(), "{v:?} deveria dar erro");
        }
    }

    #[test]
    fn teclas_f() {
        assert_eq!(numero_f("f1"), Some(1));
        assert_eq!(numero_f("f24"), Some(24));
        assert_eq!(numero_f("f0"), None);
        assert_eq!(numero_f("f25"), None);
        assert_eq!(numero_f("f"), None);
        assert_eq!(numero_f("fx"), None);
    }

    #[test]
    fn um_caractere() {
        assert_eq!(caractere("ç"), Some('ç'));
        assert_eq!(caractere("ab"), None);
        assert_eq!(caractere(""), None);
    }
}
