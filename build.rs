fn main() {
    println!("cargo:rerun-if-changed=assets/snow.rc");
    println!("cargo:rerun-if-changed=assets/snow.ico");
    println!("cargo:rerun-if-changed=assets/snow.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/snow.rc", embed_resource::NONE);
    }
}
