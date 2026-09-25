use std::env;

fn main() {
    println!("cargo::rerun-if-env-changed=JMAN_JAVAC_FRONTEND_LIB_DIR");

    if env::var_os("CARGO_FEATURE_NATIVE_FFI").is_some() {
        let directory = env::var("JMAN_JAVAC_FRONTEND_LIB_DIR")
            .expect("JMAN_JAVAC_FRONTEND_LIB_DIR is required with the native-ffi feature");
        println!("cargo::rustc-link-search=native={directory}");
        println!("cargo::rustc-link-lib=dylib=jman_javac_frontend");
        println!("cargo::rustc-link-arg=-Wl,-rpath,{directory}");
        println!("cargo::rustc-env=JMAN_COMPILED_FRONTEND_PLATFORM_HOME={directory}/platform");
    }
}
