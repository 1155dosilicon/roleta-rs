//! Gera assets/roleta.ico a partir de assets/icone.svg (e icone_pequeno.svg
//! para 16-24 px). Uso: cargo run --release --example gerar_icone
use resvg::{tiny_skia as sk, usvg};

fn render(svg: &str, tam: u32) -> Vec<u8> {
    let t = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let s = tam as f32 / t.size().width();
    let mut pm = sk::Pixmap::new(tam, tam).unwrap();
    resvg::render(&t, sk::Transform::from_scale(s, s), &mut pm.as_mut());
    pm.encode_png().unwrap()
}

fn main() {
    let grande = include_str!("../assets/icone.svg");
    let pequeno = include_str!("../assets/icone_pequeno.svg");
    let tamanhos = [16u32, 20, 24, 32, 40, 48, 64, 96, 128, 256];
    let imgs: Vec<(u32, Vec<u8>)> = tamanhos
        .iter()
        .map(|&t| (t, render(if t <= 24 { pequeno } else { grande }, t)))
        .collect();
    // ICO com imagens PNG dentro (aceito pelo Windows desde o Vista)
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(imgs.len() as u16).to_le_bytes());
    let mut ofs = 6 + 16 * imgs.len() as u32;
    for (t, png) in &imgs {
        let b = if *t >= 256 { 0 } else { *t as u8 };
        ico.extend_from_slice(&[b, b, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&ofs.to_le_bytes());
        ofs += png.len() as u32;
    }
    for (_, png) in &imgs {
        ico.extend_from_slice(png);
    }
    std::fs::write("assets/roleta.ico", &ico).unwrap();
    // amostra para conferir: todos os tamanhos lado a lado
    let mut folha = sk::Pixmap::new(tamanhos.iter().sum::<u32>() + 10 * 11, 270).unwrap();
    folha.fill(sk::Color::from_rgba8(0x20, 0x22, 0x28, 255));
    let mut x = 10;
    for (t, png) in &imgs {
        let pm = sk::Pixmap::decode_png(png).unwrap();
        folha.draw_pixmap(x, 10, pm.as_ref(), &Default::default(), sk::Transform::identity(), None);
        x += *t as i32 + 10;
    }
    let saida = std::env::args().nth(1).unwrap_or("amostra_icone.png".into());
    folha.save_png(&saida).unwrap();
    println!("assets/roleta.ico: {} bytes, {} tamanhos", ico.len(), imgs.len());
}
