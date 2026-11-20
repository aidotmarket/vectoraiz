//! The reuse check trusts the staging readback and attests to the bytes it saw.
use crate::{index::ScanRow, pipeline::reader_sha256};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, ErrorKind, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const DIRECTORY: &str = ".vz-validation";
const READER_FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Origin {
    Validated,
    Adopted,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Seal {
    generation: String,
    source_hash: String,
    recipe_id: String,
    reader_format_version: u32,
    origin: Origin,
    nodes: BTreeMap<PathBuf, Node>,
    #[serde(default)]
    needs_ocr_pages: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Node {
    Directory,
    File {
        size: u64,
        mtime: (bool, u64, u32),
        sha256: String,
    },
}

#[derive(Debug)]
pub(crate) enum Unavailable {
    Damaged,
    Unknown(io::Error),
}

impl From<io::Error> for Unavailable {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            ErrorKind::NotFound | ErrorKind::NotADirectory => Self::Damaged,
            _ => Self::Unknown(error),
        }
    }
}

pub(crate) struct Reuse {
    pub needs_ocr_pages: Vec<u32>,
    pub update: Option<Seal>,
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn path(root: &Path, generation: &str) -> PathBuf {
    root.join(DIRECTORY).join(format!("{generation}.json"))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}

fn open_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "expected regular file",
        ));
    }
    Ok(file)
}

fn mtime(time: SystemTime) -> (bool, u64, u32) {
    let (before_epoch, elapsed) = match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => (false, elapsed),
        Err(error) => (true, error.duration()),
    };
    (before_epoch, elapsed.as_secs(), elapsed.subsec_nanos())
}

fn inventory(generation: &Path) -> Result<BTreeMap<PathBuf, Node>, Unavailable> {
    for directory in [generation.parent().unwrap(), generation] {
        let metadata = directory.symlink_metadata()?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(Unavailable::Damaged);
        }
    }
    let mut current = BTreeMap::new();
    let mut directories = vec![generation.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let path = entry.path();
            let relative = path.strip_prefix(generation).unwrap().to_owned();
            let node = if is_link(&metadata) {
                return Err(Unavailable::Damaged);
            } else if metadata.is_dir() {
                directories.push(path);
                Node::Directory
            } else if metadata.is_file() {
                Node::File {
                    size: metadata.len(),
                    mtime: mtime(metadata.modified()?),
                    sha256: String::new(),
                }
            } else {
                return Err(Unavailable::Damaged);
            };
            current.insert(relative, node);
        }
    }
    Ok(current)
}

fn read(root: &Path, generation: &str) -> io::Result<Option<Seal>> {
    let directory = root.join(DIRECTORY);
    match directory.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
        Ok(_) => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "expected seal directory",
            ))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    match open_file(&path(root, generation)) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(serde_json::from_slice(&bytes).ok())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn check(root: &Path, row: &ScanRow, recipe: &str) -> Result<Reuse, Unavailable> {
    if !valid_id(&row.generation) {
        return Err(Unavailable::Damaged);
    }
    let old = read(root, &row.generation).map_err(Unavailable::Unknown)?;
    if old.as_ref().is_some_and(|seal| {
        seal.generation != row.generation
            || seal.source_hash != row.hash
            || seal.recipe_id != recipe
    }) {
        return Err(Unavailable::Damaged);
    }

    let generation = root.join("generations").join(&row.generation);
    let mut nodes = inventory(&generation)?;
    let mut refresh = true;
    if let Some(seal) = &old {
        // Check the entire layout and all lengths before opening any file.
        if nodes.len() != seal.nodes.len()
            || nodes
                .iter()
                .any(|(name, node)| match (node, seal.nodes.get(name)) {
                    (Node::Directory, Some(Node::Directory)) => false,
                    (Node::File { size, .. }, Some(Node::File { size: old, .. })) => size != old,
                    _ => true,
                })
        {
            return Err(Unavailable::Damaged);
        }
        refresh = seal.reader_format_version != READER_FORMAT_VERSION;
        let rehash_all = refresh;
        for (name, node) in &mut nodes {
            if let (
                Node::File { mtime, sha256, .. },
                Node::File {
                    mtime: old_time,
                    sha256: old_hash,
                    ..
                },
            ) = (node, &seal.nodes[name])
            {
                if rehash_all || mtime != old_time {
                    refresh = true;
                    *sha256 = hash_file(&generation.join(name))?;
                    if sha256 != old_hash {
                        return Err(Unavailable::Damaged);
                    }
                } else {
                    sha256.clone_from(old_hash);
                }
            }
        }
    } else {
        hash_files(&generation, &mut nodes)?;
    }

    let (origin, needs_ocr_pages) = old
        .map(|seal| (seal.origin, seal.needs_ocr_pages))
        .unwrap_or((Origin::Adopted, Vec::new()));
    let update = refresh.then(|| {
        Seal::new(
            &row.generation,
            &row.hash,
            recipe,
            nodes,
            origin,
            needs_ocr_pages.clone(),
        )
    });
    Ok(Reuse {
        update,
        needs_ocr_pages,
    })
}

