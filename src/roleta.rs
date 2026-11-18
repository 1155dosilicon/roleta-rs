//! Estado e animação da roleta, iguais em todos os sistemas. Cada plataforma
//! informa onde está o cursor e o monitor, e mostra o quadro que sai daqui.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use resvg::tiny_skia as sk;

use crate::config::{self, Config, Opcao};
use crate::desenho::{Desenho, Estado, MARGEM};

const TEMPO_CLIQUE: f32 = 0.25; // soltar antes disso no centro = modo clique
const T_ABRIR: f32 = 0.16;
const T_FECHAR: f32 = 0.10;

/// Intervalo entre quadros enquanto a roleta está na tela.
pub const INTERVALO: Duration = Duration::from_millis(10);
/// Pequeno atraso para a roleta sumir antes da ação.
pub const ATRASO_ACAO: Duration = Duration::from_millis(120);

/// Ponto na tela, em pixels físicos.
pub type Ponto = (i32, i32);
/// Retângulo do monitor: (esquerda, topo, direita, base), em pixels físicos.
pub type Retangulo = (i32, i32, i32, i32);

#[derive(Debug, PartialEq, Eq)]
pub enum Tick {
    /// Nada mudou na tela.
    Nada,
    Pintar,
    Esconder,
}

/// Fatia sob o ponto (dx, dy) relativo ao centro, em px lógicos. A fatia 0
/// fica em cima e as outras seguem no sentido horário. No furo do meio ou
/// fora da roleta: `None`.
fn fatia_em(dx: f32, dy: f32, n: usize, r_furo: f32, r: f32) -> Option<usize> {
    let dist = dx.hypot(dy);
    if n == 0 || dist < r_furo || dist > r {
        return None;
    }
    let larg = TAU / n as f32;
    let rel = (dy.atan2(dx) + FRAC_PI_2).rem_euclid(TAU);
    Some((rel / larg).round() as usize % n)
}

/// Centro da roleta perto de `p`, com o quadrado de meio-lado `meio` inteiro
/// dentro do monitor (se couber).
fn centro_no_monitor(p: Ponto, monitor: Retangulo, meio: i32) -> Ponto {
    let (l, t, r, b) = monitor;
    let cx = p.0.max(l + meio).min((r - meio).max(l + meio));
    let cy = p.1.max(t + meio).min((b - meio).max(t + meio));
    (cx, cy)
}

/// Aproxima `v` de `alvo` (fração `k`), encostando quando falta pouco para a
/// animação terminar de verdade. Devolve se mudou.
fn aproximar(v: &mut f32, alvo: f32, k: f32) -> bool {
    let antes = *v;
    *v += (alvo - *v) * k;
    if (alvo - *v).abs() < 0.002 {
        *v = alvo;
    }
    *v != antes
}

pub struct Roleta {
    pub cfg: Config,
    pub desenho: Desenho,

    pub aberta: bool,
    pub visivel: bool,
    /// Abriu com um clique rápido e espera clique (esquerdo escolhe, direito cancela).
    pub modo_clique: bool,
    t_abertura: Instant,
    t_fecho: Option<(Instant, f32)>,
    ultimo: Instant,
    prog: f32,
    selecionada: Option<usize>,
    hover: Vec<f32>,
    esmaecer: f32,
    cancelar: f32,
    pub centro: Ponto,
    pub canto: Ponto,
    pub escala: f32,
}

impl Roleta {
    pub fn new(cfg: Config) -> Self {
        let desenho = Desenho::new(&cfg, &config::pasta_dados());
        let n = cfg.opcoes.len();
        Roleta {
            cfg,
            desenho,
            aberta: false,
            visivel: false,
            modo_clique: false,
            t_abertura: Instant::now(),
            t_fecho: None,
            ultimo: Instant::now(),
            prog: 0.0,
            selecionada: None,
            hover: vec![0.0; n],
            esmaecer: 0.0,
            cancelar: 1.0,
            centro: (0, 0),
            canto: (0, 0),
            escala: 1.0,
        }
    }

    /// Ponto relativo ao centro, em px lógicos.
    fn relativo(&self, p: Ponto) -> (f32, f32) {
        ((p.0 - self.centro.0) as f32 / self.escala, (p.1 - self.centro.1) as f32 / self.escala)
    }

    /// No centro ou fora da roleta = nada selecionado (soltar cancela).
    pub fn indice_sob_cursor(&self, p: Ponto) -> Option<usize> {
        let (dx, dy) = self.relativo(p);
        let g = &self.desenho.geo;
        fatia_em(dx, dy, self.cfg.opcoes.len(), g.r_furo, g.r)
    }

