use std::env;

fn main() {
    println!("cargo::rerun-if-env-changed=JAVAC_FRONTEND_LIB_DIR");

    if env::var_os("CARGO_FEATURE_NATIVE_FFI").is_some() {
        let directory = env::var("JAVAC_FRONTEND_LIB_DIR")
            .expect("JAVAC_FRONTEND_LIB_DIR is required with the native-ffi feature");
        println!("cargo::rustc-link-search=native={directory}");
        println!("cargo::rustc-link-lib=dylib=jman_javac_frontend");
        println!("cargo::rustc-link-arg=-Wl,-rpath,{directory}");
    }
}
