fn main() {
    // The native module owns versions, toolchains, source checkout and caching.
    let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/webrtc");
    for file in ["build.mjs", "versions.json", "vcpkg.json", "smoke.c", "triplets"] {
        println!("cargo:rerun-if-changed={}", module.join(file).display());
    }
    println!("cargo:rerun-if-env-changed=CODEPET_NODE");
    let target = std::env::var("TARGET").expect("Cargo TARGET");
    let output = std::process::Command::new(
        std::env::var_os("CODEPET_NODE").unwrap_or_else(|| "node".into()),
    )
    .arg(module.join("build.mjs"))
    .args(["--target", &target])
    .stderr(std::process::Stdio::inherit())
    .output()
    .expect("request WebRTC SDK (Node.js is required)");
    assert!(output.status.success(), "WebRTC SDK build failed");
    let artifact: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("WebRTC artifact descriptor");
    let manifest = artifact["manifest"].as_str().expect("WebRTC manifest path");
    println!("cargo:rerun-if-changed={manifest}");
    println!("cargo:rustc-env=CODEPET_WEBRTC_ARTIFACT={manifest}");
    tauri_build::build()
}
