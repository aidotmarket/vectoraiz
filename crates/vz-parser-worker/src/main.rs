//! One request per process. stdout is exclusively length-prefixed JSON.
fn main() {
    let result = vz_core::worker::apply_memory_limit().and_then(|_| {
        let request = vz_core::worker::read_frame(std::io::stdin().lock())?;
        vz_core::parsers::parse(&request)
    });
    let response = match result {
        Ok(pages) => vz_core::worker::Response {
            pages,
            error: None,
            content_failure: false,
        },
        Err(error) => vz_core::worker::Response {
            pages: Vec::new(),
            content_failure: vz_core::failures::is_content(&error),
            error: Some(format!("{error:#}")),
        },
    };
    if vz_core::worker::write_frame(std::io::stdout().lock(), &response).is_err() {
        std::process::exit(1);
    }
}
