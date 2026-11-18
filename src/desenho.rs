//! Desenho da roleta, seguindo o visual de assets/desenho/roleta_completa.svg
//! (desenhado num canvas de 512 com raio externo 230). A geometria é desenhada
//! em código (funciona com qualquer número de opções); os ícones vêm dos SVGs.

use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;

use resvg::tiny_skia as sk;
use resvg::usvg;

use crate::config::Config;
use crate::log;

/// Espaço em volta da roleta para a sombra (px lógicos).
pub const MARGEM: f32 = 26.0;

const ICONES: &[(&str, &str, &str)] = &[
    ("pasta", include_str!("../assets/icones/pasta.svg"), include_str!("../assets/icones/pasta_ativo.svg")),
    ("print", include_str!("../assets/icones/print.svg"), include_str!("../assets/icones/print_ativo.svg")),
    ("tarefas", include_str!("../assets/icones/tarefas.svg"), include_str!("../assets/icones/tarefas_ativo.svg")),
    ("chrome", include_str!("../assets/icones/chrome.svg"), include_str!("../assets/icones/chrome_ativo.svg")),
    ("terminal", include_str!("../assets/icones/terminal.svg"), include_str!("../assets/icones/terminal_ativo.svg")),
];

/// Códigos de glifo da versão antiga -> ícone novo equivalente.
const LEGADO: &[(&str, &str)] = &[
    ("E8B7", "pasta"),
    ("E722", "print"),
    ("E756", "terminal"),
    ("E774", "chrome"),
    ("E9D9", "tarefas"),
];

// paleta do desenho
const ARCOIRIS: &[(f32, u32)] = &[
    (0.00, 0xFF3CAC),
    (0.28, 0x784BA0),
    (0.56, 0x2B86C5),
    (0.82, 0x00F5D4),
    (1.00, 0xFEE440),
];
const FATIA: &[(f32, u32)] = &[(0.0, 0xFF3CAC), (0.5, 0x2B86C5), (1.0, 0x00F5D4)];
const FUNDO_CENTRO: u32 = 0x0F172A;
const FUNDO_BORDA: u32 = 0x1E293B;
const PONTO: u32 = 0xE2E8F0;
const TEXTO: &str = "#CBD5E1";
const TEXTO_ATIVO: &str = "#F8FAFC";

enum FonteIcone {
    Svg(String, String),
    Glifo(char),
    Nenhum,
}

fn resolver_icone(icone: &str, dados: &Path) -> FonteIcone {
    let s = icone.trim();
    if s.is_empty() {
        return FonteIcone::Nenhum;
    }
    let nome = LEGADO
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(s))
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| s.to_lowercase());
    if let Some((_, n, a)) = ICONES.iter().find(|(n, _, _)| *n == nome) {
        return FonteIcone::Svg(n.to_string(), a.to_string());
    }
    if nome.ends_with(".svg") {
        let p = Path::new(s);
        let p = if p.is_absolute() { p.to_path_buf() } else { dados.join(p) };
        match std::fs::read_to_string(&p) {
            Ok(normal) => {
                let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                let ativo = std::fs::read_to_string(p.with_file_name(format!("{stem}_ativo.svg")))
                    .unwrap_or_else(|_| normal.clone());
                return FonteIcone::Svg(normal, ativo);
            }
            Err(e) => {
                log::escrever(&format!("Ícone {} não encontrado: {e}", p.display()));
                return FonteIcone::Nenhum;
            }
        }
    }
    match u32::from_str_radix(s, 16).ok().and_then(char::from_u32) {
        Some(c) => FonteIcone::Glifo(c),
        None => {
            log::escrever(&format!("Ícone desconhecido: {s:?}"));
            FonteIcone::Nenhum
        }
    }
}

