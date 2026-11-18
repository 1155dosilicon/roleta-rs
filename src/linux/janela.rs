//! Janela da roleta no X11 e o loop principal.
//!
//! - O botão lateral é pego com um grab passivo (XGrabButton) na janela raiz:
//!   enquanto ele está apertado, o X manda os eventos só para nós (o navegador
//!   não faz "voltar/avançar").
//! - No modo clique a roleta pega o ponteiro (XGrabPointer) até escolher ou
//!   cancelar, e engole esse clique.
//! - A janela é ARGB de 32 bits, sem moldura (override-redirect), sem foco e
//!   transparente para cliques; a transparência precisa de compositor.
//! - Uma thread lê os eventos do X e a bandeja manda comandos; tudo chega num
//!   canal só, e o loop acorda a cada quadro enquanto a roleta está na tela.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Instant;

use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::errors::ReplyOrIdError;
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use super::{acoes, bandeja};
use crate::config::{self, Botao, Config};
use crate::log;
use crate::roleta::{self, Roleta, Tick};

pub enum Msg {
    X(Event),
    ErroX(String),
    Editar,
    Reiniciar,
    Sair,
}

/// Máscara dos botões 1 a 5 no `state` dos eventos (o X não tem para 8/9).
const BOTOES: u16 = 0x1F00;

fn mascara_botao(b: u8) -> u16 {
    if (1..=5).contains(&b) { 0x80 << b } else { 0 }
}

struct App {
    conn: Arc<RustConnection>,
    raiz: Window,
    janela: Window,
    tela: (i32, i32),
    gc: Gcontext,
    lsb: bool,
    botoes: Vec<u8>,
    r: Roleta,
    buf: Vec<u8>,
    ultimo_ponto: roleta::Ponto,
    grab_ativo: bool,
    proximo_tick: Instant,
    acao: Option<(Instant, usize)>,
    reiniciar: bool,
    sair: bool,
}

fn botoes_de(botao: Botao) -> Vec<u8> {
    match botao {
        Botao::X1 => vec![8],
        Botao::X2 => vec![9],
        Botao::Ambos => vec![8, 9],
    }
}

fn atomo(conn: &RustConnection, nome: &str) -> Result<Atom, ReplyOrIdError> {
    Ok(conn.intern_atom(false, nome.as_bytes())?.reply()?.atom)
}

/// Visual TrueColor de 32 bits (com canal alfa).
fn visual_argb(tela: &Screen) -> Option<Visualid> {
    tela.allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR)
        .map(|v| v.visual_id)
}

impl App {
    fn criar(conn: Arc<RustConnection>, tela_n: usize, cfg: Config) -> Result<App, String> {
        let erro = |e: ReplyOrIdError| e.to_string();
        let tela = conn.setup().roots[tela_n].clone();
        let raiz = tela.root;
        let lsb = conn.setup().image_byte_order == ImageOrder::LSB_FIRST;
        let visual = visual_argb(&tela).ok_or("o X não tem visual de 32 bits (ARGB)")?;

        let cmap = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_colormap(ColormapAlloc::NONE, cmap, raiz, visual).map_err(|e| e.to_string())?;
        let janela = conn.generate_id().map_err(|e| e.to_string())?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .override_redirect(1)
            .colormap(cmap)
            .event_mask(EventMask::EXPOSURE);
        conn.create_window(32, janela, raiz, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, visual, &aux)
            .map_err(|e| e.to_string())?;

        // nome, classe e tipo (o compositor trata como menu: sem moldura nem foco)
        let _ = conn.change_property8(PropMode::REPLACE, janela, AtomEnum::WM_NAME, AtomEnum::STRING, b"Roleta");
        let _ =
            conn.change_property8(PropMode::REPLACE, janela, AtomEnum::WM_CLASS, AtomEnum::STRING, b"roleta\0Roleta\0");
        let tipo = atomo(&conn, "_NET_WM_WINDOW_TYPE").map_err(erro)?;
        let menu = atomo(&conn, "_NET_WM_WINDOW_TYPE_POPUP_MENU").map_err(erro)?;
        let _ = conn.change_property32(PropMode::REPLACE, janela, tipo, AtomEnum::ATOM, &[menu]);
        // cliques passam direto pela janela (quem recebe é o grab)
        let _ = conn.shape_rectangles(shape::SO::SET, shape::SK::INPUT, ClipOrdering::UNSORTED, janela, 0, 0, &[]);

        let gc = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_gc(gc, janela, &CreateGCAux::new()).map_err(|e| e.to_string())?;

        let botoes = botoes_de(cfg.botao);
        for &b in &botoes {
            let r = conn
                .grab_button(
                    false,
                    raiz,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    NONE,
                    ButtonIndex::from(b),
                    ModMask::ANY,
                )
                .map_err(|e| e.to_string())?
                .check();
            if let Err(e) = r {
                return Err(format!("não deu para pegar o botão {b} do mouse (outro programa já usa?): {e:?}"));
            }
        }
        conn.flush().map_err(|e| e.to_string())?;

        Ok(App {
            conn,
            raiz,
            janela,
            tela: (i32::from(tela.width_in_pixels), i32::from(tela.height_in_pixels)),
            gc,
            lsb,
            botoes,
            r: Roleta::new(cfg),
            buf: Vec::new(),
            ultimo_ponto: (0, 0),
            grab_ativo: false,
            proximo_tick: Instant::now(),
            acao: None,
            reiniciar: false,
            sair: false,
        })
    }

