fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    #[cfg(windows)]
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        winresource::WindowsResource::new()
            .set_icon("assets/app.ico")
            .set("ProductName", "Roc Request Bin")
            .set("FileDescription", "Roc Request Bin")
            .compile()
            .expect("Embed Windows application icon");
    }
}
