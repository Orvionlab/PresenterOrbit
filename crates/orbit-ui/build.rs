use std::fmt::Write;
use std::path::Path;

fn main() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut names: Vec<String> = std::fs::read_dir(&directory)
        .expect("assets/icons exists")
        .map(|entry| entry.expect("assets/icons is readable").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".svg"))
        .collect();
    names.sort();
    let mut table = String::from("const ICONS: &[(&str, &[u8])] = &[\n");
    for name in names {
        let path = directory.join(&name);
        writeln!(table, "    ({name:?}, include_bytes!({path:?})),").expect("writing to a string");
    }
    table.push_str("];\n");
    let output = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("icons.rs");
    std::fs::write(output, table).expect("OUT_DIR is writable");
}
