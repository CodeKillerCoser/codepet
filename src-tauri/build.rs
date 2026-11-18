fn main() {
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/platform/window_drawer");
        cc::Build::new()
            .files(["src/platform/window_drawer/macos.m", "src/platform/window_drawer/watch.m"])
            .flag("-fobjc-arc")
            .compile("pet_window_drawer");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=ImageIO");
    }
    tauri_build::build()
}
