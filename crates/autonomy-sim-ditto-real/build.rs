use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=c/ditto_bridge.c");
    println!("cargo:rerun-if-env-changed=DITTO_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=DITTOFFI_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=DITTOFFI_LIB_DIR");

    if env::var_os("CARGO_FEATURE_DITTOFFI").is_none() {
        return;
    }

    let source_dir = env::var_os("DITTO_SOURCE_DIR").map(PathBuf::from);
    let include_dir = env::var_os("DITTOFFI_INCLUDE_DIR")
        .map(PathBuf::from)
        .or_else(|| source_dir.as_ref().map(|path| path.join("crates/dittoffi")))
        .unwrap_or_else(|| {
            panic!(
                "set DITTO_SOURCE_DIR or DITTOFFI_INCLUDE_DIR when enabling the dittoffi feature"
            )
        });
    let lib_dir = env::var_os("DITTOFFI_LIB_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            source_dir
                .as_ref()
                .map(|path| path.join("target/release/deps"))
        })
        .unwrap_or_else(|| {
            panic!("set DITTO_SOURCE_DIR or DITTOFFI_LIB_DIR when enabling the dittoffi feature")
        });

    if !include_dir.join("dittoffi.h").is_file() {
        panic!("dittoffi.h was not found in {}", include_dir.display());
    }
    if !lib_dir.is_dir() {
        panic!(
            "Ditto library directory was not found: {}",
            lib_dir.display()
        );
    }

    cc::Build::new()
        .file("c/ditto_bridge.c")
        .include(&include_dir)
        .warnings(true)
        .compile("autonomy_sim_ditto_bridge");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=dittoffi");
    if cfg!(target_family = "unix") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }
}
