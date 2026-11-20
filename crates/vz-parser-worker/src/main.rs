//! One request per process. stdout is reserved for the bounded worker protocol.
fn main() {
    let result = vz_core::worker::apply_memory_limit().and_then(|_| {
        let request: vz_core::worker::Request =
            vz_core::worker::read_frame(std::io::stdin().lock())?;
        if request.memory_bytes() == vz_core::worker::OFFICE_MEMORY_BYTES {
            vz_core::worker::apply_office_memory_limit()?;
        }
        anyhow::ensure!(
            usize::from(request.projection.is_some())
                + usize::from(request.readback.is_some())
                + usize::from(request.office.is_some())
                + usize::from(request.generation_readback.is_some())
                <= 1,
            "ambiguous worker request"
        );
        if let Some(job) = &request.generation_readback {
            vz_core::storage::readback_staged_generation(&request.path, job).map(|()| Vec::new())
        } else if let Some(job) = &request.readback {
            let input = std::fs::read(&request.path)?;
            let inventory =
                vz_pack::sanitize::inspect_vector_artifact(&job.name, &input, &job.identity_terms)?;
            std::fs::write(
                job.output.join("inventory.json"),
                serde_json::to_vec(&serde_json::json!({"version":1,"inventory":inventory}))?,
            )?;
            Ok(Vec::new())
        } else if let Some(job) = &request.projection {
            anyhow::ensure!(request.office.is_none(), "ambiguous worker request");
            let input = std::fs::read(&request.path)?;
            let projection = vz_core::office::projection::extract_with_pdfium(
                &job.kind,
                &input,
                request.pdfium.as_deref(),
            )?;
            std::fs::write(
                job.output.join("projection.json"),
                serde_json::to_vec(&projection)?,
            )?;
            Ok(Vec::new())
        } else if let Some(job) = &request.office {
            let input = std::fs::read(&request.path)?;
            let report = if job.readback {
                vz_core::office::inspect(&job.name, &input)?
            } else {
                let (bytes, report) = if job.sanitized_input {
                    vz_core::office::edit_with_chart_sources(
                        &job.name,
                        &input,
                        &job.edits,
                        &job.chart_sources,
                    )?
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
