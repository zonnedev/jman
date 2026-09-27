use std::{env, fs, path::PathBuf};

#[path = "src/versioning.rs"]
mod versioning;

fn main() {
    println!("cargo:rerun-if-env-changed=JMAN_BUILD_VERSION");

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides CARGO_MANIFEST_DIR"),
    );
    let repository = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("jman-version belongs to the JMAN workspace");
    let fallback = env::var("CARGO_PKG_VERSION").expect("Cargo provides CARGO_PKG_VERSION");

    versioning::emit_git_rerun_directives(repository);

    let info = match env::var("JMAN_BUILD_VERSION") {
        Ok(version) => {
            assert!(
                versioning::is_semver(&version),
                "JMAN_BUILD_VERSION must be valid SemVer, got {version:?}"
            );
            versioning::VersionInfo::override_version(&fallback, version)
        }
        Err(_) => versioning::discover(repository, &fallback),
    };

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"))
        .join("build_info.rs");
    fs::write(output, info.as_rust_source()).expect("write generated JMAN build information");
}
