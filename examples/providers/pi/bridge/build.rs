// Experimental adapter: derive typed forwarding from the distributed method registry.
// No copied method list and no Provider-specific business code in the bridge.
fn main() {
    let path = "../provider-manifest.json";
    println!("cargo:rerun-if-changed={path}");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut code = String::from("impl ProtocolServer for Bridge {\n");
    for method in manifest["methods"].as_array().unwrap() {
        let name = method["name"].as_str().unwrap();
        let mut rust_name = String::new();
        for c in name.chars() {
            if c == '.' {
                rust_name.push('_');
            } else if c.is_ascii_uppercase() {
                rust_name.push('_');
                rust_name.push(c.to_ascii_lowercase());
            } else {
                rust_name.push(c);
            }
        }
        let request = method["request"]["$ref"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap();
        let response = method["response"]["$ref"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap();
        code.push_str(&format!("fn {rust_name}<'a>(&'a self, request: {request}) -> ProtocolFuture<'a, {response}> {{ Box::pin(async move {{ self.call(\"{name}\", request).await }}) }}\n"));
    }
    code.push_str("}\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("forward.rs"),
        code,
    )
    .unwrap();
}
