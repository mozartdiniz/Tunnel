pub use gettextrs::*;
use std::fmt::Display;

pub mod duration;

fn fmt(mut format: String, args: &[&dyn Display]) -> String {
    for arg in args {
        format = format.replacen("{}", &arg.to_string(), 1);
    }

    for (i, arg) in args.iter().enumerate() {
        format = format.replace(&format!("{{{i}}}"), &arg.to_string());
    }

    format
}

pub fn gettextf(msgid: &str, args: &[&dyn Display]) -> String {
    fmt(gettext(msgid), args)
}

pub fn pgettextf(msgctxt: &str, msgid: &str, args: &[&dyn Display]) -> String {
    fmt(pgettext(msgctxt, msgid), args)
}

pub fn ngettextf(msgid: &str, msgid_plural: &str, n: u32, args: &[&dyn Display]) -> String {
    fmt(ngettext(msgid, msgid_plural, n), args)
}

pub fn ngettextf_(msgid: &str, msgid_plural: &str, n: u32) -> String {
    ngettextf(msgid, msgid_plural, n, &[&n])
}

pub fn npgettextf(
    msgctxt: &str,
    msgid: &str,
    msgid_plural: &str,
    n: u32,
    args: &[&dyn Display],
) -> String {
    fmt(npgettext(msgctxt, msgid, msgid_plural, n), args)
}
