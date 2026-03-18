use crate::gettext::gettextf;
use crate::gettext::*;
use crate::globals;
use crate::ui::application::WarpApplication;
use crate::ui::window::WarpApplicationWindow;
use adw::prelude::*;
use async_zip::error::ZipError;
use std::fmt::{Debug, Display, Formatter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use wormhole::ParseCodeError;
use wormhole::ParsePasswordError;
use wormhole::WormholeError;
use wormhole::rendezvous::RendezvousError;
use wormhole::transfer::TransferError;
use wormhole::transit::RelayHintParseError;

#[derive(thiserror::Error, Debug)]
pub struct UiError {
    msg: String,
}

#[allow(dead_code)]
impl UiError {
    pub fn new(msg: &impl ToString) -> Self {
        Self {
            msg: msg.to_string(),
        }
    }

    pub fn handle(self) {
        AppError::from(self).handle();
    }
}

impl Display for UiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    Canceled,
    Io {
        #[from]
        source: std::io::Error,
    },
    Url {
        #[from]
        source: url::ParseError,
    },
    Transfer {
        #[from]
        source: TransferError,
    },
    Wormhole {
        #[from]
        source: WormholeError,
    },
    WormholeRelayHintParseError {
        #[from]
        source: RelayHintParseError,
    },
    WormholeParseCodeError {
        #[from]
        source: ParseCodeError,
    },
    Ui {
        #[from]
        source: UiError,
    },
    AsyncChannelRecvError {
        #[from]
        source: async_channel::RecvError,
    },
    AsyncBroadcastRecvError {
        #[from]
        source: async_broadcast::RecvError,
    },
    Zip {
        #[from]
        source: ZipError,
    },
    Panic {
        msg: String,
    },
    Glib {
        #[from]
        source: glib::Error,
    },
    #[cfg(target_os = "linux")]
    Ashpd {
        #[from]
        source: ashpd::Error,
    },
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Canceled => write!(f, "Operation was canceled by the user"),
            AppError::Io { source } => write!(f, "I/O Error: {source}"),
            AppError::Url { source } => write!(f, "URL ParseError: {source}"),
            AppError::Transfer { source } => write!(f, "TransferError: {source:?}"),
            AppError::Wormhole { source } => write!(f, "WormholeError: {source:?}"),
            AppError::WormholeRelayHintParseError { source } => {
                write!(f, "WormholeRelayHintParseError: {source:?}")
            }
            AppError::WormholeParseCodeError { source } => {
                write!(f, "WormholeParseCodeError: {source:?}")
            }
            AppError::Ui { source } => write!(f, "UiError: {source}"),
            AppError::AsyncChannelRecvError { source } => {
                write!(f, "AsyncChannelRecvError: {source}")
            }
            AppError::AsyncBroadcastRecvError { source } => {
                write!(f, "AsyncBroadcastRecvError: {source}")
            }
            AppError::Zip { source } => write!(f, "ZipError: {source}"),
            AppError::Panic { msg } => write!(f, "Panic: {msg}"),
            AppError::Glib { source } => write!(f, "Glib: {source}"),
            #[cfg(target_os = "linux")]
            AppError::Ashpd { source } => write!(f, "Ashpd: {source}"),
        }
    }
}

// Don't show more than one error dialog at the same time, it will get very annoying
static ERROR_DIALOG_ALREADY_SHOWING: AtomicBool = AtomicBool::new(false);

impl AppError {
    #[allow(clippy::unnecessary_wraps)]
    pub fn ok<R>(res: R) -> Result<R, AppError> {
        Ok(res)
    }

    pub fn is_user_canceled(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            matches!(
                self,
                AppError::Canceled
                    | AppError::Ashpd {
                        source: ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
                    }
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            matches!(self, AppError::Canceled)
        }
    }

    pub fn handle(self) {
        if self.is_user_canceled() {
            // Don't do anything here, the user canceled the operation
            return;
        }

        log::error!("{:?}", self);

        if gtk::is_initialized() {
            let window: Option<WarpApplicationWindow> = match gio::Application::default() {
                Some(app) => match app.downcast::<gtk::Application>() {
                    Ok(app) => {
                        let windows = app.windows();
                        windows
                            .first()
                            .cloned()
                            .and_then(|window| window.downcast().ok())
                    }
                    Err(_) => None,
                },
                _ => None,
            };

            if let Some(window) = window {
                if window.is_visible() {
                    self.show_error_dialog(&window);
                    return; // DON'T PANIC
                }
            }
        }

        // We can just panic if we can't display a proper error in the UI
        panic!("An error occurred during application initialisation: {self}",);
    }

