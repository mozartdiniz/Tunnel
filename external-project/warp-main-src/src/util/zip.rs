use std::path::{Path, PathBuf};

use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
use futures::{AsyncReadExt, AsyncWrite, AsyncWriteExt, TryStreamExt};
use smol::fs::File;

use super::error::{AppError, UiError};

// See https://github.com/Majored/rs-async-zip/blob/main/examples/cli_compress.rs
async fn handle_directory<W: AsyncWrite + Unpin + Send + 'static, F: Fn(usize, usize)>(
    input_path: &Path,
    mut writer: ZipFileWriter<W>,
    callback: F,
) -> Result<(), AppError> {
    let mut num_files = 0;
    let mut size = 0;

    callback(0, 0);

    let entries = walk_dir(input_path.into()).await?;
    let input_dir_str = input_path
        .as_os_str()
        .to_str()
        .ok_or(UiError::new(&format!(
            "Input path \"{}\" not valid UTF-8.",
            input_path.display()
        )))?;

    for entry_path_buf in entries {
        let entry_path = entry_path_buf.as_path();
        let entry_str = entry_path
            .as_os_str()
            .to_str()
            .ok_or(UiError::new(&format!(
                "Directory file path \"{}\" not valid UTF-8.",
                entry_path.display()
            )))?;

        if !entry_str.starts_with(input_dir_str) {
            return Err(UiError::new(&format!(
                "Directory file path \"{entry_str}\" does not start with base input directory path."
            ))
            .into());
        }

        let entry_str = &entry_str[input_dir_str.len() + 1..];
        let res = write_entry(entry_str, entry_path, writer).await?;
        size += res.0;
        writer = res.1;

        num_files += 1;

        callback(num_files, size);
    }

    let mut inner = writer.close().await?;
    inner.flush().await?;
    inner.close().await?;

    Ok(())
}

async fn write_entry<W: AsyncWrite + Unpin + Send + 'static>(
    filename: &str,
    input_path: &Path,
    mut writer: ZipFileWriter<W>,
) -> Result<(usize, ZipFileWriter<W>), AppError> {
    let mut input_file = File::open(input_path).await?;
    let input_file_size = input_file.metadata().await?.len() as usize;

    // We need to do memory mapped I/O for big files. This means extra work, so we only do it for files > 10 MiB
    if input_file_size > 10 * 1024 * 1024 {
        let input_path = input_path.to_path_buf();
        let filename = filename.to_owned();
        log::trace!("Adding file {filename} to zip file via mmap");

        let result: Result<ZipFileWriter<W>, AppError> = smol::unblock(move || {
            let input_file = std::fs::File::open(input_path)?;

            // Safety: This is unsafe because the underlying file could change while it's being mapped
            // This would only lead to corrupt data.
            let mmap = unsafe { memmap2::MmapOptions::new().map(&input_file)? };

            let builder = ZipEntryBuilder::new(filename.into(), Compression::Stored);
            smol::block_on(writer.write_entry_whole(builder, &mmap))?;

            Ok(writer)
        })
        .await;

        Ok((input_file_size, result?))
    } else {
        let mut buffer = Vec::with_capacity(input_file_size);
        input_file.read_to_end(&mut buffer).await?;

        let builder = ZipEntryBuilder::new(filename.into(), Compression::Stored);
        writer.write_entry_whole(builder, &buffer).await?;
        Ok((input_file_size, writer))
    }
}

async fn walk_dir(dir: PathBuf) -> Result<Vec<PathBuf>, AppError> {
    let mut dirs = vec![dir];
    let mut files = vec![];

    while !dirs.is_empty() {
        let mut dir_iter = smol::fs::read_dir(dirs.remove(0)).await?;

        while let Some(entry) = dir_iter.try_next().await? {
            let entry_path_buf = entry.path();

            if entry_path_buf.is_dir() {
                dirs.push(entry_path_buf);
            } else {
                files.push(entry_path_buf);
            }
        }
    }

    Ok(files)
}

pub async fn zip_dir<W: AsyncWrite + Unpin + Send + 'static, F: Fn(usize, usize)>(
    dir: &Path,
    writer: W,
    callback: F,
) -> Result<(), AppError> {
    let output_writer = ZipFileWriter::new(writer);

    if !dir.is_dir() {
        return Err(UiError::new(&"Directory expected").into());
    }

    handle_directory(dir, output_writer, callback).await?;

    Ok(())
}
