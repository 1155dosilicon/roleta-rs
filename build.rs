fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/roleta.ico"); // recurso de ícone ID 1 (usado na bandeja)
        res.set("FileDescription", "Roleta do mouse");
        res.set("ProductName", "Roleta");
        res.compile().expect("falha ao embutir o ícone");
    }
    println!("cargo:rerun-if-changed=assets/roleta.ico");
}
