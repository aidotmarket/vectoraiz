#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    vz_office_windows_harness::launch::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows feasibility harness: Windows required; no probes run");
    std::process::exit(78);
}
