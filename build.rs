//! Embeds the application icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/janmp.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/janmp.ico").set("ProductName", "janmp").set("FileDescription", "janmp music player");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
