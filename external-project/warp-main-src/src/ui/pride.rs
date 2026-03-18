use std::fmt::Display;

use chrono::prelude::*;
use gtk::prelude::*;

enum Season {
    /* Days */
    /// Intersex Awareness Day, Intersex Day Of Remembrance
    Intersex,
    /// International Lesbian Day, Lesbian Visibility Week
    Lesbian,
    /// World AIDS Day
    Aids,
    /// Autistic Pride Day
    Autism,
    /// Pansexual and Panromantic Awareness and Visibility Day
    Pan,

    /* Weeks */
    /// Trans Awareness Week / TDOV / TDOR
    Trans,
    // Aromantic Spectrum Awareness Week
    Aro,
    /// Ace Week
    Ace,
    /// Bisexual Awareness Week, Bi Visibility Day
    Bi,
    /// Non-Binary Awareness Week
    NonBinary,

    /* Months */
    /// Pride Month
    Pride,
    /// Disability Pride Month
    Disability,
    /// Black History Month
    BlackHistory,
}

impl Display for Season {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Season::Intersex => "intersex",
                Season::Lesbian => "lesbian",
                Season::Aids => "aids",
                Season::Autism => "autism",
                Season::Pan => "pan",
                Season::Trans => "trans",
                Season::Aro => "aro",
                Season::Ace => "ace",
                Season::Bi => "bi",
                Season::NonBinary => "non-binary",
                Season::Pride => "pride",
                Season::Disability => "disability",
                Season::BlackHistory => "black-history",
            }
        )
    }
}

impl Season {
    const fn all() -> &'static [Self] {
        &[
            Season::Intersex,
            Season::Lesbian,
            Season::Aids,
            Season::Autism,
            Season::Pan,
            Season::Trans,
            Season::Aro,
            Season::Ace,
            Season::Bi,
            Season::NonBinary,
            Season::Pride,
            Season::Disability,
            Season::BlackHistory,
        ]
    }

    fn is_season(&self, date: &chrono::DateTime<Local>) -> bool {
        match self {
            Season::Intersex => {
                (date.month() == 10 && date.day() == 26) || (date.month() == 11 && date.day() == 8)
            }
            Season::Lesbian => {
                (date.month() == 10 && date.day() == 8)
                    || (date.month() == 4 && date.day() >= 26)
                    || (date.month() == 5 && date.day() <= 2)
            }
            Season::Aids => date.month() == 12 && date.day() == 1,
            Season::Autism => date.month() == 6 && date.day() == 18,
            Season::Pan => date.month() == 5 && date.day() == 24,
            Season::Trans => {
                (date.month() == 11 && date.day() >= 13 && date.day() <= 19) // Awareness week
                    || (date.month() == 11 && date.day() == 20) // TDOR
                    || (date.month() == 3 && date.day() == 31) // TDOV
            }
            Season::Aro => {
                // The week following 14th February (Sunday-Saturday)
                let february_14 = Local.with_ymd_and_hms(date.year(), 2, 14, 0, 0, 0).unwrap();
                let weekday_offset = february_14.weekday().num_days_from_sunday();
                let start = 14 + 7 - weekday_offset;
                let end = start + 6;

                date.month() == 2 && date.day() >= start && date.day() <= end
            }
            Season::Ace => {
                // Last week of October, starting on Sunday
                let last_day_october = Local
                    .with_ymd_and_hms(date.year(), 10, 31, 0, 0, 0)
                    .unwrap();

                let weekday_offset_last_day_october =
                    last_day_october.weekday().num_days_from_sunday();
                let start = if weekday_offset_last_day_october == 6 {
                    31 - 7
                } else {
                    31 - weekday_offset_last_day_october - 7
                };
                let end = start + 6;

                date.month() == 10 && date.day() >= start && date.day() <= end
            }
            Season::Bi => date.month() == 9 && date.day() >= 16 && date.day() <= 23,
            Season::NonBinary => {
                // The week, starting Sunday/Monday, surrounding 14 July
                // We will just start on Sunday and end on Monday, so 8 days

                let july_14 = Local.with_ymd_and_hms(date.year(), 7, 14, 0, 0, 0).unwrap();
                let weekday_july_14_offset = july_14.weekday().num_days_from_sunday();
                let start = 14 - weekday_july_14_offset;
                let end = start + 7;

                date.month() == 7 && date.day() >= start && date.day() <= end
            }
            Season::Pride => date.month() == 6,
            Season::Disability => date.month() == 7,
            Season::BlackHistory => date.month() == 2 || date.month() == 10,
        }
    }

    pub fn for_date(date: &chrono::DateTime<Local>) -> Option<&'static Self> {
        Self::all().iter().find(|&season| season.is_season(date))
    }

    pub fn current() -> Option<&'static Self> {
        Self::for_date(&chrono::Local::now())
    }
}

