use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" => {
            println!("cargo::rustc-link-arg=-Wl,-rpath,$ORIGIN");
        }
        "macos" => {
            println!("cargo::rustc-link-arg=-Wl,-rpath,@loader_path");
        }
        _ => {}
    }
}
