use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=packaging/app.rc");
    println!("cargo:rerun-if-changed=packaging/icons/icon.ico");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let major = env!("CARGO_PKG_VERSION_MAJOR");
        let minor = env!("CARGO_PKG_VERSION_MINOR");
        let patch = env!("CARGO_PKG_VERSION_PATCH");
        let version = env!("CARGO_PKG_VERSION");
        std::fs::write(
            out_dir.join("app-version.h"),
            format!(
                "#define ORBIT_VERSION {major},{minor},{patch},0\n#define ORBIT_FILE_VERSION \"{major}.{minor}.{patch}.0\"\n#define ORBIT_PRODUCT_VERSION \"{version}\"\n"
            ),
        )
        .expect("The Windows application version header must be written");
        embed_resource::compile_for(
            "packaging/app.rc",
            ["orbit-app"],
            embed_resource::ParamsIncludeDirs([std::path::Path::new("packaging"), &out_dir]),
        )
        .manifest_required()
        .expect("The Windows application resources must be embedded");
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    for file in [
        "Cargo.lock",
        "Cargo.toml",
        "crates/orbit-app/Cargo.toml",
        "crates/orbit-ui/Cargo.toml",
        "crates/orbit-host/Cargo.toml",
        "scripts/generate-rust-notices.mts",
        "docs/licenses/upstream-crate-notices.json",
        "crates/orbit-ui/assets/icons/LICENSE",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(file).display());
    }
    let output = out_dir.join("dependency-notices.txt");
    let status = Command::new("node")
        .arg(root.join("scripts/generate-rust-notices.mts"))
        .arg("--out")
        .arg(&output)
        .arg("--json")
        .arg(output.with_extension("json"))
        .arg("--target")
        .arg(env::var("TARGET").unwrap())
        .current_dir(root)
        .status()
        .expect("Node.js is required to generate dependency notices");
    assert!(status.success(), "Dependency notice generation failed");
}
