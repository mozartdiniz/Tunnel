mod action_view;
pub mod application;
mod fs;
mod licenses;
mod preferences;
mod pride;
mod progress;
mod welcome_dialog;
pub mod window;

#[cfg(feature = "qr_code_scanning")]
mod camera;
#[cfg(feature = "qr_code_scanning")]
mod camera_row;