fn cor(hex: u32, a: f32) -> sk::Color {
    sk::Color::from_rgba8(
        (hex >> 16) as u8,
        (hex >> 8) as u8,
        hex as u8,
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn degrade(p0: (f32, f32), p1: (f32, f32), stops: &[(f32, u32)], a: f32) -> sk::Shader<'static> {
    let paradas = stops.iter().map(|&(o, c)| sk::GradientStop::new(o, cor(c, a))).collect();
    sk::LinearGradient::new(
        sk::Point::from_xy(p0.0, p0.1),
        sk::Point::from_xy(p1.0, p1.1),
        paradas,
        sk::SpreadMode::Pad,
        sk::Transform::identity(),
    )
    .unwrap_or(sk::Shader::SolidColor(cor(stops[0].1, a)))
}

fn pincel(shader: sk::Shader<'static>) -> sk::Paint<'static> {
    sk::Paint { shader, anti_alias: true, ..Default::default() }
}

fn solido(c: sk::Color) -> sk::Paint<'static> {
    let mut p = sk::Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn circulo(r: f32) -> sk::Path {
    sk::PathBuilder::from_circle(0.0, 0.0, r.max(0.1)).unwrap()
}

/// Arco de circunferência (centro na origem) com curvas de Bézier.
fn arco(pb: &mut sk::PathBuilder, r: f32, a0: f32, a1: f32, mover: bool) {
    let n = ((a1 - a0).abs() / (PI / 2.0)).ceil().max(1.0) as usize;
    let d = (a1 - a0) / n as f32;
    let k = 4.0 / 3.0 * (d / 4.0).tan();
    let (x0, y0) = (r * a0.cos(), r * a0.sin());
    if mover {
        pb.move_to(x0, y0);
    } else {
        pb.line_to(x0, y0);
    }
    let mut a = a0;
    for _ in 0..n {
        let b = a + d;
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        pb.cubic_to(
            r * (ca - k * sa),
            r * (sa + k * ca),
            r * (cb + k * sb),
            r * (sb - k * cb),
            r * cb,
            r * sb,
        );
        a = b;
    }
}

fn fatia(r_in: f32, r_out: f32, a0: f32, a1: f32) -> sk::Path {
    let mut pb = sk::PathBuilder::new();
    arco(&mut pb, r_out, a0, a1, true);
    arco(&mut pb, r_in, a1, a0, false);
    pb.close();
    pb.finish().unwrap()
}

fn escapar(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Medidas da roleta em px lógicos (antes da escala do monitor).
pub struct Geometria {
    pub r: f32,
    pub r_furo: f32,
    r_hub: f32,
    k: f32,
    tam_icone: f32,
    tam_texto: f32,
}

impl Geometria {
    fn new(r: f32) -> Self {
        let k = r / 230.0; // o desenho original tem raio 230
        Geometria {
            r,
            r_furo: 68.0 * k,
            r_hub: 56.0 * k,
            k,
            tam_icone: (64.0 * k * 1.1).clamp(30.0, 80.0),
            tam_texto: (13.0 * k).max(11.5),
        }
    }
}

#[derive(Default)]
struct Rotulo {
    icone: [Option<sk::Pixmap>; 2], // normal, ativo
    texto: [Option<sk::Pixmap>; 2],
}

pub struct Estado<'a> {
    pub prog: f32,
    pub hover: &'a [f32],
    pub esmaecer: f32,
    pub cancelar: f32,
}

pub struct Desenho {
    pub geo: Geometria,
    opcoes: Vec<(String, FonteIcone)>,
    opt: usvg::Options<'static>,
    escala: f32,
    rotulos: Vec<Rotulo>,
    quadro: sk::Pixmap,
}

impl Desenho {
    pub fn new(cfg: &Config, dados: &Path) -> Self {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family(if cfg!(windows) { "Segoe UI" } else { "Inter" });
        let opt = usvg::Options { fontdb: Arc::new(db), ..Default::default() };
        let opcoes = cfg
            .opcoes
            .iter()
            .map(|o| (o.nome.clone(), resolver_icone(&o.icone, dados)))
            .collect();
        Desenho {
            geo: Geometria::new(cfg.raio),
            opcoes,
            opt,
            escala: 0.0,
            rotulos: Vec::new(),
            quadro: sk::Pixmap::new(1, 1).unwrap(),
        }
    }

    pub fn lado_px(&self) -> u32 {
        (2.0 * (self.geo.r + MARGEM) * self.escala).ceil() as u32
    }

    /// Prepara os desenhos para a escala (DPI) do monitor. Só refaz se mudou.
    pub fn preparar(&mut self, escala: f32) {
        if (escala - self.escala).abs() < 0.001 && !self.rotulos.is_empty() {
            return;
        }
        self.escala = escala;
        let lado = self.lado_px();
        self.quadro = sk::Pixmap::new(lado, lado).unwrap();
        let ti = (self.geo.tam_icone * escala).round() as u32;
        let fs = self.geo.tam_texto * escala;
        let larg = self.geo.r * 0.66 * escala;
        let mut rotulos = Vec::new();
        for (nome, ic) in &self.opcoes {
            let mut r = Rotulo::default();
            match ic {
                FonteIcone::Svg(n, a) => {
                    r.icone = [self.svg_em_pixmap(n, ti), self.svg_em_pixmap(a, ti)];
                }
                FonteIcone::Glifo(c) => {
                    r.icone = [self.glifo(*c, ti, "#E2E8F0"), self.glifo(*c, ti, "#FFFFFF")];
                }
                FonteIcone::Nenhum => {}
            }
            let linhas = self.quebrar(nome, fs, larg);
            r.texto = [self.texto(&linhas, fs, TEXTO), self.texto(&linhas, fs, TEXTO_ATIVO)];
            rotulos.push(r);
        }
        self.rotulos = rotulos;
    }

    fn arvore(&self, svg: &str) -> Option<usvg::Tree> {
        match usvg::Tree::from_str(svg, &self.opt) {
            Ok(t) => Some(t),
            Err(e) => {
                log::escrever(&format!("SVG inválido: {e}"));
                None
            }
        }
    }

    fn svg_em_pixmap(&self, svg: &str, tam: u32) -> Option<sk::Pixmap> {
        let t = self.arvore(svg)?;
        let sz = t.size();
        let s = (tam as f32 / sz.width()).min(tam as f32 / sz.height());
        let dx = (tam as f32 - sz.width() * s) / 2.0;
        let dy = (tam as f32 - sz.height() * s) / 2.0;
        let mut pm = sk::Pixmap::new(tam.max(1), tam.max(1))?;
        resvg::render(&t, sk::Transform::from_row(s, 0.0, 0.0, s, dx, dy), &mut pm.as_mut());
        Some(pm)
    }

    fn glifo(&self, c: char, tam: u32, cor: &str) -> Option<sk::Pixmap> {
        let t = tam as f32;
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{t}" height="{t}" viewBox="0 0 {t} {t}"><text x="{}" y="{}" font-family="Segoe Fluent Icons, Segoe MDL2 Assets, Symbols Nerd Font, JetBrainsMono Nerd Font Mono, JetBrainsMono Nerd Font" font-size="{}" fill="{cor}" text-anchor="middle">&#x{:X};</text></svg>"#,
            t / 2.0,
            t * 0.72,
            t * 0.55,
            c as u32
        );
        self.svg_em_pixmap(&svg, tam)
    }

    fn svg_texto(&self, linhas: &[String], fs: f32, cor: &str, w: f32) -> String {
        let lh = fs * 1.22;
        let mut s = String::new();
        for (i, l) in linhas.iter().enumerate() {
            s += &format!(
                r#"<text x="{}" y="{}" text-anchor="middle">{}</text>"#,
                w / 2.0,
                fs + i as f32 * lh,
                escapar(l)
            );
        }
        let h = (lh * linhas.len() as f32 + fs * 0.35).ceil();
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><g font-family="Segoe UI, Inter, Noto Sans, DejaVu Sans, sans-serif" font-weight="600" font-size="{fs}" fill="{cor}">{s}</g></svg>"#
        )
    }

    fn largura(&self, txt: &str, fs: f32) -> f32 {
        let svg = self.svg_texto(&[txt.to_string()], fs, "#000", 2000.0);
        self.arvore(&svg)
            .and_then(|t| t.root().abs_bounding_box().map(|b| b.width()))
            .unwrap_or(fs * 0.6 * txt.chars().count() as f32)
    }

    fn quebrar(&self, nome: &str, fs: f32, larg: f32) -> Vec<String> {
        let mut linhas: Vec<String> = Vec::new();
        for palavra in nome.split_whitespace() {
            if let Some(ult) = linhas.last_mut() {
                let teste = format!("{ult} {palavra}");
                if self.largura(&teste, fs) <= larg {
                    *ult = teste;
                    continue;
                }
            }
            linhas.push(palavra.to_string());
        }
        if linhas.is_empty() {
            linhas.push(String::new());
        }
        linhas
    }

    fn texto(&self, linhas: &[String], fs: f32, cor: &str) -> Option<sk::Pixmap> {
        let w = linhas.iter().map(|l| self.largura(l, fs)).fold(0.0, f32::max).ceil() + 6.0;
        let svg = self.svg_texto(linhas, fs, cor, w);
        let t = self.arvore(&svg)?;
        let sz = t.size();
        let mut pm = sk::Pixmap::new(sz.width().ceil() as u32, sz.height().ceil() as u32)?;
        resvg::render(&t, sk::Transform::identity(), &mut pm.as_mut());
        Some(pm)
    }

    /// Desenha um quadro. `est.prog` 0..1 é a animação de abrir/fechar.
    pub fn desenhar(&mut self, est: &Estado) -> &sk::Pixmap {
        let g = &self.geo;
        let e = self.escala;
        let lado = self.quadro.width() as f32;
        let n = self.opcoes.len().max(1);
        let larg = 2.0 * PI / n as f32;
        let s = 0.82 + 0.18 * est.prog;
        let ts = sk::Transform::from_row(e * s, 0.0, 0.0, e * s, lado / 2.0, lado / 2.0);
        let q = &mut self.quadro;
        q.fill(sk::Color::TRANSPARENT);
        let (r, k) = (g.r, g.k);

        // sombra
        let rs = r + MARGEM - 4.0;
        if let Some(sh) = sk::RadialGradient::new(
            sk::Point::from_xy(0.0, 5.0),
            0.0,
            sk::Point::from_xy(0.0, 5.0),
            rs,
            vec![
                sk::GradientStop::new(0.0, cor(0, 0.55)),
                sk::GradientStop::new((r - 6.0) / rs, cor(0, 0.47)),
                sk::GradientStop::new(1.0, cor(0, 0.0)),
            ],
            sk::SpreadMode::Pad,
            sk::Transform::identity(),
        ) {
            let c = sk::PathBuilder::from_circle(0.0, 5.0, rs).unwrap();
            q.fill_path(&c, &pincel(sh), sk::FillRule::Winding, ts, None);
        }

        // anel de vidro (com o furo central)
        let mut pb = sk::PathBuilder::new();
        pb.push_circle(0.0, 0.0, r);
        pb.push_circle(0.0, 0.0, g.r_furo);
        let anel = pb.finish().unwrap();
        if let Some(sh) = sk::RadialGradient::new(
            sk::Point::zero(),
            0.0,
            sk::Point::zero(),
            r,
            vec![
                sk::GradientStop::new(0.25, cor(FUNDO_CENTRO, 0.95)),
                sk::GradientStop::new(1.0, cor(FUNDO_BORDA, 0.92)),
            ],
            sk::SpreadMode::Pad,
            sk::Transform::identity(),
        ) {
            q.fill_path(&anel, &pincel(sh), sk::FillRule::EvenOdd, ts, None);
        }

        // fatias ativas (com transição suave)
        for i in 0..n {
            let h = est.hover.get(i).copied().unwrap_or(0.0);
            if h < 0.01 {
                continue;
            }
            let c = -PI / 2.0 + i as f32 * larg;
            let p = fatia(g.r_furo, r, c - larg / 2.0, c + larg / 2.0);
            let b = p.bounds();
            let sh = degrade((b.left(), b.top()), (b.right(), b.bottom()), FATIA, 0.45 * h);
            q.fill_path(&p, &pincel(sh), sk::FillRule::Winding, ts, None);
        }

        // divisórias
        if n > 1 {
            let mut pb = sk::PathBuilder::new();
            for i in 0..n {
                let a = -PI / 2.0 + (i as f32 - 0.5) * larg;
                pb.move_to(g.r_furo * a.cos(), g.r_furo * a.sin());
                pb.line_to(r * a.cos(), r * a.sin());
            }
            let st = sk::Stroke { width: (1.5 * k).max(1.0), line_cap: sk::LineCap::Round, ..Default::default() };
            q.stroke_path(&pb.finish().unwrap(), &solido(cor(0xFFFFFF, 0.18)), &st, ts, None);
        }

        // contornos arco-íris
        for (raio, w) in [(r, (5.0 * k).max(2.2)), (g.r_furo, (3.5 * k).max(1.6))] {
            let sh = degrade((-raio, -raio), (raio, raio), ARCOIRIS, 1.0);
            let st = sk::Stroke { width: w, ..Default::default() };
            q.stroke_path(&circulo(raio), &pincel(sh), &st, ts, None);
        }

        // miolo (soltar aqui = cancelar)
        let hub = circulo(g.r_hub);
        q.fill_path(&hub, &solido(cor(FUNDO_CENTRO, 1.0)), sk::FillRule::Winding, ts, None);
        let sh = degrade((-g.r_hub, -g.r_hub), (g.r_hub, g.r_hub), ARCOIRIS, 1.0);
        let st = sk::Stroke { width: (3.0 * k).max(1.5), ..Default::default() };
        q.stroke_path(&hub, &pincel(sh), &st, ts, None);
        let ponto = circulo((6.0 * k).max(2.5) * (0.85 + 0.15 * est.cancelar));
        let a_ponto = 0.45 + 0.55 * est.cancelar;
        q.fill_path(&ponto, &solido(cor(PONTO, a_ponto)), sk::FillRule::Winding, ts, None);

        // ícones + nomes
        let rm = (g.r_furo + r) / 2.0 + 2.0 * k;
        let nitido = s > 0.999;
        let pos = |x: f32, y: f32| {
            let (px, py) = (lado / 2.0 + e * s * x, lado / 2.0 + e * s * y);
            if nitido { (px.round(), py.round()) } else { (px, py) }
        };
        let qualidade = if nitido { sk::FilterQuality::Nearest } else { sk::FilterQuality::Bilinear };
        for (i, rot) in self.rotulos.iter().enumerate() {
            let h = est.hover.get(i).copied().unwrap_or(0.0);
            let base = 1.0 - 0.3 * est.esmaecer * (1.0 - h);
            let c = -PI / 2.0 + i as f32 * larg;
            let (cx, cy) = (rm * c.cos(), rm * c.sin());
            let ti = rot.icone[0].as_ref().map(|_| g.tam_icone).unwrap_or(0.0);
            let passo_ic = ti * 0.86; // o ícone tem folga embaixo no desenho
            let th = rot.texto[0].as_ref().map(|p| p.height() as f32 / e).unwrap_or(0.0);
            let topo = cy - (passo_ic + th) / 2.0;
            let pares = [
                (&rot.icone, cx - ti / 2.0, topo),
                (&rot.texto, cx - rot.texto[0].as_ref().map(|p| p.width() as f32 / e).unwrap_or(0.0) / 2.0, topo + passo_ic),
            ];
            for (pms, x, y) in pares {
                let (px, py) = pos(x, y);
                for (j, op) in [(0usize, base * (1.0 - h)), (1, base * h)] {
                    if op < 0.01 {
                        continue;
                    }
                    if let Some(pm) = &pms[j] {
                        let paint = sk::PixmapPaint { opacity: op, quality: qualidade, ..Default::default() };
                        q.draw_pixmap(0, 0, pm.as_ref(), &paint, sk::Transform::from_row(s, 0.0, 0.0, s, px, py), None);
                    }
                }
            }
        }
        &self.quadro
    }
}
