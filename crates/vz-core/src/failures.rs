//! Fail closed: only explicit document-content errors permit migration publication.
use anyhow::Error;

#[derive(Debug)]
struct ContentFailure(String);
impl std::fmt::Display for ContentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ContentFailure {}

#[derive(Debug)]
struct InfrastructureFailure;
impl std::fmt::Display for InfrastructureFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("parser infrastructure failure")
    }
}
impl std::error::Error for InfrastructureFailure {}

pub fn content(reason: impl Into<String>) -> Error {
    Error::new(ContentFailure(reason.into()))
}
pub fn is_content(error: &Error) -> bool {
    error.downcast_ref::<ContentFailure>().is_some()
}
pub fn infrastructure(error: Error) -> Error {
    error.context(InfrastructureFailure)
}

/// Source reads may fail because a particular file is unreadable. Other I/O
/// failures (including storage exhaustion/device errors) remain infrastructure.
pub fn source_io(error: Error) -> Error {
    if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
        matches!(
            e.kind(),
            std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::NotFound
                | std::io::ErrorKind::InvalidData
                | std::io::ErrorKind::UnexpectedEof
        )
    }) {
        content(format!("{error:#}"))
    } else {
        error
    }
}

/// Parser format/decoding failures are content errors. Infrastructure marked at
/// dependency boundaries and non-file-specific I/O must survive the protocol.
pub fn parser(error: Error) -> Error {
    if error.downcast_ref::<InfrastructureFailure>().is_some() {
        return error;
    }
    if error.downcast_ref::<std::io::Error>().is_some() {
        return source_io(error);
    }
    content(format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_is_explicit_and_io_fails_closed() {
        assert!(is_content(&parser(anyhow::anyhow!("corrupt ZIP"))));
        assert!(is_content(&source_io(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
        )));
        for error in [
            anyhow::anyhow!("worker crashed"),
            anyhow::anyhow!("worker timeout"),
            anyhow::anyhow!("model failed"),
            std::io::Error::from(std::io::ErrorKind::StorageFull).into(),
            parser(infrastructure(anyhow::anyhow!("PDFium load failed"))),
        ] {
            assert!(!is_content(&error));
        }
    }
}
