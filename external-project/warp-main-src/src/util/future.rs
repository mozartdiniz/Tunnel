use crate::error;
use crate::ui::application::WarpApplication;
use crate::util::error::AppError;
use futures::{FutureExt, pin_mut, select};
use std::future::Future;

/// Spawn a future on an async task
pub async fn spawn_async<T>(
    func: impl Future<Output = Result<T, impl Into<AppError> + Send + 'static>> + 'static + Send,
) -> Result<T, AppError>
where
    T: 'static + Send,
{
    let task = smol::spawn(func);
    task.catch_unwind()
        .await
        .map_err(|_| error::error_for_panic())?
        .map_err(Into::into)
}

pub fn invoke_main_with_app<F>(func: F)
where
    F: FnOnce(WarpApplication) + 'static + Send,
{
    glib::MainContext::default().invoke(|| func(WarpApplication::default()));
}

pub async fn block_on_main_with_app_async<R>(
    func: impl FnOnce(WarpApplication) -> R + 'static + Send,
) -> R
where
    R: 'static + Send,
{
    let (sender, receiver) = async_channel::bounded(1);
    glib::MainContext::default().invoke(move || {
        sender
            .send_blocking(func(WarpApplication::default()))
            .unwrap();
    });

    receiver
        .recv()
        .await
        .expect("async_channel is supposed to work")
}

/// Run a future from main thread with error handling
pub fn main_async_local<F, E>(error_handler: E, func: F)
where
    F: Future<Output = Result<(), AppError>> + 'static,
    E: FnOnce(AppError) + 'static,
{
    glib::MainContext::default().spawn_local(async move {
        match func.await {
            Ok(()) => (),
            Err(app_error) => error_handler(app_error),
        }
    });
}

/// Run a future from main thread without error handling
pub fn main_async_local_infallible<F>(func: F)
where
    F: Future<Output = ()> + 'static,
{
    glib::MainContext::default().spawn_local(func);
}

pub async fn cancelable_future<T>(
    future: impl Future<Output = T>,
    cancel_future: impl Future<Output = ()>,
) -> Result<T, AppError> {
    let future = future.fuse();
    let cancel_future = cancel_future.fuse();

    pin_mut!(future, cancel_future);

    select! {
        res = future => {
            Ok(res)
        },
        () = cancel_future => {
            log::debug!("Future canceled");
            Err(AppError::Canceled)
        }
    }
}
