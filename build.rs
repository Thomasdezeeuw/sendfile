use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

const LOREM_IPSUM: &'static [u8] = include_bytes!("tests/files/lorem_ipsum_1.txt");

fn main() {
    println!("rerun-if-changed=tests/files/lorem_ipsum_1.txt");
    println!("rerun-if-changed=build.rs");
    let root_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let dir = Path::new(&root_dir).join("tests").join("files");

    for size in &[50, 100, 200, 500, 1000] {
        let path = dir.join(format!("lorem_ipsum_{}.txt", size));
        let mut file = File::create(path).unwrap();

        for _ in 0..*size {
            file.write_all(LOREM_IPSUM).unwrap();
        }
    }
}
