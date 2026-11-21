//! Janelas (roleta transparente + janela de controle escondida), bandeja,
//! animação e a lógica de abrir/soltar/clicar.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, HMONITOR,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC, SelectObject,
};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForSystem, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::acoes::{self, wide};
use super::hook;
use crate::config::{self, Botao, Config};
use crate::log;
use crate::roleta::{self, Roleta, Tick};

const WM_BANDEJA: u32 = WM_APP + 2;
const ID_ANIM: usize = 1;
const ID_ACAO: usize = 2;
const CMD_EDITAR: usize = 10;
const CMD_REINICIAR: usize = 11;
const CMD_SAIR: usize = 12;

static MSG_TASKBAR: AtomicU32 = AtomicU32::new(0);

/// Bitmap de 32 bits (BGRA) selecionado num DC de memória, para o
/// UpdateLayeredWindow.
struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    antigo: HGDIOBJ,
    /// `lado * lado * 4` bytes, válidos enquanto o `Dib` existir.
    bits: *mut u8,
    lado: u32,
}

impl Dib {
    fn new(lado: u32) -> Option<Dib> {
        // SAFETY: DC da tela pego e solto em seguida; o DC de memória é nosso.
        let dc = unsafe {
            let tela = GetDC(null_mut());
            let dc = CreateCompatibleDC(tela);
            ReleaseDC(null_mut(), tela);
            dc
        };
        if dc.is_null() {
            return None;
        }
        // SAFETY: BITMAPINFO é POD (zero é válido).
        let mut bi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = lado as i32;
        bi.bmiHeader.biHeight = -(lado as i32); // de cima para baixo
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut c_void = null_mut();
        // SAFETY: `bi` descreve um DIB de 32 bits; `bits` recebe o ponteiro
        // para a memória que o GDI aloca (lado * lado * 4 bytes).
        let bmp = unsafe { CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
        if bmp.is_null() || bits.is_null() {
            // SAFETY: DC criado acima e não usado em mais nada.
            unsafe { DeleteDC(dc) };
            return None;
        }
        // SAFETY: dc e bmp válidos; o objeto antigo volta no Drop.
        let antigo = unsafe { SelectObject(dc, bmp) };
        Some(Dib { dc, bmp, antigo, bits: bits.cast(), lado })
    }

    fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: `bits` aponta para lado*lado*4 bytes que vivem até o Drop, e
        // o &mut self garante que não há outro slice ao mesmo tempo.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.lado * self.lado * 4) as usize) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: desfaz o new na ordem certa (tira o bitmap do DC antes de apagar).
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
    r: Roleta,
    dib: Option<Dib>,
    icone: HICON,
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
    // SAFETY: `p` é um POINT válido para escrita.
    unsafe { GetCursorPos(&mut p) };
    p
}

fn ponto() -> roleta::Ponto {
    let p = cursor();
    (p.x, p.y)
}

/// Escala (DPI efetivo / 96) do monitor; 1.0 se não der para saber.
fn escala_do_monitor(mon: HMONITOR) -> f32 {
    let (mut dx, mut dy) = (96u32, 96u32);
    // SAFETY: dx e dy são u32 válidos para escrita; monitor inválido = erro.
    if unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } != 0 {
        dx = 96;
    }
    dx as f32 / 96.0
}

impl App {
    /// O hook precisa saber se deve capturar os cliques esquerdo/direito.
    fn sincronizar(&self) {
        hook::MODO_CLIQUE.store(self.r.modo_clique, Relaxed);
    }