fn css_class(season: &Season) -> String {
    format!("flag-{season}")
}

pub fn apply_seasonal_style(widget: &impl IsA<gtk::Widget>) {
    for season in Season::all() {
        widget.remove_css_class(&css_class(season));
    }

    if let Some(season) = Season::current() {
        log::debug!("Adding pride CSS class {}", &css_class(season));
        widget.add_css_class(&css_class(season));
    }
}

#[cfg(test)]
mod test {
    use super::Season;
    use chrono::prelude::*;

    #[test]
    fn intersex() {
        let date = Local.with_ymd_and_hms(2022, 10, 26, 0, 0, 0).unwrap();
        assert!(Season::Intersex.is_season(&date));

        let minute_before = Local.with_ymd_and_hms(2022, 10, 25, 23, 59, 0).unwrap();
        assert!(!Season::Intersex.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 10, 27, 0, 0, 0).unwrap();
        assert!(!Season::Intersex.is_season(&day_after));

        let date = Local.with_ymd_and_hms(2022, 11, 8, 0, 0, 0).unwrap();
        assert!(Season::Intersex.is_season(&date));

        let minute_before = Local.with_ymd_and_hms(2022, 11, 7, 23, 59, 0).unwrap();
        assert!(!Season::Intersex.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 11, 9, 0, 0, 0).unwrap();
        assert!(!Season::Intersex.is_season(&day_after));
    }

    #[test]
    fn lesbian() {
        let date = Local.with_ymd_and_hms(2022, 10, 8, 0, 0, 0).unwrap();
        assert!(Season::Lesbian.is_season(&date));

        let minute_before = Local.with_ymd_and_hms(2022, 10, 7, 23, 59, 0).unwrap();
        assert!(!Season::Lesbian.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 10, 9, 0, 0, 0).unwrap();
        assert!(!Season::Lesbian.is_season(&day_after));

        // visibility week
        let date = Local.with_ymd_and_hms(2022, 4, 26, 0, 0, 0).unwrap();
        assert!(Season::Lesbian.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2022, 5, 2, 0, 0, 0).unwrap();
        assert!(Season::Lesbian.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2022, 4, 25, 23, 59, 0).unwrap();
        assert!(!Season::Lesbian.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 5, 3, 0, 0, 0).unwrap();
        assert!(!Season::Lesbian.is_season(&day_after));
    }

    #[test]
    fn aids() {
        let date = Local.with_ymd_and_hms(2022, 12, 1, 0, 0, 0).unwrap();
        assert!(Season::Aids.is_season(&date));

        let minute_before = Local.with_ymd_and_hms(2022, 11, 30, 23, 59, 0).unwrap();
        assert!(!Season::Aids.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 12, 2, 0, 0, 0).unwrap();
        assert!(!Season::Aids.is_season(&day_after));
    }

    #[test]
    fn autism() {
        let date = Local.with_ymd_and_hms(2022, 6, 18, 0, 0, 0).unwrap();
        assert!(Season::Autism.is_season(&date));

        let minute_before = Local.with_ymd_and_hms(2022, 6, 17, 23, 59, 0).unwrap();
        assert!(!Season::Autism.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 6, 19, 0, 0, 0).unwrap();
        assert!(!Season::Autism.is_season(&day_after));
    }

    #[test]
    fn pan() {
        let date = Local.with_ymd_and_hms(2022, 5, 24, 0, 0, 0).unwrap();
        assert!(Season::Pan.is_season(&date));

        let minute_before = Local.with_ymd_and_hms(2022, 5, 23, 23, 59, 0).unwrap();
        assert!(!Season::Pan.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 5, 25, 0, 0, 0).unwrap();
        assert!(!Season::Pan.is_season(&day_after));
    }

    #[test]
    fn trans() {
        let date = Local.with_ymd_and_hms(2022, 11, 13, 0, 0, 0).unwrap();
        assert!(Season::Trans.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2022, 11, 19, 0, 0, 0).unwrap();
        assert!(Season::Trans.is_season(&date2));

        let date3 = Local.with_ymd_and_hms(2022, 11, 20, 0, 0, 0).unwrap();
        assert!(Season::Trans.is_season(&date3));

        let date4 = Local.with_ymd_and_hms(2022, 3, 31, 0, 0, 0).unwrap();
        assert!(Season::Trans.is_season(&date4));

        let minute_before = Local.with_ymd_and_hms(2022, 11, 12, 23, 59, 0).unwrap();
        assert!(!Season::Trans.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 11, 21, 0, 0, 0).unwrap();
        assert!(!Season::Trans.is_season(&day_after));
    }

