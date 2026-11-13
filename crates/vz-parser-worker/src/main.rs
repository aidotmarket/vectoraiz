//! One request per process. stdout is exclusively length-prefixed JSON.
fn main() {
    #[cfg(all(windows, feature = "office-feasibility"))]
    {
        windows_feasibility_main();
    }
    #[cfg(not(all(windows, feature = "office-feasibility")))]
    native_main();
}

#[cfg(not(all(windows, feature = "office-feasibility")))]
fn native_main() {
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

/// This entry point is compiled only into the explicitly opted-in test worker.
#[cfg(all(windows, feature = "office-feasibility"))]
fn windows_feasibility_main() {
    if std::env::args().nth(1).as_deref() == Some("--office-windows-child") {
        return;
    }
    let result = (|| -> anyhow::Result<serde_json::Value> {
        let value: serde_json::Value = vz_core::worker::read_frame(std::io::stdin().lock())?;
        if let Some(request) = value.get("office_windows_probe") {
            return vz_office_windows_harness::probes::run(request);
        }
        vz_core::worker::apply_memory_limit()?;
        let request = serde_json::from_value(value)?;
        let parsed = vz_core::parsers::parse(&request);
        let response = match parsed {
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
        Ok(serde_json::to_value(response)?)
    })();
    match result {
        Ok(response) => {
            if vz_core::worker::write_frame(std::io::stdout().lock(), &response).is_err() {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}
