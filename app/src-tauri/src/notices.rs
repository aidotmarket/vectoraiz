use std::{fs::File, io::Read, path::Path};

const MAX_NOTICES_BYTES: u64 = 8 * 1024 * 1024;

pub fn read(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let too_large = || "Third-party notices exceed the 8 MiB size limit".to_string();
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_NOTICES_BYTES {
        return Err(too_large());
    }
    // Bound the read too, in case the resource grows after the metadata check.
    let mut contents = String::new();
    file.take(MAX_NOTICES_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(|e| e.to_string())?;
    if contents.len() as u64 > MAX_NOTICES_BYTES {
        return Err(too_large());
    }
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_notices_are_preserved() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let contents = "Bundled dependency notices\r\nCopyright © contributors\n";
        std::fs::write(file.path(), contents).unwrap();
        assert_eq!(read(file.path()).unwrap(), contents);
    }

    #[test]
    fn notices_at_the_limit_are_allowed() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(MAX_NOTICES_BYTES).unwrap();
        assert_eq!(read(file.path()).unwrap().len() as u64, MAX_NOTICES_BYTES);
    }

    #[test]
    fn oversized_notices_have_a_clear_error() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(MAX_NOTICES_BYTES + 1).unwrap();
        assert_eq!(
            read(file.path()).unwrap_err(),
            "Third-party notices exceed the 8 MiB size limit"
        );
    }
}