    fn longe_demais(&self, p: Ponto) -> bool {
        let (dx, dy) = self.relativo(p);
        dx.hypot(dy) > self.desenho.geo.r + MARGEM - 4.0
    }

    /// Lado da janela (quadrada) em pixels físicos.
    pub fn lado(&self) -> u32 {
        self.desenho.lado_px()
    }

    /// Abre em volta do cursor, inteira dentro do monitor. Se já estava aberta,
    /// fecha e devolve `false`.
    pub fn abrir(&mut self, p: Ponto, monitor: Retangulo, escala: f32) -> bool {
        if self.aberta {
            // clicou de novo com ela aberta: fecha
            self.fechar();
            return false;
        }
        self.escala = escala;
        self.desenho.preparar(escala);
        let meio = (self.lado() / 2) as i32;
        let (cx, cy) = centro_no_monitor(p, monitor, meio);
        self.centro = (cx, cy);
        self.canto = (cx - meio, cy - meio);

        self.selecionada = None;
        self.hover.clear();
        self.hover.resize(self.cfg.opcoes.len(), 0.0);
        self.esmaecer = 0.0;
        self.cancelar = 1.0;
        self.aberta = true;
        self.modo_clique = false;
        self.t_fecho = None;
        self.t_abertura = Instant::now();
        self.ultimo = self.t_abertura;
        self.prog = 0.0;
        self.visivel = true;
        true
    }

    pub fn fechar(&mut self) {
        if !self.aberta {
            return;
        }
        self.aberta = false;
        self.modo_clique = false;
        self.t_fecho = Some((Instant::now(), self.prog));
    }

    /// Fecha e devolve a opção sob o cursor, que deve rodar depois de `ATRASO_ACAO`.
    pub fn confirmar(&mut self, p: Ponto) -> Option<usize> {
        let idx = self.indice_sob_cursor(p);
        self.fechar();
        idx
    }

    /// Botão lateral solto. Clique rápido no centro deixa a roleta aberta
    /// (`modo_clique`); senão confirma a opção sob o cursor.
    pub fn soltar(&mut self, p: Ponto) -> Option<usize> {
        if !self.aberta {
            return None;
        }
        let rapido = self.t_abertura.elapsed().as_secs_f32() < TEMPO_CLIQUE;
        if rapido && self.indice_sob_cursor(p).is_none() {
            self.modo_clique = true;
            None
        } else {
            self.confirmar(p)
        }
    }

    /// Avança a animação. Chamar a cada `INTERVALO` enquanto `visivel`.
    /// Devolve `Nada` quando o quadro ficaria igual ao anterior (roleta
    /// aberta e parada), para não redesenhar à toa.
    pub fn tick(&mut self, p: Ponto) -> Tick {
        if !self.visivel {
            return Tick::Nada;
        }
        let agora = Instant::now();
        let dt = (agora - self.ultimo).as_secs_f32().min(0.1);
        self.ultimo = agora;
        let prog_antes = self.prog;
        if self.aberta {
            let t = ((agora - self.t_abertura).as_secs_f32() / T_ABRIR).min(1.0);
            self.prog = 1.0 - (1.0 - t).powi(3);
            if self.longe_demais(p) {
                // saiu da roleta: fecha (e não fica presa se o botão for
                // solto em cima de uma janela que a roleta não enxerga)
                self.fechar();
                return Tick::Nada;
            }
            self.selecionada = self.indice_sob_cursor(p);
        } else if let Some((t0, p0)) = self.t_fecho {
            let t = ((agora - t0).as_secs_f32() / T_FECHAR).min(1.0);
            self.prog = p0 * (1.0 - t);
            if t >= 1.0 {
                self.prog = 0.0;
                self.visivel = false;
                return Tick::Esconder;
            }
        }

        // transições suaves do destaque (independente da taxa de quadros)
        let k = 1.0 - 0.68f32.powf(dt / 0.010);
        let mut mudou = self.prog != prog_antes;
        for (i, h) in self.hover.iter_mut().enumerate() {
            let alvo = if Some(i) == self.selecionada { 1.0 } else { 0.0 };
            mudou |= aproximar(h, alvo, k);
        }
        let tem = if self.selecionada.is_some() { 1.0 } else { 0.0 };
        mudou |= aproximar(&mut self.esmaecer, tem, k);
        mudou |= aproximar(&mut self.cancelar, 1.0 - tem, k);
        if mudou { Tick::Pintar } else { Tick::Nada }
    }

    /// Quadro atual: RGBA pré-multiplicado, `lado` x `lado`.
    pub fn quadro(&mut self) -> &sk::Pixmap {
        let est = Estado { prog: self.prog, hover: &self.hover, esmaecer: self.esmaecer, cancelar: self.cancelar };
        self.desenho.desenhar(&est)
    }