    fn abrir(&mut self) {
        let pt = cursor();
        // SAFETY: sem ponteiros; MONITOR_DEFAULTTONEAREST sempre devolve um monitor.
        let mon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
        // SAFETY: MONITORINFO é POD (zero é válido).
        let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        // SAFETY: `mi` tem o cbSize certo e é válido para escrita.
        unsafe { GetMonitorInfoW(mon, &mut mi) };
        let r = mi.rcMonitor;
        let monitor = (r.left, r.top, r.right, r.bottom);

        let estava_visivel = self.r.visivel;
        let abriu = self.r.abrir((pt.x, pt.y), monitor, escala_do_monitor(mon));
        self.sincronizar();
        if !abriu {
            return;
        }
        if self.r.precisa_centralizar((pt.x, pt.y)) {
            let (cx, cy) = self.r.centro;
            // SAFETY: sem ponteiros; coordenadas físicas (processo per-monitor v2).
            unsafe { SetCursorPos(cx, cy) };
        }
        let lado = self.r.lado();
        if self.dib.as_ref().map(|d| d.lado) != Some(lado) {
            self.dib = Dib::new(lado);
        }
        self.pintar();
        // SAFETY: janelas nossas, válidas enquanto o App existe.
        unsafe {
            ShowWindow(self.janela, SW_SHOWNOACTIVATE);
            SetWindowPos(self.janela, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            if !estava_visivel {
                timeBeginPeriod(1); // timer de 10 ms de verdade enquanto anima
                SetTimer(self.ctrl, ID_ANIM, roleta::INTERVALO.as_millis() as u32, None);
            }
        }
    }

    fn fechar(&mut self) {
        self.r.fechar();
        self.sincronizar();
    }

    fn agendar(&mut self, idx: Option<usize>) {
        self.sincronizar();
        if let Some(i) = idx {
            self.acao_pendente = Some(i);
            // SAFETY: janela nossa; sem callback (chega como WM_TIMER).
            unsafe { SetTimer(self.ctrl, ID_ACAO, roleta::ATRASO_ACAO.as_millis() as u32, None) };
        }
    }

    fn confirmar(&mut self) {
        let idx = self.r.confirmar(ponto());
        self.agendar(idx);
    }

    fn soltar(&mut self) {
        let idx = self.r.soltar(ponto());
        self.agendar(idx);
    }

    fn tick(&mut self) {
        match self.r.tick(ponto()) {
            Tick::Pintar => self.pintar(),
            Tick::Esconder => self.esconder(),
            Tick::Nada => {}
        }
        self.sincronizar();
    }

    fn esconder(&mut self) {
        // SAFETY: janelas nossas; equilibra o timeBeginPeriod do abrir.
        unsafe {
            KillTimer(self.ctrl, ID_ANIM);
            timeEndPeriod(1);
            ShowWindow(self.janela, SW_HIDE);
        }
    }

    fn pintar(&mut self) {
        let alfa = self.r.alfa();
        let canto = POINT { x: self.r.canto.0, y: self.r.canto.1 };
        let pm = self.r.quadro();
        let Some(dib) = &mut self.dib else { return };
        let lado = dib.lado;
        if pm.width() != lado || pm.height() != lado {
            return;
        }
        // RGBA pré-multiplicado (tiny-skia) -> BGRA pré-multiplicado (GDI)
        for (d, s) in dib.pixels().as_chunks_mut::<4>().0.iter_mut().zip(pm.data().as_chunks::<4>().0) {
            *d = [s[2], s[1], s[0], s[3]];
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alfa,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let tam = SIZE { cx: lado as i32, cy: lado as i32 };
        let origem = POINT { x: 0, y: 0 };
        // SAFETY: todos os ponteiros são de locais vivos; o DC do Dib tem o
        // bitmap do tamanho `tam` selecionado.
        unsafe {
            let tela = GetDC(null_mut());
            UpdateLayeredWindow(self.janela, tela, &canto, &tam, dib.dc, &origem, 0, &blend, ULW_ALPHA);
            ReleaseDC(null_mut(), tela);
        }
    }

    fn dados_bandeja(&self) -> NOTIFYICONDATAW {
        // SAFETY: NOTIFYICONDATAW é POD (zero é válido).
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
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
        // SAFETY: `nid` preenchido e com cbSize certo.
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    }

    fn sair(&mut self) {
        hook::parar();
        let nid = self.dados_bandeja();
        // SAFETY: `nid` identifica o ícone criado no mostrar_bandeja.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &nid);
            if self.r.visivel {
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
        hook::EV_CLIQUE if a.r.aberta => a.confirmar(),
        hook::EV_CANCELAR => a.fechar(),
        _ => {}
    });
}

fn executar_pendente() {
    // pega a opção e solta o estado antes de executar (ShellExecute etc.
    // podem processar mensagens enquanto rodam)
    let op = com_app(|a| {
        // SAFETY: janela nossa.
        unsafe { KillTimer(a.ctrl, ID_ACAO) };
        a.acao_pendente.take().and_then(|i| a.r.opcao(i).cloned())
    })
    .flatten();
    if let Some(op) = op
        && acoes::executar(&op)
    {
        com_app(App::sair);
    }
}

fn menu_bandeja(ctrl: HWND) {
    let itens: [(u32, usize, &str); 5] = [
        (MF_STRING | MF_GRAYED, 0, "Roleta ativa"),
        (MF_SEPARATOR, 0, ""),
        (MF_STRING, CMD_EDITAR, "Editar opções"),
        (MF_STRING, CMD_REINICIAR, "Reiniciar (aplica mudanças)"),
        (MF_STRING, CMD_SAIR, "Sair"),
    ];
    let p = cursor();
    // SAFETY: o menu é criado e destruído aqui; o AppendMenuW copia cada
    // texto (que termina em 0) antes de `w` sair de escopo.
    let cmd = unsafe {
        let m = CreatePopupMenu();
        for (f, id, txt) in itens {
            let w = wide(txt);
            AppendMenuW(m, f, id, if txt.is_empty() { null() } else { w.as_ptr() });
        }
        SetForegroundWindow(ctrl);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, 0, ctrl, null());
        PostMessageW(ctrl, WM_NULL, 0, 0);
        DestroyMenu(m);
        cmd
    };
    match cmd as usize {
        CMD_EDITAR => acoes::abrir_como_usuario(&config::caminho().to_string_lossy()),
        CMD_REINICIAR => {
            com_app(|a| {
                a.reiniciar = true;
                a.sair();
            });
        }
        CMD_SAIR => {
            com_app(App::sair);
        }
        _ => {}
    }
}

/// Procedimento da janela de controle (escondida): hook, timers e bandeja.
unsafe extern "system" fn proc_ctrl(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        hook::WM_ROLETA => evento(wparam),
        WM_TIMER if wparam == ID_ANIM => {
            com_app(App::tick);
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
        // SAFETY: repassa os argumentos recebidos do Windows.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

/// Procedimento da janela da roleta.
unsafe extern "system" fn proc_janela(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        // cliques que chegam direto na janela (quando o hook não os pegou)
        WM_LBUTTONDOWN => {
            com_app(|a| {
                if a.r.aberta {
                    a.confirmar();
                }
            });
            0
        }
        WM_RBUTTONDOWN => {
            com_app(App::fechar);
            0
        }
        // SAFETY: repassa os argumentos recebidos do Windows.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Registra uma classe de janela. `nome` precisa terminar em 0 (ver [`wide`]).
fn registrar(nome: &[u16], proc_: WNDPROC) {
    assert_eq!(nome.last(), Some(&0), "nome da classe sem o 0 final");
    // SAFETY: sem pré-condições (módulo do próprio executável, cursor do sistema).
    let (inst, cursor) = unsafe { (GetModuleHandleW(null()), LoadCursorW(null_mut(), IDC_ARROW)) };
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: proc_,
        hInstance: inst,
        hCursor: cursor,
        lpszClassName: nome.as_ptr(),
        // SAFETY: WNDCLASSEXW é POD (zero é válido nos campos que sobram).
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `wc` preenchido; o Windows copia o nome da classe.
    unsafe { RegisterClassExW(&wc) };
}

/// Cria a janela de controle (escondida) e a da roleta (em camadas, sem foco).
fn criar_janelas() -> Option<(HWND, HWND)> {
    let c_ctrl = wide("RoletaControle");
    let c_jan = wide("RoletaJanela");
    let titulo = wide("Roleta");
    registrar(&c_ctrl, Some(proc_ctrl));
    registrar(&c_jan, Some(proc_janela));
    // SAFETY: classes registradas acima; as strings terminam em 0 e vivem até
    // o fim das chamadas.
    let (ctrl, janela) = unsafe {
        let inst = GetModuleHandleW(null());
        let ctrl = CreateWindowExW(
            0,
            c_ctrl.as_ptr(),
            titulo.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            inst,
            null(),
        );
        // WS_EX_NOACTIVATE: a janela não pega o foco (atalhos vão para o programa ativo)
        let janela = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            c_jan.as_ptr(),
            titulo.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            inst,
            null(),
        );
        (ctrl, janela)
    };
    (!ctrl.is_null() && !janela.is_null()).then_some((ctrl, janela))
}

/// Ícone da bandeja (recurso 1, embutido pelo build.rs) no tamanho do sistema.
fn carregar_icone() -> HICON {
    // SAFETY: recurso 1 = MAKEINTRESOURCE(1); falha devolve nulo (sem ícone).
    unsafe {
        let lado = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem());
        LoadImageW(GetModuleHandleW(null()), 1 as _, IMAGE_ICON, lado, lado, LR_DEFAULTCOLOR) as HICON
    }
}

/// Roda a roleta até sair. Devolve `true` se pediu para reiniciar.
pub fn rodar(cfg: Config) -> bool {
    let Some((ctrl, janela)) = criar_janelas() else {
        log::escrever("ERRO: não deu para criar as janelas");
        return false;
    };
    let taskbar = wide("TaskbarCreated");
    // SAFETY: a string termina em 0.
    MSG_TASKBAR.store(unsafe { RegisterWindowMessageW(taskbar.as_ptr()) }, Relaxed);

    let t0 = Instant::now();
    let botao = cfg.botao;
    let app = App {
        ctrl,
        janela,
        r: Roleta::new(cfg),
        dib: None,
        icone: carregar_icone(),
        acao_pendente: None,
        reiniciar: false,
    };
    APP.with(|a| *a.borrow_mut() = Some(app));
    // já deixa os desenhos prontos para o monitor principal
    com_app(|a| {
        // SAFETY: sem ponteiros.
        let mon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST) };
        a.r.desenho.preparar(escala_do_monitor(mon));
        a.mostrar_bandeja();
    });
    hook::iniciar(ctrl, botao);

    let nomes = match botao {
        Botao::X1 => "lateral de trás (X1)",
        Botao::X2 => "lateral da frente (X2)",
        Botao::Ambos => "laterais (X1 e X2)",
    };
    log::escrever(&format!(
        "Roleta ativa! Segure o botão {nomes}. (administrador: {}; pronta em {} ms)",
        if acoes::eh_admin() { "sim" } else { "não" },
        t0.elapsed().as_millis()
    ));

    // SAFETY: MSG é POD (zero é válido) e só é usado nesta thread.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let reiniciar = APP.with(|a| a.borrow().as_ref().is_some_and(|a| a.reiniciar));
    // SAFETY: janelas nossas, que não são mais usadas depois daqui.
    unsafe {
        DestroyWindow(janela);
        DestroyWindow(ctrl);
    }
    APP.with(|a| *a.borrow_mut() = None);
    reiniciar
}
