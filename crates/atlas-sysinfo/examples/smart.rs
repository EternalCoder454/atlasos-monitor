//! Prints what every drive says about its health through udisks2:
//! `cargo run -p atlas-sysinfo --example smart`.

use atlas_sysinfo::smart::SmartReader;

fn main() {
    let Some(mut reader) = SmartReader::new() else {
        println!("udisks2 is not on the system bus");
        return;
    };
    let mut names: Vec<String> = std::fs::read_dir("/sys/block")
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    for name in names {
        match reader.read(&name) {
            Some(h) => println!("{name}: {h:#?}"),
            None => println!("{name}: nothing to report"),
        }
    }
}
