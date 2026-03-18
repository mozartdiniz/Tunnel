use std::path::{Path, PathBuf};

use anyhow::Result;

/// Zip a directory tree into a temp file. Returns `(temp_file, zip_size_bytes)`.
pub async fn zip_directory(dir_path: PathBuf) -> Result<(tempfile::NamedTempFile, u64)> {
    tokio::task::spawn_blocking(move || {
        let tmp = tempfile::NamedTempFile::new()?;
        {
            let file = tmp.as_file().try_clone()?;
            let mut zip = zip::ZipWriter::new(file);
            add_dir_to_zip(&dir_path, &dir_path, &mut zip)?;
            zip.finish()?;
        }
        let size = tmp.as_file().metadata()?.len();
        Ok((tmp, size))
    })
    .await?
}

pub fn add_dir_to_zip(
    base: &Path,
    dir: &Path,
    zip: &mut zip::ZipWriter<impl std::io::Write + std::io::Seek>,
) -> Result<()> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(base)?;
        let zip_name = rel.to_string_lossy();

        if zip_name.is_empty() {
            continue;
        }

        if path.is_dir() {
            zip.add_directory(zip_name.as_ref(), options)?;
        } else {
            zip.start_file(zip_name.as_ref(), options)?;
            let mut f = std::fs::File::open(path)?;
            std::io::copy(&mut f, zip)?;
        }
    }
    Ok(())
}
