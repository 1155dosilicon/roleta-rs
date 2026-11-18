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

use super::acoes::{self, wide};
use super::hook;
use crate::config::{self, Config};
use crate::log;
use crate::roleta::{self, Roleta, Tick};

const WM_BANDEJA: u32 = WM_APP + 2;
const ID_ANIM: usize = 1;
const ID_ACAO: usize = 2;
const CMD_EDITAR: usize = 10;
const CMD_REINICIAR: usize = 11;
const CMD_SAIR: usize = 12;

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
    unsafe { GetCursorPos(&mut p) };
    p
}

fn ponto() -> roleta::Ponto {
    let p = cursor();
    (p.x, p.y)
}

impl App {
    /// O hook precisa saber se deve capturar os cliques esquerdo/direito.
    fn sincronizar(&self) {
        hook::MODO_CLIQUE.store(self.r.modo_clique, Relaxed);
    }

    fn abrir(&mut self) {
        let pt = cursor();
        let (monitor, escala) = unsafe {
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 {
                dx = 96;
            }
            let r = mi.rcMonitor;
            ((r.left, r.top, r.right, r.bottom), dx as f32 / 96.0)
        };
        let estava_visivel = self.r.visivel;
        if !self.r.abrir((pt.x, pt.y), monitor, escala) {
            self.sincronizar();
            return;
        }
        self.sincronizar();
        let lado = self.r.lado();
        if self.dib.as_ref().map(|d| d.lado) != Some(lado) {
            self.dib = Dib::new(lado);
        }
        self.pintar();
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
            UpdateLayeredWindow(self.janela, tela, &canto, &tam, dib.dc, &origem, 0, &blend, ULW_ALPHA);
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
        unsafe { KillTimer(a.ctrl, ID_ACAO) };
        a.acao_pendente.take().and_then(|i| a.r.opcao(i))
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
                if a.r.aberta {
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
        let botao = cfg.botao.clone();
        let app = App { ctrl, janela, r: Roleta::new(cfg), dib: None, icone, acao_pendente: None, reiniciar: false };
        APP.with(|a| *a.borrow_mut() = Some(app));
        // já deixa os desenhos prontos para o monitor principal
        com_app(|a| {
            let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST);
            let (mut dx, mut dy) = (96u32, 96u32);
            GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            a.r.desenho.preparar(dx as f32 / 96.0);
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