    /// Posição do cursor e botões apertados (1 a 5).
    fn ponteiro(&mut self) -> (roleta::Ponto, u16) {
        match self.conn.query_pointer(self.raiz).map(|c| c.reply()) {
            Ok(Ok(r)) => {
                self.ultimo_ponto = (i32::from(r.root_x), i32::from(r.root_y));
                (self.ultimo_ponto, u16::from(r.mask) & BOTOES)
            }
            _ => (self.ultimo_ponto, 0),
        }
    }

    /// Escala da tela pelo Xft.dpi (o mesmo que o XFCE usa para as fontes).
    fn escala(&self) -> f32 {
        let dpi = self
            .conn
            .get_property(false, self.raiz, AtomEnum::RESOURCE_MANAGER, AtomEnum::STRING, 0, 1 << 20)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| {
                String::from_utf8_lossy(&r.value)
                    .lines()
                    .find_map(|l| l.strip_prefix("Xft.dpi:").and_then(|v| v.trim().parse::<f32>().ok()))
            })
            .unwrap_or(96.0);
        (dpi / 96.0).clamp(0.5, 4.0)
    }

    /// Monitor que contém o ponto (RandR); senão a tela inteira.
    fn monitor(&self, p: roleta::Ponto) -> roleta::Retangulo {
        let inteira = (0, 0, self.tela.0, self.tela.1);
        let Ok(Ok(r)) = self.conn.randr_get_monitors(self.raiz, true).map(|c| c.reply()) else {
            return inteira;
        };
        let rets: Vec<roleta::Retangulo> = r
            .monitors
            .iter()
            .map(|m| {
                let (x, y) = (i32::from(m.x), i32::from(m.y));
                (x, y, x + i32::from(m.width), y + i32::from(m.height))
            })
            .collect();
        rets.iter()
            .copied()
            .find(|&(l, t, r, b)| p.0 >= l && p.0 < r && p.1 >= t && p.1 < b)
            .or_else(|| rets.first().copied())
            .unwrap_or(inteira)
    }

    fn abrir(&mut self) {
        let (p, _) = self.ponteiro();
        let (mon, escala) = (self.monitor(p), self.escala());
        let estava_visivel = self.r.visivel;
        if !self.r.abrir(p, mon, escala) {
            return;
        }
        let lado = self.r.lado();
        let aux = ConfigureWindowAux::new()
            .x(self.r.canto.0)
            .y(self.r.canto.1)
            .width(lado)
            .height(lado)
            .stack_mode(StackMode::ABOVE);
        let _ = self.conn.configure_window(self.janela, &aux);
        let _ = self.conn.map_window(self.janela);
        self.pintar();
        if !estava_visivel {
            self.proximo_tick = Instant::now() + roleta::INTERVALO;
        }
    }

    fn agendar(&mut self, idx: Option<usize>) {
        if let Some(i) = idx {
            self.acao = Some((Instant::now() + roleta::ATRASO_ACAO, i));
        }
    }

    fn soltar(&mut self) {
        let (p, _) = self.ponteiro();
        let idx = self.r.soltar(p);
        self.agendar(idx);
        if self.r.modo_clique && !self.grab_ativo {
            self.pegar_ponteiro();
        }
    }

    /// Modo clique: os próximos cliques vêm para a roleta, não para as janelas.
    fn pegar_ponteiro(&mut self) {
        let r = self
            .conn
            .grab_pointer(
                false,
                self.raiz,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                NONE,
                NONE,
                CURRENT_TIME,
            )
            .ok()
            .and_then(|c| c.reply().ok());
        match r {
            Some(r) if r.status == GrabStatus::SUCCESS => self.grab_ativo = true,
            r => {
                log::escrever(&format!("Não deu para pegar o ponteiro ({:?}); fechando.", r.map(|r| r.status)));
                self.r.fechar();
            }
        }
    }

    fn soltar_ponteiro(&mut self) {
        let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        let _ = self.conn.flush();
        self.grab_ativo = false;
    }

    fn evento(&mut self, ev: Event) {
        match ev {
            Event::ButtonPress(e) => {
                if std::env::var_os("ROLETA_DEBUG").is_some() {
                    log::escrever(&format!("aperta {}", e.detail));
                }
                if self.botoes.contains(&e.detail) {
                    self.abrir();
                } else if self.r.modo_clique && self.r.aberta {
                    match e.detail {
                        1 => {
                            let (p, _) = self.ponteiro();
                            let idx = self.r.confirmar(p);
                            self.agendar(idx);
                        }
                        3 => self.r.fechar(),
                        _ => {}
                    }
                }
            }
            Event::ButtonRelease(e) => {
                if std::env::var_os("ROLETA_DEBUG").is_some() {
                    log::escrever(&format!("solta {}", e.detail));
                }
                if self.botoes.contains(&e.detail) {
                    self.soltar();
                }
                // engole até soltar o botão do clique que fechou a roleta
                let resto = u16::from(e.state) & BOTOES & !mascara_botao(e.detail);
                if self.grab_ativo && !self.r.modo_clique && resto == 0 {
                    self.soltar_ponteiro();
                }
            }
            Event::Expose(e) if e.window == self.janela && self.r.visivel => self.pintar(),
            Event::Error(e) => log::escrever(&format!("Erro do X: {e:?}")),
            _ => {}
        }
    }

    fn tick(&mut self) {
        let (p, apertados) = self.ponteiro();
        match self.r.tick(p) {
            Tick::Pintar => self.pintar(),
            Tick::Esconder => {
                let _ = self.conn.unmap_window(self.janela);
                let _ = self.conn.flush();
            }
            Tick::Nada => {}
        }
        // fechou sozinha (cursor saiu da roleta) no modo clique
        if self.grab_ativo && !self.r.modo_clique && apertados == 0 {
            self.soltar_ponteiro();
        }
    }

    fn pintar(&mut self) {
        let alfa = u32::from(self.r.alfa());
        let lado = self.r.lado();
        let pm = self.r.quadro();
        if pm.width() != lado || pm.height() != lado {
            return;
        }
        // RGBA pré-multiplicado (tiny-skia) -> pixel ARGB de 32 bits do X,
        // também pré-multiplicado, já com a opacidade geral da animação
        self.buf.resize(pm.data().len(), 0);
        let m = |v: u8| ((u32::from(v) * alfa + 127) / 255) as u8;
        for (d, s) in self.buf.as_chunks_mut::<4>().0.iter_mut().zip(pm.data().as_chunks::<4>().0) {
            let (r, g, b, a) = (m(s[0]), m(s[1]), m(s[2]), m(s[3]));
            if self.lsb {
                d.copy_from_slice(&[b, g, r, a]);
            } else {
                d.copy_from_slice(&[a, r, g, b]);
            }
        }
        // em faixas que caibam no tamanho máximo de requisição do servidor
        let linha = lado as usize * 4;
        let max = self.conn.maximum_request_bytes().saturating_sub(64);
        let por_vez = (max / linha).clamp(1, lado as usize);
        for (i, faixa) in self.buf.chunks(por_vez * linha).enumerate() {
            let h = (faixa.len() / linha) as u16;
            let y = (i * por_vez) as i16;
            let _ =
                self.conn.put_image(ImageFormat::Z_PIXMAP, self.janela, self.gc, lado as u16, h, 0, y, 0, 32, faixa);
        }
        let _ = self.conn.flush();
    }

    fn executar_pendente(&mut self) {
        let Some((_, i)) = self.acao.take() else { return };
        let Some(op) = self.r.opcao(i) else { return };
        if acoes::executar(op, &self.conn) {
            self.sair = true;
        }
    }

    /// Quanto esperar pelo próximo evento (`None` = até chegar algum).
    fn espera(&self) -> Option<std::time::Duration> {
        let agora = Instant::now();
        let tick = self.r.visivel.then_some(self.proximo_tick);
        let acao = self.acao.map(|(t, _)| t);
        [tick, acao].into_iter().flatten().min().map(|t| t.saturating_duration_since(agora))
    }

    fn temporizadores(&mut self) {
        let agora = Instant::now();
        if self.r.visivel && agora >= self.proximo_tick {
            self.proximo_tick = agora + roleta::INTERVALO;
            self.tick();
        }
        if self.acao.is_some_and(|(t, _)| agora >= t) {
            self.executar_pendente();
        }
    }

    fn encerrar(&mut self) {
        if self.grab_ativo {
            self.soltar_ponteiro();
        }
        for &b in &self.botoes {
            let _ = self.conn.ungrab_button(ButtonIndex::from(b), self.raiz, ModMask::ANY);
        }
        let _ = self.conn.destroy_window(self.janela);
        let _ = self.conn.flush();
    }
}

