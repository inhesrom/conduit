// Guarantee the embed folder exists so rust-embed always compiles, even on a
// fresh checkout where the frontend hasn't been built yet. A real
// `bun run build` overwrites this placeholder; in release builds the daemon
// embeds whatever dist contains.
use std::fs;
use std::path::Path;

fn main() {
    let dist = Path::new("../../web/app/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        let _ = fs::create_dir_all(dist);
        let _ = fs::write(
            &index,
            "<!doctype html><meta charset=utf-8><title>conduit</title>\
             <body style=\"font-family:sans-serif;background:#0e1116;color:#dce1e8;padding:40px\">\
             <p>The conduit web UI hasn't been built yet. Run <code>bun --cwd web run build</code>.</p>",
        );
    }
    println!("cargo:rerun-if-changed=../../web/app/dist");
}
