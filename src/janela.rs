//! Janelas (roleta transparente + janela de controle escondida), bandeja,
//! animação e a lógica de abrir/soltar/clicar.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForSystem, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::acoes::{self, wide};
use crate::config::{self, Config};
use crate::desenho::{Desenho, Estado, MARGEM};
use crate::hook;
use crate::log;

const WM_BANDEJA: u32 = WM_APP + 2;
const ID_ANIM: usize = 1;
const ID_ACAO: usize = 2;
const CMD_EDITAR: usize = 10;
const CMD_REINICIAR: usize = 11;
const CMD_SAIR: usize = 12;

const TEMPO_CLIQUE: f32 = 0.25; // soltar antes disso no centro = modo clique
const T_ABRIR: f32 = 0.16;
const T_FECHAR: f32 = 0.10;

static MSG_TASKBAR: AtomicU32 = AtomicU32::new(0);

struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    antigo: HGDIOBJ,
    bits: *mut u8,
    lado: u32,
}

impl Dib {
    fn new(lado: u32) -> Option<Dib> {
        unsafe {
            let tela = GetDC(null_mut());
            let dc = CreateCompatibleDC(tela);
            ReleaseDC(null_mut(), tela);
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = lado as i32;
            bi.bmiHeader.biHeight = -(lado as i32); // de cima para baixo
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let antigo = SelectObject(dc, bmp);
            Some(Dib { dc, bmp, antigo, bits: bits as *mut u8, lado })
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.antigo);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

struct App {
    ctrl: HWND,
    janela: HWND,
    cfg: Config,
    desenho: Desenho,
    dib: Option<Dib>,
    icone: HICON,

    aberta: bool,
    visivel: bool,
    t_abertura: Instant,
    t_fecho: Option<(Instant, f32)>,
    ultimo: Instant,
    prog: f32,
    selecionada: Option<usize>,
    hover: Vec<f32>,
    esmaecer: f32,
    cancelar: f32,
    centro: POINT,
    canto: POINT,
    escala: f32,

    acao_pendente: Option<usize>,
    reiniciar: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Acessa o estado. Se já estiver em uso (mensagem reentrante), ignora.
fn com_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn cursor() -> POINT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    p
}

impl App {
    fn dist_e_angulo(&self) -> (f32, f32) {
        let p = cursor();
        let dx = (p.x - self.centro.x) as f32 / self.escala;
        let dy = (p.y - self.centro.y) as f32 / self.escala;
        (dx.hypot(dy), dy.atan2(dx))
    }

    fn indice_sob_cursor(&self) -> Option<usize> {
        let (dist, ang) = self.dist_e_angulo();
        // no centro ou fora da roleta = nada selecionado (soltar cancela)
        if dist < self.desenho.geo.r_furo || dist > self.desenho.geo.r {
            return None;
        }
        let n = self.cfg.opcoes.len();
        let larg = std::f32::consts::TAU / n as f32;
        let rel = (ang + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
        Some((rel / larg).round() as usize % n)
    }

    fn longe_demais(&self) -> bool {
        self.dist_e_angulo().0 > self.desenho.geo.r + MARGEM - 4.0
    }

    fn abrir(&mut self) {
        if self.aberta {
            // clicou de novo com ela aberta: fecha
            self.fechar();
            return;
        }
        let pt = cursor();
        unsafe {
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 {
                dx = 96;
            }
            self.escala = dx as f32 / 96.0;
            self.desenho.preparar(self.escala);
            let lado = self.desenho.lado_px();
            if self.dib.as_ref().map(|d| d.lado) != Some(lado) {
                self.dib = Dib::new(lado);
            }
            // a janela inteira cabe na tela
            let meio = lado as i32 / 2;
            let r = mi.rcMonitor;
            let cx = pt.x.max(r.left + meio).min((r.right - meio).max(r.left + meio));
            let cy = pt.y.max(r.top + meio).min((r.bottom - meio).max(r.top + meio));
            self.centro = POINT { x: cx, y: cy };
            self.canto = POINT { x: cx - meio, y: cy - meio };
        }

        self.selecionada = None;
        self.hover = vec![0.0; self.cfg.opcoes.len()];
        self.esmaecer = 0.0;
        self.cancelar = 1.0;
        self.aberta = true;
        self.t_fecho = None;
        self.t_abertura = Instant::now();
        self.ultimo = self.t_abertura;
        self.prog = 0.0;
        self.pintar();
        unsafe {
            ShowWindow(self.janela, SW_SHOWNOACTIVATE);
            SetWindowPos(self.janela, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            if !self.visivel {
                timeBeginPeriod(1); // timer de 10 ms de verdade enquanto anima
                SetTimer(self.ctrl, ID_ANIM, 10, None);
            }
        }
        self.visivel = true;
    }

    fn fechar(&mut self) {
        if !self.aberta {
            return;
        }
        self.aberta = false;
        hook::MODO_CLIQUE.store(false, Relaxed);
        self.t_fecho = Some((Instant::now(), self.prog));
    }

    fn confirmar(&mut self) {
        let idx = self.indice_sob_cursor();
        self.fechar();
        if let Some(i) = idx {
            // pequeno atraso para a roleta sumir antes da ação
            self.acao_pendente = Some(i);
            unsafe { SetTimer(self.ctrl, ID_ACAO, 120, None) };
        }
    }

    fn soltar(&mut self) {
        if !self.aberta {
            return;
        }
        let rapido = self.t_abertura.elapsed().as_secs_f32() < TEMPO_CLIQUE;
        if rapido && self.indice_sob_cursor().is_none() {
            hook::MODO_CLIQUE.store(true, Relaxed); // fica aberta esperando clique
        } else {
            self.confirmar();
        }
    }

    fn tick(&mut self) {
        if !self.visivel {
            return;
        }
        let agora = Instant::now();
        let dt = (agora - self.ultimo).as_secs_f32().min(0.1);
        self.ultimo = agora;
        if self.aberta {
            let t = ((agora - self.t_abertura).as_secs_f32() / T_ABRIR).min(1.0);
            self.prog = 1.0 - (1.0 - t).powi(3);
            if self.longe_demais() {
                // saiu da roleta: fecha (e não fica presa se o botão for
                // solto em cima de uma janela de administrador)
                self.fechar();
                return;
            }
            self.selecionada = self.indice_sob_cursor();
        } else if let Some((t0, p0)) = self.t_fecho {
            let t = ((agora - t0).as_secs_f32() / T_FECHAR).min(1.0);
            self.prog = p0 * (1.0 - t);
            if t >= 1.0 {
                self.prog = 0.0;
                self.esconder();
                return;
            }
        }

        // transições suaves do destaque (independente da taxa de quadros)
        let k = 1.0 - 0.68f32.powf(dt / 0.010);
        for (i, h) in self.hover.iter_mut().enumerate() {
            let alvo = if Some(i) == self.selecionada { 1.0 } else { 0.0 };
            *h += (alvo - *h) * k;
        }
        let tem = self.selecionada.is_some() as i32 as f32;
        self.esmaecer += (tem - self.esmaecer) * k;
        self.cancelar += ((1.0 - tem) - self.cancelar) * k;
        self.pintar();
    }

    fn esconder(&mut self) {
        unsafe {
            KillTimer(self.ctrl, ID_ANIM);
            timeEndPeriod(1);
            ShowWindow(self.janela, SW_HIDE);
        }
        self.visivel = false;
    }

    fn pintar(&mut self) {
        let est = Estado { prog: self.prog, hover: &self.hover, esmaecer: self.esmaecer, cancelar: self.cancelar };
        let pm = self.desenho.desenhar(&est);
        let Some(dib) = &self.dib else { return };
        let lado = dib.lado;
        if pm.width() != lado {
            return;
        }
        // RGBA pré-multiplicado (tiny-skia) -> BGRA pré-multiplicado (GDI)
        let dst = unsafe { std::slice::from_raw_parts_mut(dib.bits, (lado * lado * 4) as usize) };
        for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
        let alfa = ((self.prog * 1.25).min(1.0) * 255.0).round() as u8;
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alfa,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let tam = SIZE { cx: lado as i32, cy: lado as i32 };
        let origem = POINT { x: 0, y: 0 };
        unsafe {
            let tela = GetDC(null_mut());
            UpdateLayeredWindow(self.janela, tela, &self.canto, &tam, dib.dc, &origem, 0, &blend, ULW_ALPHA);
            ReleaseDC(null_mut(), tela);
        }
    }

    fn dados_bandeja(&self) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.ctrl;
        nid.uID = 1;
        nid
    }

    fn mostrar_bandeja(&self) {
        let mut nid = self.dados_bandeja();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_BANDEJA;
        nid.hIcon = self.icone;
        for (d, s) in nid.szTip.iter_mut().zip("Roleta do mouse".encode_utf16()) {
            *d = s;
        }
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    }

    fn sair(&mut self) {
        hook::parar();
        let nid = self.dados_bandeja();
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &nid);
            if self.visivel {
                timeEndPeriod(1);
            }
            PostQuitMessage(0);
        }
    }
}

fn evento(ev: usize) {
    if std::env::var_os("ROLETA_DEBUG").is_some() {
        log::escrever(&format!("evento {ev}"));
    }
    com_app(|a| match ev {
        hook::EV_ABRIR => a.abrir(),
        hook::EV_SOLTAR => a.soltar(),
        hook::EV_CLIQUE if a.aberta => a.confirmar(),
        hook::EV_CANCELAR => a.fechar(),
        _ => {}
    });
}

fn executar_pendente() {
    // pega a opção e solta o estado antes de executar (ShellExecute etc.
    // podem processar mensagens enquanto rodam)
    let op = com_app(|a| {
        unsafe { KillTimer(a.ctrl, ID_ACAO) };
        a.acao_pendente.take().and_then(|i| a.cfg.opcoes.get(i).cloned())
    })
    .flatten();
    if let Some(op) = op {
        if acoes::executar(&op) {
            com_app(|a| a.sair());
        }
    }
}

fn menu_bandeja(ctrl: HWND) {
    unsafe {
        let m = CreatePopupMenu();
        let itens: [(u32, usize, &str); 5] = [
            (MF_STRING | MF_GRAYED, 0, "Roleta ativa"),
            (MF_SEPARATOR, 0, ""),
            (MF_STRING, CMD_EDITAR, "Editar opções"),
            (MF_STRING, CMD_REINICIAR, "Reiniciar (aplica mudanças)"),
            (MF_STRING, CMD_SAIR, "Sair"),
        ];
        for (f, id, txt) in itens {
            let w = wide(txt);
            AppendMenuW(m, f, id, if txt.is_empty() { null() } else { w.as_ptr() });
        }
        let p = cursor();
        SetForegroundWindow(ctrl);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, 0, ctrl, null());
        PostMessageW(ctrl, WM_NULL, 0, 0);
        DestroyMenu(m);
        match cmd as usize {
            CMD_EDITAR => acoes::abrir_como_usuario(&config::caminho().to_string_lossy()),
            CMD_REINICIAR => {
                com_app(|a| {
                    a.reiniciar = true;
                    a.sair();
                });
            }
            CMD_SAIR => {
                com_app(|a| a.sair());
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn proc_ctrl(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        hook::WM_ROLETA => evento(wparam),
        WM_TIMER if wparam == ID_ANIM => {
            com_app(|a| a.tick());
        }
        WM_TIMER if wparam == ID_ACAO => executar_pendente(),
        WM_BANDEJA => {
            let ev = (lparam & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                menu_bandeja(hwnd);
            }
        }
        m if m != 0 && m == MSG_TASKBAR.load(Relaxed) => {
            // o Explorer reiniciou: põe o ícone de volta
            com_app(|a| a.mostrar_bandeja());
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

unsafe extern "system" fn proc_janela(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        // cliques que chegam direto na janela (quando o hook não os pegou)
        WM_LBUTTONDOWN => {
            com_app(|a| {
                if a.aberta {
                    a.confirmar()
                }
            });
            0
        }
        WM_RBUTTONDOWN => {
            com_app(|a| a.fechar());
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn registrar(nome: &[u16], proc_: WNDPROC) {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: proc_,
        hInstance: unsafe { GetModuleHandleW(null()) },
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        lpszClassName: nome.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { RegisterClassExW(&wc) };
}

/// Roda a roleta até sair. Devolve `true` se pediu para reiniciar.
pub fn rodar(cfg: Config) -> bool {
    unsafe {
        let inst = GetModuleHandleW(null());
        let c_ctrl = wide("RoletaControle");
        let c_jan = wide("RoletaJanela");
        let titulo = wide("Roleta");
        registrar(&c_ctrl, Some(proc_ctrl));
        registrar(&c_jan, Some(proc_janela));
        let ctrl = CreateWindowExW(0, c_ctrl.as_ptr(), titulo.as_ptr(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), inst, null());
        // WS_EX_NOACTIVATE: a janela não pega o foco (atalhos vão para o programa ativo)
        let janela = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            c_jan.as_ptr(),
            titulo.as_ptr(),
            WS_POPUP,
            0, 0, 1, 1,
            null_mut(), null_mut(), inst, null(),
        );
        if ctrl.is_null() || janela.is_null() {
            log::escrever("ERRO: não deu para criar as janelas");
            return false;
        }
        MSG_TASKBAR.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Relaxed);

        let lado_ic = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
        let icone = LoadImageW(inst, 1 as _, IMAGE_ICON, lado_ic, lado_ic, LR_DEFAULTCOLOR) as HICON;

        let t0 = Instant::now();
        let desenho = Desenho::new(&cfg, &config::pasta_dados());
        let botao = cfg.botao.clone();
        let n = cfg.opcoes.len();
        let app = App {
            ctrl,
            janela,
            cfg,
            desenho,
            dib: None,
            icone,
            aberta: false,
            visivel: false,
            t_abertura: Instant::now(),
            t_fecho: None,
            ultimo: Instant::now(),
            prog: 0.0,
            selecionada: None,
            hover: vec![0.0; n],
            esmaecer: 0.0,
            cancelar: 1.0,
            centro: POINT { x: 0, y: 0 },
            canto: POINT { x: 0, y: 0 },
            escala: 1.0,
            acao_pendente: None,
            reiniciar: false,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        // já deixa os desenhos prontos para o monitor principal
        com_app(|a| {
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST);
            let (mut dx, mut dy) = (96u32, 96u32);
            GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            a.desenho.preparar(dx as f32 / 96.0);
            a.mostrar_bandeja();
        });
        hook::iniciar(ctrl, &botao);

        let nomes = match botao.as_str() {
            "x2" => "lateral da frente (X2)",
            "ambos" => "laterais (X1 e X2)",
            _ => "lateral de trás (X1)",
        };
        log::escrever(&format!(
            "Roleta ativa! Segure o botão {nomes}. (administrador: {}; pronta em {} ms)",
            if acoes::eh_admin() { "sim" } else { "não" },
            t0.elapsed().as_millis()
        ));

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let reiniciar = APP.with(|a| a.borrow().as_ref().map(|a| a.reiniciar).unwrap_or(false));
        DestroyWindow(janela);
        DestroyWindow(ctrl);
        APP.with(|a| *a.borrow_mut() = None);
        reiniciar
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
            ("sel0".into(), (0..n).map(|i| (i == 0) as i32 as f32).collect(), 1.0),
            ("sel2".into(), (0..n).map(|i| (i == 2) as i32 as f32).collect(), 1.0),
        ];
        for (nome, hover, esm) in casos {
            let est = Estado { prog: 1.0, hover: &hover, esmaecer: esm, cancelar: 1.0 - esm };
            let pm = d.desenhar(&est).clone();
            // fundo cinza-azulado para enxergar a transparência
            let mut fundo = resvg::tiny_skia::Pixmap::new(pm.width(), pm.height()).unwrap();
            fundo.fill(resvg::tiny_skia::Color::from_rgba8(0x3a, 0x4a, 0x5a, 255));
            fundo.draw_pixmap(0, 0, pm.as_ref(), &Default::default(), resvg::tiny_skia::Transform::identity(), None);
            let arq = format!("{pasta}/previa_{nome}_{escala}x.png");
            if let Err(e) = fundo.save_png(&arq) {
                eprintln!("{arq}: {e}");
            }
        }
    }
}
