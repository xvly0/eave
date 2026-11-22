fn main() {
    println!("cargo:rerun-if-changed=assets/eave.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/eave.ico");
        res.set("ProductName", "Eave");
        res.set("FileDescription", "Eave — Dynamic Island companion for Windows");
        res.set("OriginalFilename", "eave.exe");
        res.set("InternalName", "eave");
        let _ = res.compile();
    }
}
