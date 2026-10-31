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

/// Any source I/O failure can indicate unavailable storage; retry without publishing.
pub fn source_io(error: Error) -> Error {
    infrastructure(error)
}

/// Parser format/decoding failures are content errors. Infrastructure marked at
/// dependency boundaries and non-file-specific I/O must survive the protocol.
pub fn parser(error: Error) -> Error {
    if error.downcast_ref::<InfrastructureFailure>().is_some() {
        return error;
    }
    // PdfiumError deliberately exposes no source(), including for wrapped I/O.
    // Enumerate known document exclusions; unknown/internal/resource errors fail closed.
    if let Some(pdf) = error.downcast_ref::<pdfium_render::prelude::PdfiumError>() {
        use pdfium_render::prelude::{PdfiumError as P, PdfiumInternalError as I};
        return if matches!(
            pdf,
            P::PdfiumLibraryInternalError(I::FormatError | I::PasswordError | I::SecurityError)
                | P::NoPagesInDocument
                | P::ImageSizeOutOfBounds
        ) {
            content(format!("{error:#}"))
        } else {
            infrastructure(error)
        };
    }
    if error.chain().any(|cause| cause.is::<std::io::Error>()) {
        return source_io(error);
    }
    content(format!("{error:#}"))
}

/// PDFium errors raised after the document loaded belong to that document (a
/// damaged page), except I/O, which can still mean unavailable storage.
pub fn pdf_page(error: pdfium_render::prelude::PdfiumError) -> Error {
    match error {
        pdfium_render::prelude::PdfiumError::IoError(_) => infrastructure(error.into()),
        _ => content(format!("PDF page failed: {error:#?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_errors_after_load_are_document_failures_except_io() {
        use pdfium_render::prelude::{PdfiumError as P, PdfiumInternalError as I};
        for error in [
            P::PdfiumLibraryInternalError(I::PageError),
            P::PdfiumLibraryInternalError(I::Unknown),
            P::PdfiumFunctionReturnValueIndicatedFailure,
        ] {
            assert!(is_content(&pdf_page(error)));
        }
        assert!(!is_content(&pdf_page(P::IoError(std::io::Error::other(
            "disk gone"
        )))));
    }
    #[test]
    fn classification_is_explicit_and_io_fails_closed() {
        assert!(is_content(&parser(anyhow::anyhow!("corrupt ZIP"))));
        assert!(!is_content(&source_io(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
        )));
        // ZIP and workbook errors wrap I/O errors in their own error types.
        let device = zip::result::ZipError::Io(std::io::Error::from(std::io::ErrorKind::Other));
        assert!(!is_content(&parser(device.into())));
        let unreadable =
            zip::result::ZipError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(!is_content(&parser(unreadable.into())));
        use pdfium_render::prelude::{PdfiumError as P, PdfiumInternalError as I};
        for pdf in [
            P::IoError(std::io::Error::from(std::io::ErrorKind::Other)),
            P::PdfiumLibraryInternalError(I::FileError),
            P::PdfiumLibraryInternalError(I::Unknown),
            P::PdfiumLibraryInternalError(I::PageError),
            P::PdfiumFunctionReturnValueIndicatedFailure,
            P::DataBufferLengthMismatch,
        ] {
            assert!(!is_content(&parser(pdf.into())));
        }
        for internal in [I::FormatError, I::PasswordError, I::SecurityError] {
            assert!(is_content(&parser(
                P::PdfiumLibraryInternalError(internal).into()
            )));
        }
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