    pub fn show_error_dialog(&self, window: &WarpApplicationWindow) {
        let msg1 = &gettext("An error occurred");
        let msg2 = self.gettext_error();

        let res = ERROR_DIALOG_ALREADY_SHOWING.compare_exchange(
            false,
            true,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        if res.is_ok() {
            let dialog = adw::AlertDialog::builder()
                .heading(msg1)
                .body(msg2)
                .close_response("close")
                .build();

            dialog.add_response("close", &gettext("_Close"));

            dialog.choose(window, gio::Cancellable::NONE, |_response| {
                ERROR_DIALOG_ALREADY_SHOWING.store(false, Ordering::SeqCst);
            });
        }
    }

    fn gettext_error_wormhole(wormhole_error: &WormholeError) -> String {
        match wormhole_error {
            WormholeError::ProtocolJson(_) | WormholeError::Protocol(_) => {
                gettext("Corrupt or unexpected message received")
            }
            WormholeError::ServerError(err) => {
                if let RendezvousError::Server(msg) = err {
                    if &**msg == "crowded" {
                        gettext(
                            "The rendezvous server will not allow further connections for this code. A new code needs to be generated.",
                        )
                    } else if &**msg == "pruney" {
                        gettext(
                            "The rendezvous server removed the code due to inactivity. A new code needs to be generated.",
                        )
                    } else {
                        gettextf(
                            "The rendezvous server responded with an unknown message: {}",
                            &[msg],
                        )
                    }
                } else if WarpApplication::default()
                    .main_window()
                    .config()
                    .rendezvous_server_url
                    .is_some()
                {
                    gettext(
                        "Error connecting to the rendezvous server.\nYou have entered a custom rendezvous server URL in preferences. Please verify the URL is correct and the server is working.",
                    )
                } else {
                    gettext(
                        "Error connecting to the rendezvous server.\nPlease try again later / verify you are connected to the internet.",
                    )
                }
            }
            WormholeError::PakeFailed => gettext(
                "Encryption key confirmation failed. If you or your peer didn't mistype the code, this is a sign of an attacker guessing passwords. Please try again some time later.",
            ),
            WormholeError::Crypto => gettext("Cannot decrypt a received message"),
            _ => gettext("An unknown error occurred"),
        }
    }

    fn gettext_error_wormhole_parse_code(error: &ParseCodeError) -> String {
        match error {
            // Translators: magic-wormhole codes usually look like n-word-word... where n MUST be an arabic number (unlimited digits). The first '-' is mandatory. This error occurs when an empty code was entered.
            ParseCodeError::Empty => gettext("The specified code must not be empty"),
            // Translators: magic-wormhole codes usually look like n-word-word... where n MUST be an arabic number (unlimited digits). The first '-' is mandatory. This error occurs when a code was entered that does not contain any '-'.
            ParseCodeError::SeparatorMissing => gettext("The code must contain at least one “-”"),
            // Translators: magic-wormhole codes usually look like n-word-word... where n MUST be an arabic number (unlimited digits). This error occurs when a code was entered that does not contain a number before the '-'.
            ParseCodeError::Nameplate(_) => gettext("The first segment of a code must be a number"),
            ParseCodeError::Password(ParsePasswordError::LittleEntropy { value, .. }) => {
                // Translators: A code is entered that is so weak that it poses a potential security risk.
                ngettextf_(
                    "Code too weak. It can be guessed with an average of {} try.",
                    "Code too weak. It can be guessed with an average of {} tries.",
                    *value as u32,
                )
            }
            ParseCodeError::Password(ParsePasswordError::TooShort { required, .. }) => {
                // Translators: A code is entered that is so short that it poses a potential security risk.
                ngettextf_(
                    "Code too short. The text after the “-” must be at least {} character long.",
                    "Code too short. The text after the “-” must be at least {} characters long.",
                    *required as u32,
                )
            }
            err => {
                log::debug!("{err:?}");
                gettext("Code too weak")
            }
        }
    }

    fn gettext_error_io(io_error: &std::io::Error) -> String {
        match io_error.kind() {
            std::io::ErrorKind::NotFound => gettext("File / Directory not found"),
            std::io::ErrorKind::PermissionDenied => gettext("Permission denied"),
            _ => io_error.to_string(),
        }
    }

    #[cfg(target_os = "linux")]
    fn gettext_error_ashpd(error: &ashpd::Error) -> String {
        match error {
            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => gettext("Canceled"),
            ashpd::Error::Portal(ashpd::PortalError::NotAllowed(_)) => {
                gettext("Portal service response: Permission denied")
            }
            ashpd::Error::Portal(ashpd::PortalError::Failed(err)) => {
                gettextf("The portal service has failed: {}", &[err])
            }
            ashpd::Error::Portal(ashpd::PortalError::ZBus(zbus)) => gettextf(
                "Error communicating with the portal service via zbus: {}",
                &[&format!("{zbus:?}")],
            ),
            ashpd::Error::Portal(err) => gettextf("Portal error: {}", &[err]),
            _ => gettextf("Portal error: {}", &[&error]),
        }
    }

    pub fn gettext_error(&self) -> String {
        match self {
            AppError::Canceled => "canceled".to_owned(),
            AppError::Io { source } => Self::gettext_error_io(source),
            // TODO those should not appear publicly
            AppError::Url { source } => source.to_string(),
            AppError::Transfer { source } => match source {
                TransferError::AckError => gettext("Transfer was not acknowledged by peer"),
                TransferError::Checksum | TransferError::FilesystemSkew => {
                    gettext("The received file is corrupted")
                }
                TransferError::FileSize {
                    sent_size,
                    file_size,
                } => gettextf(
                    "The file contained a different amount of bytes than advertised! Sent {} bytes, but should have been {}",
                    &[sent_size, &file_size],
                ),
                TransferError::PeerError(msg) => {
                    if msg == "Task has been cancelled" {
                        gettext("The other side has cancelled the transfer")
                    } else if msg == "transfer rejected" {
                        gettext("The other side has rejected the transfer")
                    } else {
                        gettextf("Something went wrong on the other side: {}", &[msg])
                    }
                }
                TransferError::UnsupportedOffer
                | TransferError::ProtocolJson(_)
                | TransferError::ProtocolMsgpack(_)
                | TransferError::Protocol(_)
                | TransferError::ProtocolUnexpectedMessage(_, _) => {
                    gettext("Corrupt or unexpected message received")
                }
                TransferError::Wormhole(source) => {
                    log::error!("Wormhole error: {:?}", source);
                    Self::gettext_error_wormhole(source)
                }
                TransferError::TransitConnect(_) => {
                    gettext("Error while establishing file transfer connection")
                }
                TransferError::Transit(_) => gettext("Unknown file transfer error"),
                TransferError::IO(source) => Self::gettext_error_io(source),
                _ => gettext("An unknown error occurred"),
            },
            AppError::Wormhole { source } => Self::gettext_error_wormhole(source),
            AppError::WormholeRelayHintParseError { source } => source.to_string(),
            AppError::WormholeParseCodeError { source } => {
                Self::gettext_error_wormhole_parse_code(source)
            }
            // UIErrors are generated by our code and already wrapped in gettext
            AppError::Ui { source } => source.to_string(),
            AppError::AsyncChannelRecvError { .. } | AppError::AsyncBroadcastRecvError { .. } => {
                gettext("An unknown error occurred")
            }
            AppError::Zip { source } => gettextf(
                "An unknown error occurred while creating a zip file: {}",
                &[source],
            ),
            AppError::Panic { .. } => gettext(
                "An unexpected error occurred. Please report an issue with the error message.",
            ),
            AppError::Glib { source } => source.to_string(),
            #[cfg(target_os = "linux")]
            AppError::Ashpd { source } => Self::gettext_error_ashpd(source),
        }
    }
}

pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let location = panic_info.location().unwrap();
        let msg = match panic_info.payload().downcast_ref::<&'static str>() {
            Some(s) => *s,
            None => match panic_info.payload().downcast_ref::<String>() {
                Some(s) => &s[..],
                None => "Box<dyn Any>",
            },
        }
        .to_owned();

        let backtrace = backtrace::Backtrace::new();
        let info_msg = format!("thread '{name}' panicked at '{msg}', {location}\n{backtrace:?}");

        globals::PANIC_BACKTRACES.lock().unwrap().push(info_msg);
        default_hook(panic_info);
    }));
}

pub fn error_for_panic() -> AppError {
    let mut msg = String::new();

    let mut backtrace_info = globals::PANIC_BACKTRACES.lock().unwrap();
    for backtrace_msg in backtrace_info.iter() {
        let _ignored = writeln!(msg, "{backtrace_msg}");
    }

    backtrace_info.clear();

    if msg.is_empty() {
        "Unknown panic cause".clone_into(&mut msg);
    }

    AppError::Panic { msg }
}
