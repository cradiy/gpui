use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "{} is not a regular file",
        path.display()
    );
    ensure!(
        metadata.len() <= limit as u64,
        "{} exceeds {limit} bytes",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "{} grew beyond {limit} bytes",
        path.display()
    );
    Ok(bytes)
}

pub fn resource_path(root: &Path, uri: &str) -> Result<PathBuf> {
    ensure!(
        !uri.starts_with('/') && !uri.contains(['\\', ':']),
        "only relative local resource URIs are accepted: {uri}"
    );
    let base = url::Url::from_directory_path(root)
        .map_err(|_| anyhow::anyhow!("invalid asset directory"))?;
    let resolved = base.join(uri).context("resource URI")?;
    ensure!(
        resolved.query().is_none() && resolved.fragment().is_none(),
        "resource query/fragment is unsupported: {uri}"
    );
    let path = resolved
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("resource is not a local file: {uri}"))?
        .canonicalize()
        .with_context(|| format!("resource {uri}"))?;
    ensure!(
        path.starts_with(root),
        "resource leaves the asset directory: {uri}"
    );
    Ok(path)
}