    /// Opacidade da janela inteira (aparece um pouco antes de crescer).
    pub fn alfa(&self) -> u8 {
        ((self.prog * 1.25).min(1.0) * 255.0).round() as u8
    }

    pub fn opcao(&self, i: usize) -> Option<&Opcao> {
        self.cfg.opcoes.get(i)
    }
}

/// Renderiza imagens de prévia (sem janela), para conferir o visual.
pub fn previa(pasta: &str) {
    let cfg = config::carregar();
    let mut d = Desenho::new(&cfg, &config::pasta_dados());
    let n = cfg.opcoes.len();
    let _ = std::fs::create_dir_all(pasta);
    for escala in [1.0f32, 1.5] {
        d.preparar(escala);
        let casos: Vec<(String, Vec<f32>, f32)> = vec![
            ("nenhuma".into(), vec![0.0; n], 0.0),
            ("sel0".into(), (0..n).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect(), 1.0),
            ("sel2".into(), (0..n).map(|i| if i == 2 { 1.0 } else { 0.0 }).collect(), 1.0),
        ];
        for (nome, hover, esm) in casos {
            let est = Estado { prog: 1.0, hover: &hover, esmaecer: esm, cancelar: 1.0 - esm };
            let pm = d.desenhar(&est);
            // fundo cinza-azulado para enxergar a transparência
            let Some(mut fundo) = sk::Pixmap::new(pm.width(), pm.height()) else { return };
            fundo.fill(sk::Color::from_rgba8(0x3a, 0x4a, 0x5a, 255));
            fundo.draw_pixmap(0, 0, pm.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), None);
            let arq = format!("{pasta}/previa_{nome}_{escala}x.png");
            if let Err(e) = fundo.save_png(&arq) {
                eprintln!("{arq}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fatia_de_cima_e_sentido_horario() {
        // 4 fatias, raio 100, furo 20; y cresce para baixo (tela)
        assert_eq!(fatia_em(0.0, -50.0, 4, 20.0, 100.0), Some(0)); // cima
        assert_eq!(fatia_em(50.0, 0.0, 4, 20.0, 100.0), Some(1)); // direita
        assert_eq!(fatia_em(0.0, 50.0, 4, 20.0, 100.0), Some(2)); // baixo
        assert_eq!(fatia_em(-50.0, 0.0, 4, 20.0, 100.0), Some(3)); // esquerda
    }

    #[test]
    fn fatia_centrada_no_angulo() {
        // com 4 fatias, a 0 vai de -45° a 45° em volta do topo
        assert_eq!(fatia_em(30.0, -50.0, 4, 20.0, 100.0), Some(0));
        assert_eq!(fatia_em(-30.0, -50.0, 4, 20.0, 100.0), Some(0));
        assert_eq!(fatia_em(60.0, -50.0, 4, 20.0, 100.0), Some(1));
    }

    #[test]
    fn furo_e_fora_nao_tem_fatia() {
        assert_eq!(fatia_em(0.0, 0.0, 4, 20.0, 100.0), None);
        assert_eq!(fatia_em(10.0, 10.0, 4, 20.0, 100.0), None);
        assert_eq!(fatia_em(0.0, -101.0, 4, 20.0, 100.0), None);
        assert_eq!(fatia_em(0.0, -50.0, 0, 20.0, 100.0), None);
    }

    #[test]
    fn uma_opcao_ocupa_tudo() {
        for (dx, dy) in [(0.0, -50.0), (50.0, 0.0), (0.0, 50.0), (-50.0, 0.0)] {
            assert_eq!(fatia_em(dx, dy, 1, 20.0, 100.0), Some(0));
        }
    }

    #[test]
    fn centro_fica_dentro_do_monitor() {
        let mon = (0, 0, 1920, 1080);
        assert_eq!(centro_no_monitor((500, 500), mon, 100), (500, 500));
        assert_eq!(centro_no_monitor((10, 10), mon, 100), (100, 100));
        assert_eq!(centro_no_monitor((1915, 1075), mon, 100), (1820, 980));
        // segundo monitor à direita
        assert_eq!(centro_no_monitor((1930, 5), (1920, 0, 3840, 1080), 100), (2020, 100));
        // monitor menor que a roleta: encosta no canto de cima/esquerda
        assert_eq!(centro_no_monitor((50, 50), (0, 0, 150, 150), 100), (100, 100));
    }

    #[test]
    fn aproximar_chega_no_alvo() {
        let mut v = 0.0;
        let mut passos = 0;
        while aproximar(&mut v, 1.0, 0.32) {
            passos += 1;
            assert!(passos < 100, "não convergiu");
        }
        assert_eq!(v, 1.0);
        assert!(!aproximar(&mut v, 1.0, 0.32));
    }
}
