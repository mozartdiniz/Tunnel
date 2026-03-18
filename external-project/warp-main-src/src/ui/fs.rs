use crate::error::UiError;
use crate::util::error::AppError;
use crate::{gettext, globals};
use futures::FutureExt;
use futures::{pin_mut, select};
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Path, PathBuf};

pub fn default_download_dir() -> Result<PathBuf, AppError> {
    if let Some(downloads) = glib::user_special_dir(glib::UserDirectory::Downloads) {
        Ok(downloads)
    } else {
        Err(UiError::new(&gettext(
            "Downloads dir missing. Please set XDG_DOWNLOAD_DIR",
        ))
        .into())
    }
}

pub async fn compress_folder_cancelable<F: 'static + Fn(usize, usize) + Send>(
    path: &Path,
    cancel_future: impl Future<Output = ()>,
    progress_callback: F,
) -> Result<tempfile::NamedTempFile, AppError> {
    let tar_path_future = compress_folder(path, progress_callback);
    let tar_path_future = tar_path_future.fuse();
    let cancel_future = cancel_future.fuse();

    pin_mut!(tar_path_future, cancel_future);

    select! {
        res = tar_path_future => {
            if res.is_ok() {
                log::debug!("Created archive");
            }

            res
        },
        () = cancel_future => {
            log::debug!("Archive creation canceled");
            // Canceled / Error: We drop the smol::Task at the end of this function which aborts it
            // The dropped NamedTempFile will be deleted as well
            Err(AppError::Canceled)
        }
    }
}

pub async fn compress_folder<F: 'static + Fn(usize, usize) + Send>(
    path: &Path,
    progress_callback: F,
) -> Result<tempfile::NamedTempFile, AppError> {
    let path = path.to_path_buf();
    assert!(path.is_dir(), "Wrong compress_folder invocation");

    let tmp_dir = &*globals::CACHE_DIR;
    std::fs::create_dir_all(tmp_dir)?;

    let zip_file = tempfile::Builder::new()
        .prefix("warp_archive_")
        .suffix(".zip")
        .tempfile_in(tmp_dir)?;

    let async_zip_file = smol::fs::File::from(zip_file.reopen()?);

    log::debug!("Creating archive: {}", zip_file.path().display());
    smol::spawn(async move {
        crate::util::zip::zip_dir(&path, async_zip_file, progress_callback).await?;
        Ok(zip_file)
    })
    .await
}

pub fn safe_persist_tempfile(
    temp_file: tempfile::NamedTempFile,
    filename: &Path,
) -> std::io::Result<PathBuf> {
    let mut temp_path = temp_file.into_temp_path();
    let orig_file_stem: String = filename
        .file_stem()
        .map(OsStr::to_string_lossy)
        .map_or("Downloaded File".to_owned(), Cow::into_owned);

    let file_ext: String = filename
        .extension()
        .unwrap_or(&OsString::from("bin"))
        .to_string_lossy()
        .into_owned();

    let dir = temp_path
        .parent()
        .map_or(PathBuf::from("."), ToOwned::to_owned);

    let mut i = 1;
    #[allow(clippy::redundant_clone)]
    let mut file_stem = orig_file_stem.clone();

    loop {
        let path = dir.join(PathBuf::from(format!("{file_stem}.{file_ext}")));

        match temp_path.persist_noclobber(&path) {
            Ok(()) => {
                return Ok(path);
            }
            Err(err) => {
                if err.error.kind() != std::io::ErrorKind::AlreadyExists {
                    log::error!("Error creating file '{}': {}", path.display(), err);
                    return Err(err.error);
                }

                file_stem = format!("{orig_file_stem} ({i})");
                i += 1;

                temp_path = err.path;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use super::safe_persist_tempfile;

    #[test]
    fn test_safe_persist_tempfile() {
        let filename = PathBuf::from("warp_test_safe_persist_tempfile.bin");
        let temp_dir = std::env::temp_dir();
        let mut to_remove = Vec::new();

        for file in std::fs::read_dir(&temp_dir).unwrap().flatten() {
            let path = file.path();
            if path.file_stem().is_some_and(|stem| {
                stem.to_string_lossy()
                    .starts_with(filename.file_stem().unwrap().to_string_lossy().as_ref())
            }) && path.extension().is_some_and(|ext| ext == "bin")
            {
                std::fs::remove_file(path).unwrap();
            }
        }

        for i in 0..10 {
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            let res = safe_persist_tempfile(temp_file, &filename);
            let path = res.unwrap();

            if i == 0 {
                assert_eq!(path, temp_dir.join(&filename));
            } else {
                assert_eq!(
                    path,
                    temp_dir.join(PathBuf::from(format!(
                        "{} ({i}).{}",
                        filename.file_stem().unwrap().to_string_lossy(),
                        filename.extension().unwrap().to_string_lossy()
                    )))
                );
            }

            to_remove.push(path);
        }

        for path in to_remove {
            std::fs::remove_file(path).unwrap();
        }
    }
}
