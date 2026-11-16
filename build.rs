fn main() {
    println!("cargo:rerun-if-changed=assets/guardian.rc");
    println!("cargo:rerun-if-changed=assets/guardian.ico");
    println!("cargo:rerun-if-changed=assets/guardian.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/guardian.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed Windows application resources");
    }
}
