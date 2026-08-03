fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let upstream_linker = root
        .join("../../.generated/hardware-abstraction-ir/evidence/wch/ch32v203g6u6/generated/embassy-usb-cdc-smoke");
    println!("cargo:rustc-link-search={}", root.display());
    println!("cargo:rustc-link-search={}", upstream_linker.display());
    println!("cargo:rustc-link-arg=-Tmemory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