    #[test]
    fn aro() {
        let date = Local.with_ymd_and_hms(2023, 2, 19, 0, 0, 0).unwrap();
        assert!(Season::Aro.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2023, 2, 25, 0, 0, 0).unwrap();
        assert!(Season::Aro.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2023, 2, 18, 23, 59, 0).unwrap();
        assert!(!Season::Aro.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2023, 2, 26, 0, 0, 0).unwrap();
        assert!(!Season::Aro.is_season(&day_after));
    }

    #[test]
    fn ace() {
        let date = Local.with_ymd_and_hms(2022, 10, 23, 0, 0, 0).unwrap();
        assert!(Season::Ace.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2022, 10, 29, 0, 0, 0).unwrap();
        assert!(Season::Ace.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2022, 10, 22, 23, 59, 0).unwrap();
        assert!(!Season::Ace.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 10, 30, 0, 0, 0).unwrap();
        assert!(!Season::Ace.is_season(&day_after));
    }

    #[test]
    fn bi() {
        let date = Local.with_ymd_and_hms(2022, 9, 16, 0, 0, 0).unwrap();
        assert!(Season::Bi.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2022, 9, 23, 0, 0, 0).unwrap();
        assert!(Season::Bi.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2022, 9, 15, 23, 59, 0).unwrap();
        assert!(!Season::Bi.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2022, 9, 24, 0, 0, 0).unwrap();
        assert!(!Season::Bi.is_season(&day_after));
    }

    #[test]
    fn non_binary() {
        let date = Local.with_ymd_and_hms(2023, 7, 9, 0, 0, 0).unwrap();
        assert!(Season::NonBinary.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2023, 7, 16, 0, 0, 0).unwrap();
        assert!(Season::NonBinary.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2023, 7, 8, 23, 59, 0).unwrap();
        assert!(!Season::NonBinary.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2023, 7, 17, 0, 0, 0).unwrap();
        assert!(!Season::NonBinary.is_season(&day_after));
    }

    #[test]
    fn pride() {
        let date = Local.with_ymd_and_hms(2023, 6, 1, 0, 0, 0).unwrap();
        assert!(Season::Pride.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2023, 6, 30, 0, 0, 0).unwrap();
        assert!(Season::Pride.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2023, 5, 31, 23, 59, 0).unwrap();
        assert!(!Season::Pride.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2023, 7, 1, 0, 0, 0).unwrap();
        assert!(!Season::Pride.is_season(&day_after));
    }

    #[test]
    fn disability_pride() {
        let date = Local.with_ymd_and_hms(2023, 7, 1, 0, 0, 0).unwrap();
        assert!(Season::Disability.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2023, 7, 31, 0, 0, 0).unwrap();
        assert!(Season::Disability.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2023, 6, 30, 23, 59, 0).unwrap();
        assert!(!Season::Disability.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2023, 8, 1, 0, 0, 0).unwrap();
        assert!(!Season::Disability.is_season(&day_after));
    }

    #[test]
    fn black_history_month_1() {
        let date = Local.with_ymd_and_hms(2023, 2, 1, 0, 0, 0).unwrap();
        assert!(Season::BlackHistory.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2023, 2, 28, 0, 0, 0).unwrap();
        assert!(Season::BlackHistory.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2023, 1, 31, 23, 59, 0).unwrap();
        assert!(!Season::BlackHistory.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2023, 3, 1, 0, 0, 0).unwrap();
        assert!(!Season::BlackHistory.is_season(&day_after));
    }

    #[test]
    fn black_history_month_2() {
        let date = Local.with_ymd_and_hms(2023, 10, 1, 0, 0, 0).unwrap();
        assert!(Season::BlackHistory.is_season(&date));

        let date2 = Local.with_ymd_and_hms(2023, 10, 31, 0, 0, 0).unwrap();
        assert!(Season::BlackHistory.is_season(&date2));

        let minute_before = Local.with_ymd_and_hms(2023, 9, 30, 23, 59, 0).unwrap();
        assert!(!Season::BlackHistory.is_season(&minute_before));

        let day_after = Local.with_ymd_and_hms(2023, 11, 1, 0, 0, 0).unwrap();
        assert!(!Season::BlackHistory.is_season(&day_after));
    }
}
