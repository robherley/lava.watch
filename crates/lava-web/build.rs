// build.rs — sanity-check that all embedded assets exist before we try to
// `include_bytes!` them. Friendly errors pointing at the script if missing.

use std::path::Path;

fn main() {
    let required = [
        ("../lava-wasm/pkg/lava_wasm.js", "script/build-wasm"),
        ("../lava-wasm/pkg/lava_wasm_bg.wasm", "script/build-wasm"),
    ];

    for (path, hint) in required {
        if !Path::new(path).exists() {
            eprintln!("\nlava-web: missing asset at {path}");
            eprintln!("  build it first: `{hint}`\n");
            std::process::exit(1);
        }
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=static");
}