fn ler_eventos(conn: Arc<RustConnection>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        loop {
            match conn.wait_for_event() {
                Ok(ev) => {
                    if tx.send(Msg::X(ev)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Msg::ErroX(e.to_string()));
                    break;
                }
            }
        }
    });
}

fn laco(app: &mut App, rx: &Receiver<Msg>) {
    while !app.sair {
        let msg = match app.espera() {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(d) => rx.recv_timeout(d),
        };
        match msg {
            Ok(Msg::X(ev)) => app.evento(ev),
            Ok(Msg::ErroX(e)) => {
                log::escrever(&format!("Conexão com o X caiu: {e}"));
                return;
            }
            Ok(Msg::Editar) => acoes::abrir(&config::caminho().to_string_lossy()),
            Ok(Msg::Reiniciar) => {
                app.reiniciar = true;
                app.sair = true;
            }
            Ok(Msg::Sair) => app.sair = true,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        app.temporizadores();
    }
}

/// Roda a roleta até sair. Devolve `true` se pediu para reiniciar.
pub fn rodar(cfg: Config) -> bool {
    let t0 = Instant::now();
    let (conn, tela_n) = match x11rb::connect(None) {
        Ok(c) => c,
        Err(e) => {
            log::escrever(&format!("ERRO: sem conexão com o X11 ({e}). A roleta no Linux precisa de uma sessão X11."));
            return false;
        }
    };
    let conn = Arc::new(conn);
    let mut app = match App::criar(conn.clone(), tela_n, cfg) {
        Ok(a) => a,
        Err(e) => {
            log::escrever(&format!("ERRO: {e}"));
            return false;
        }
    };
    // já deixa os desenhos prontos
    let escala = app.escala();
    app.r.desenho.preparar(escala);

    let (tx, rx) = mpsc::channel();
    ler_eventos(conn, tx.clone());
    let _bandeja = bandeja::iniciar(tx);

    let nomes = match app.r.cfg.botao {
        Botao::X1 => "lateral de trás (X1, botão 8)",
        Botao::X2 => "lateral da frente (X2, botão 9)",
        Botao::Ambos => "laterais (X1 e X2, botões 8 e 9)",
    };
    log::escrever(&format!("Roleta ativa! Segure o botão {nomes}. (pronta em {} ms)", t0.elapsed().as_millis()));

    laco(&mut app, &rx);
    app.encerrar();
    app.reiniciar
}