#[cfg(test)]
thread_local! {
    pub(crate) static HASHED_FILES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn hash_file(path: &Path) -> Result<String, Unavailable> {
    #[cfg(test)]
    HASHED_FILES.with(|count| count.set(count.get() + 1));
    Ok(reader_sha256(open_file(path)?)?)
}

fn hash_files(generation: &Path, nodes: &mut BTreeMap<PathBuf, Node>) -> Result<(), Unavailable> {
    for (name, node) in nodes {
        if let Node::File { sha256, .. } = node {
            *sha256 = hash_file(&generation.join(name))?;
        }
    }
    Ok(())
}

impl Seal {
    fn new(
        generation: &str,
        source_hash: &str,
        recipe_id: &str,
        nodes: BTreeMap<PathBuf, Node>,
        origin: Origin,
        needs_ocr_pages: Vec<u32>,
    ) -> Self {
        Self {
            generation: generation.to_owned(),
            source_hash: source_hash.to_owned(),
            recipe_id: recipe_id.to_owned(),
            reader_format_version: READER_FORMAT_VERSION,
            origin,
            nodes,
            needs_ocr_pages,
        }
    }

    pub(crate) fn validated(
        root: &Path,
        generation: &str,
        recipe: &str,
        doc: &crate::pipeline::Document,
    ) -> anyhow::Result<()> {
        let directory = root.join("generations").join(generation);
        let observe = || {
            let mut nodes = inventory(&directory)?;
            hash_files(&directory, &mut nodes)?;
            Ok(nodes)
        };
        let nodes = observe().map_err(|error| match error {
            Unavailable::Damaged => anyhow::anyhow!("staged generation changed after readback"),
            Unavailable::Unknown(error) => anyhow::Error::from(error),
        })?;
        Self::new(
            generation,
            &doc.sha256,
            recipe,
            nodes,
            Origin::Validated,
            doc.needs_ocr_pages.clone(),
        )
        .write(root)?;
        sync_directory(&root.join(DIRECTORY))?;
        Ok(())
    }

    pub(crate) fn write(&self, root: &Path) -> io::Result<()> {
        let directory = prepare_directory(root)?;
        let mut file = tempfile::Builder::new()
            .prefix("seal-")
            .suffix(".tmp")
            .tempfile_in(&directory)?;
        serde_json::to_writer(&mut file, self)?;
        file.as_file().sync_all()?;
        file.persist(path(root, &self.generation))?;
        Ok(())
    }
}

fn prepare_directory(root: &Path) -> io::Result<PathBuf> {
    let directory = root.join(DIRECTORY);
    match fs::create_dir(&directory) {
        Ok(()) => sync_directory(root)?,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = directory.symlink_metadata()?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "expected seal directory",
        ));
    }
    Ok(directory)
}

pub(crate) fn install(from: &Path, to: &Path, generation: &str) -> io::Result<()> {
    let source = path(from, generation);
    // An adopted workspace generation can be reused after a seal write failure.
    match source.symlink_metadata() {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Ok(metadata) if metadata.is_file() && !is_link(&metadata) => {}
        Ok(_) => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "expected generation seal",
            ))
        }
        Err(error) => return Err(error),
    }
    let directory = prepare_directory(to)?;
    fs::rename(source, path(to, generation))?;
    if let Err(error) = sync_directory(&directory) {
        fs::rename(path(to, generation), path(from, generation))?;
        return Err(error);
    }
    Ok(())
}

fn sync_directory(_directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(_directory)?.sync_all()?;
    Ok(())
}

pub(crate) fn recover(root: &Path, active: &std::collections::HashSet<&str>) -> io::Result<()> {
    let directory = root.join(DIRECTORY);
    // Access to seals is optional for cleanup of the generations themselves.
    let entries = match directory.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => fs::read_dir(&directory),
        Ok(_) => return Ok(()),
        Err(error) => Err(error),
    };
    let entries = match entries {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let generation = name.strip_suffix(".json");
        if generation.is_some_and(|id| active.contains(id)) {
            continue;
        }
        if generation.is_some() || name.ends_with(".tmp") {
            crate::storage::remove_index_node(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
