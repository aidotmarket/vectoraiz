//! Headless entry point for the scaffold diagnostic.
fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["self-test"] {
        eprintln!("Usage: vectoraiz-cli self-test");
        std::process::exit(2);
    }
    println!(
        "{}",
        serde_json::to_string(&vz_core::self_test()).expect("self-test is serializable")
    );
}
