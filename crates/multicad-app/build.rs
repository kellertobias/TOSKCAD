fn main() {
    println!("cargo:rerun-if-changed=../../assets/branding/app/AppIcon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/branding/app/AppIcon.ico")
            .compile()
            .expect("Could not compile the Windows application icon");
    }
}
