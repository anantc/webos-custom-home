fn main() {
    if std::env::var("TARGET").as_deref() != Ok("armv7-unknown-linux-gnueabi") {
        return;
    }
    let out = std::env::var("OUT_DIR").unwrap();
    let cc = std::env::var("CC_armv7_unknown_linux_gnueabi").unwrap();
    let obj = format!("{out}/glibc_compat_shim.o");
    assert!(std::process::Command::new(cc)
        .args([
            "-fPIC",
            "-c",
            "src/platform/glibc_compat_shim.c",
            "-o",
            &obj
        ])
        .status()
        .unwrap()
        .success());
    println!("cargo:rustc-link-arg={obj}");
    println!("cargo:rerun-if-changed=src/platform/glibc_compat_shim.c");
    let prefix = std::env::var("SDL3_PREFIX").expect("Run through ./dev.sh");
    println!("cargo:rustc-link-search=native={prefix}/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
    println!("cargo:rerun-if-env-changed=SDL3_PREFIX");
}
