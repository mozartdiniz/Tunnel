/// HTTP endpoint handlers for the LocalSend v2 receive side.
///
/// Each handler lives in its own submodule matching the route it serves.
mod cancel;
mod info;
mod prepare_upload;
mod upload;

pub use cancel::{handler_cancel, CancelParams};
pub use info::handler_device_info;
pub use prepare_upload::handler_prepare_upload;
pub use upload::{handler_upload, UploadParams};
