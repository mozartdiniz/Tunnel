use super::*;

/// From <https://gitlab.gnome.org/World/pika-backup/-/blob/main/src/ui/utils/duration.rs>
pub fn left(done_bytes: usize, total_bytes: usize, d: &chrono::Duration) -> String {
    let bytes_str = glib::format_size(done_bytes as u64);
    let total_str = glib::format_size(total_bytes as u64);

    let progress = pgettextf(
        "File size transferred",
        // Translators: {0} = file size transferred, {1} = total file size, Example: 17.3MB / 20.5MB
        "{0} / {1}",
        &[&bytes_str, &total_str],
    );

    let time_remaining = if d.num_minutes() < 2 {
        ngettextf(
            // Translators: File transfer time left
            "One second left",
            "{} seconds left",
            (d.num_seconds() + 1) as u32,
            &[&(d.num_seconds() + 1).to_string()],
        )
    } else if d.num_hours() < 2 {
        ngettextf(
            // Translators: File transfer time left
            "One minute left",
            "{} minutes left",
            (d.num_minutes() + 1) as u32,
            &[&(d.num_minutes() + 1).to_string()],
        )
    } else if d.num_days() < 2 {
        ngettextf(
            // Translators: File transfer time left
            "One hour left",
            "{} hours left",
            (d.num_hours() + 1) as u32,
            &[&(d.num_hours() + 1).to_string()],
        )
    } else {
        ngettextf(
            // Translators: File transfer time left
            "One day left",
            "{} days left",
            (d.num_days() + 1) as u32,
            &[&(d.num_days() + 1).to_string()],
        )
    };

    pgettextf(
        "Combine bytes progress {0} and time remaining {1}",
        // Translators: {0} = 11.3MB / 20.7MB, {1} = 3 seconds left
        "{0} — {1}",
        &[&progress, &time_remaining],
    )
}
