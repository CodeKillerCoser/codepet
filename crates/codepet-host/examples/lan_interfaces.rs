fn main() {
    for interface in codepet_host::remote_lan_interfaces() {
        println!("{}", serde_json::to_string(&interface).unwrap());
    }
    println!("Selected: {:?}", codepet_host::select_remote_lan_ipv4(None));
}
