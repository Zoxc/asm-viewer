//! The icon Windows shows for the executable itself, in Explorer and on a shortcut: a
//! resource linked into the binary, which only a Windows host's resource compiler can
//! make. The window's own icon is set at launch (`src/main.rs`) and needs none of this.

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/app.ico");
        resource.compile().expect("the icon resource compiles");
    }
}
