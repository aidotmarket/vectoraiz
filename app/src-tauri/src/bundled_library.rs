use std::{fs, io, path::Path};

/// Refresh only the runtime library; leave user models and index data untouched.
pub(crate) fn refresh(bundled: &Path, installed: &Path) -> io::Result<()> {
    let bundled_size = fs::metadata(bundled)?.len();
    match fs::metadata(installed) {
        Ok(metadata)
            if metadata.len() == bundled_size
                && vz_core::pipeline::file_sha256(bundled)?
                    == vz_core::pipeline::file_sha256(installed)? =>
        {
            return Ok(());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = installed.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(bundled, installed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_missing_and_changed_library_preserving_user_data() {
        let root = std::env::temp_dir().join(format!("vz-pdfium-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let bundled = root.join("bundled");
        let installed = root.join("models/pdfium/library");
        let sentinel = root.join("models/user-model");
        fs::write(&bundled, b"first").unwrap();
        refresh(&bundled, &installed).unwrap();
        fs::write(&sentinel, b"user data").unwrap();
        let modified = fs::metadata(&installed).unwrap().modified().unwrap();
        refresh(&bundled, &installed).unwrap();
        assert_eq!(
            fs::metadata(&installed).unwrap().modified().unwrap(),
            modified
        );
        for content in [b"other".as_slice(), b"different size".as_slice()] {
            fs::write(&bundled, content).unwrap();
            refresh(&bundled, &installed).unwrap();
            assert_eq!(fs::read(&installed).unwrap(), content);
            assert_eq!(fs::read(&sentinel).unwrap(), b"user data");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
