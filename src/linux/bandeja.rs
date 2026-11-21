//! Ícone na bandeja (StatusNotifierItem, pelo D-Bus) com o menu Editar
//! opções / Reiniciar / Sair. Funciona na bandeja do XFCE, KDE, etc.

use std::sync::mpsc::Sender;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use resvg::tiny_skia as sk;
use resvg::usvg;

use super::janela::Msg;
use crate::log;

const ICONE: &str = include_str!("../../assets/icone.svg");

pub struct Bandeja {
    tx: Sender<Msg>,
    icones: Vec<ksni::Icon>,
}

/// Renderiza o ícone em ARGB32 big-endian (formato do StatusNotifierItem).
fn icone(arvore: &usvg::Tree, lado: u32) -> Option<ksni::Icon> {
    let mut pm = sk::Pixmap::new(lado, lado)?;
    let tam = arvore.size();
    let s = lado as f32 / tam.width().max(tam.height());
    resvg::render(arvore, sk::Transform::from_scale(s, s), &mut pm.as_mut());
    let mut data = Vec::with_capacity((lado * lado * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        data.extend_from_slice(&[c.alpha(), c.red(), c.green(), c.blue()]);
    }
    Some(ksni::Icon { width: lado as i32, height: lado as i32, data })
}

impl Bandeja {
    fn item(&self, label: &str, msg: fn() -> Msg) -> ksni::MenuItem<Self> {
        StandardItem {
            label: label.into(),
            activate: Box::new(move |b: &mut Self| {
                let _ = b.tx.send(msg());
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Bandeja {
    fn id(&self) -> String {
        "roleta".into()
    }
    fn title(&self) -> String {
        "Roleta do mouse".into()
    }
    /// Nome no tema de ícones: a bandeja usa o desenho do tema do usuário (o
    /// instalar.sh põe `roleta.svg` no hicolor, então o nome sempre resolve).
    fn icon_name(&self) -> String {
        "roleta".into()
    }
    /// Reserva para bandejas que não acham o nome no tema.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icones.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Roleta do mouse".into(),
            description: "Segure o botão lateral do mouse".into(),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem { label: "Roleta ativa".into(), enabled: false, ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            self.item("Editar opções", || Msg::Editar),
            self.item("Reiniciar (aplica mudanças)", || Msg::Reiniciar),
            self.item("Sair", || Msg::Sair),
        ]
    }
}

/// Mostra o ícone. Sem bandeja (ou sem D-Bus) a roleta funciona do mesmo jeito.
pub fn iniciar(tx: Sender<Msg>) -> Option<Handle<Bandeja>> {
    let icones = usvg::Tree::from_str(ICONE, &usvg::Options::default())
        .map(|t| [16, 22, 24, 32, 48, 64].into_iter().filter_map(|l| icone(&t, l)).collect())
        .unwrap_or_default();
    match (Bandeja { tx, icones }).spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            log::escrever(&format!("Sem ícone na bandeja: {e}"));
            None
        }
    }
}
