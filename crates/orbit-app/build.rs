use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=packaging/app.rc");
    println!("cargo:rerun-if-changed=packaging/icons/icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile_for(
            "packaging/app.rc",
            ["orbit-app"],
            embed_resource::ParamsIncludeDirs(["packaging"]),
        )
        .manifest_required()
        .expect("The Windows application icon must be embedded");
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
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("dependency-notices.txt");
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
