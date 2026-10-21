use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-env-changed=TVHOME_IPK");
    println!("cargo:rerun-if-env-changed=TVHOME_LICENSES");
    println!("cargo:rerun-if-env-changed=CARGO_TARGET_DIR");
    let notices_out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("licenses.txt");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("home.ipk");
    if env::var_os("CARGO_FEATURE_BUNDLED_HOME").is_some() {
        let package = env::var_os("TVHOME_IPK")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from("../tv/dist/template/com.anant.tvhome.native_0.1.0_arm.ipk")
            });
        println!("cargo:rerun-if-changed={}", package.display());
        fs::copy(&package, &out).expect("Build the Home template first: ./tv/dev.sh template");
        let notices = env::var_os("TVHOME_LICENSES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env::var_os("CARGO_TARGET_DIR").unwrap_or_else(|| "target".into()))
                    .join("release-licenses.txt")
            });
        println!("cargo:rerun-if-changed={}", notices.display());
        fs::copy(notices, notices_out)
            .expect("Prepare license notices first: node cli/licenses.cjs");
    } else {
        fs::write(out, []).unwrap();
        println!("cargo:rerun-if-changed=../LICENSE");
        fs::copy("../LICENSE", notices_out).unwrap();
    }
}
