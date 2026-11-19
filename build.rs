fn main() {
    println!("cargo:rerun-if-changed=assets/guardian.rc");
    println!("cargo:rerun-if-changed=assets/guardian.ico");
    println!("cargo:rerun-if-changed=assets/guardian.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = env!("CARGO_PKG_VERSION");
        let parts = format!(
            "{},{},{},0",
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH"),
        );
        let macros = [
            format!("GUARDIAN_VERSION_PARTS={parts}"),
            format!("GUARDIAN_FILE_VERSION=\"{version}.0\\0\""),
            format!("GUARDIAN_PRODUCT_VERSION=\"{version}\\0\""),
        ];
        embed_resource::compile("assets/guardian.rc", macros)
            .manifest_required()
            .expect("failed to embed Windows application resources");
    }
}
