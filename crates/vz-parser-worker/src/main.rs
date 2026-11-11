//! One request per process. stdout is exclusively length-prefixed JSON.
fn main() {
    let result = vz_core::worker::apply_memory_limit().and_then(|_| {
        let request: vz_core::worker::Request =
            vz_core::worker::read_frame(std::io::stdin().lock())?;
        if request.office_label().is_some() {
            vz_core::worker::apply_office_memory_limit()?;
        }
        if let Some(job) = &request.office {
            let input = std::fs::read(&request.path)?;
            let report = if job.readback {
                vz_core::office::inspect(&job.name, &input)?
            } else {
                let (bytes, report) = if job.sanitized_input {
                    vz_core::office::edit(&job.name, &input, &job.edits)?
                } else {
                    vz_core::office::rebuild(&job.name, &input, &job.edits)?
                };
                std::fs::write(job.output.join("output.bin"), &bytes)?;
                report
            };
            // The output directory is UUID-owned private staging. Removal
            // values never travel through stdout or a delivered package.
            std::fs::write(job.output.join("report.json"), serde_json::to_vec(&report)?)?;
            Ok(Vec::new())
        } else {
            vz_core::parsers::parse(&request)
        }
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
