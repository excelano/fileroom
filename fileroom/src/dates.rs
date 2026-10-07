//! Civil date arithmetic (SPEC §8.2): cutoffs and periods, with the
//! month-end rule. Eligibility never reads a clock; the two functions that
//! do are here for the timestamps a run writes.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use crate::conventions::{Date, Instant};
use crate::schedule::{Cutoff, Period, Unit};

fn date(year: u16, month: u8, day: u8) -> Date {
    Date { year, month, day }
}

/// Days since 1970-01-01, which may be negative.
#[must_use]
pub fn days_since_epoch(d: Date) -> i64 {
    let (y, m, day) = (i64::from(d.year), i64::from(d.month), i64::from(d.day));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date a number of days since 1970-01-01 names.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn from_days_since_epoch(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    date(year as u16, month as u8, day as u8)
}

/// The date `n` days later, or earlier for a negative `n`.
#[must_use]
pub fn add_days(d: Date, n: i64) -> Date {
    from_days_since_epoch(days_since_epoch(d) + n)
}

/// The same day of the month `n` months later, or the last day of that
/// month where the day does not exist (SPEC §8.2).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn add_months(d: Date, n: i64) -> Date {
    let months = i64::from(d.year) * 12 + i64::from(d.month) - 1 + n;
    let year = months.div_euclid(12) as u16;
    let month = (months.rem_euclid(12) + 1) as u8;
    date(year, month, d.day.min(date(year, month, 1).days_in_month()))
}

/// The date a period later (SPEC §8.2).
#[must_use]
pub fn add_period(d: Date, p: Period) -> Date {
    match p.unit {
        Unit::Years => add_months(d, 12 * i64::from(p.count)),
        Unit::Months => add_months(d, i64::from(p.count)),
        Unit::Days => add_days(d, i64::from(p.count)),
    }
}

/// The last day of the date's month.
#[must_use]
pub fn end_of_month(d: Date) -> Date {
    date(d.year, d.month, d.days_in_month())
}

/// The last day of the cutoff period containing the date (SPEC §8.2).
///
/// `fiscal_year_start_month` is 1 to 12; a fiscal year beginning in January
/// is the calendar year.
#[must_use]
pub fn cutoff(d: Date, c: Cutoff, fiscal_year_start_month: u8) -> Date {
    match c {
        Cutoff::None => d,
        Cutoff::Month => end_of_month(d),
        Cutoff::Quarter => end_of_month(date(d.year, ((d.month - 1) / 3) * 3 + 3, 1)),
        Cutoff::CalendarYear => date(d.year, 12, 31),
        Cutoff::FiscalYear => {
            let start = fiscal_year_start_month.clamp(1, 12);
            if start == 1 {
                return date(d.year, 12, 31);
            }
            let this_year = date(d.year, start, 1);
            let next_start = if d >= this_year {
                date(d.year + 1, start, 1)
            } else {
                this_year
            };
            add_days(next_start, -1)
        }
    }
}

/// The current instant in UTC from the system clock.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn utc_now() -> Instant {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    Instant {
        date: from_days_since_epoch(days),
        hour: (rest / 3600) as u8,
        minute: ((rest % 3600) / 60) as u8,
        second: (rest % 60) as u8,
        nanosecond: 0,
    }
}

/// Today's UTC date from the system clock.
#[must_use]
pub fn utc_today() -> Date {
    utc_now().date
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: u16, m: u8, d: u8) -> Date {
        Date::new(y, m, d).unwrap()
    }

    fn every_day(from: u16, to: u16) -> impl Iterator<Item = Date> {
        (days_since_epoch(date(from, 1, 1))..=days_since_epoch(date(to, 12, 31)))
            .map(from_days_since_epoch)
    }

    #[test]
    fn epoch_days_round_trip_and_step_by_one() {
        let mut previous = None;
        for d in every_day(1896, 2104) {
            assert_eq!(from_days_since_epoch(days_since_epoch(d)), d);
            if let Some(p) = previous {
                assert_eq!(days_since_epoch(d), days_since_epoch(p) + 1);
                assert!(d > p);
            }
            previous = Some(d);
        }
        assert_eq!(days_since_epoch(date(1970, 1, 1)), 0);
        assert_eq!(days_since_epoch(date(2000, 3, 1)), 11017);
    }

    #[test]
    fn months_clamp_to_the_month_end_and_never_overshoot() {
        assert_eq!(add_months(date(2024, 2, 29), 12), date(2025, 2, 28));
        assert_eq!(add_months(date(2024, 1, 31), 1), date(2024, 2, 29));
        assert_eq!(add_months(date(2023, 1, 31), 1), date(2023, 2, 28));
        assert_eq!(add_months(date(2024, 3, 31), -1), date(2024, 2, 29));
        assert_eq!(add_months(date(2024, 12, 15), 1), date(2025, 1, 15));
        for d in every_day(2019, 2025) {
            for n in [1, 2, 3, 6, 11, 12, 13, 24, 36, 120] {
                let later = add_months(d, n);
                let months = i64::from(later.year) * 12 + i64::from(later.month)
                    - (i64::from(d.year) * 12 + i64::from(d.month));
                assert_eq!(months, n, "{d} + {n} months = {later}");
                assert!(later.day <= d.day);
                assert!(later.day == d.day || later.day == later.days_in_month());
                let back = add_months(later, -n);
                assert!(back <= d && back.month == d.month && back.year == d.year);
            }
        }
    }

    #[test]
    fn periods_add_as_their_unit_says() {
        let p = |count, unit| Period { count, unit };
        assert_eq!(
            add_period(date(2024, 1, 17), p(6, Unit::Years)),
            date(2030, 1, 17)
        );
        assert_eq!(
            add_period(date(2024, 1, 31), p(1, Unit::Months)),
            date(2024, 2, 29)
        );
        assert_eq!(
            add_period(date(2024, 12, 31), p(90, Unit::Days)),
            date(2025, 3, 31)
        );
        assert_eq!(
            add_period(date(2024, 1, 17), p(0, Unit::Years)),
            date(2024, 1, 17)
        );
    }

    #[test]
    fn cutoffs_end_the_period_containing_the_date() {
        for d in every_day(2023, 2025) {
            assert_eq!(cutoff(d, Cutoff::None, 10), d);
            let month = cutoff(d, Cutoff::Month, 10);
            assert!(month >= d && month.month == d.month && month.day == d.days_in_month());
            let quarter = cutoff(d, Cutoff::Quarter, 10);
            assert!(
                quarter >= d
                    && quarter.month.is_multiple_of(3)
                    && quarter.day == quarter.days_in_month()
            );
            assert!(days_since_epoch(quarter) - days_since_epoch(d) < 92);
            let year = cutoff(d, Cutoff::CalendarYear, 10);
            assert_eq!(year, date(d.year, 12, 31));
            for start in 1..=12u8 {
                let fy = cutoff(d, Cutoff::FiscalYear, start);
                assert!(fy >= d, "{d} fy{start} {fy}");
                assert!(days_since_epoch(fy) - days_since_epoch(d) < 366);
                let next = add_days(fy, 1);
                assert_eq!(
                    (next.month, next.day),
                    (start, 1),
                    "{d} fy{start} ends {fy}"
                );
                assert_eq!(
                    cutoff(fy, Cutoff::FiscalYear, start),
                    fy,
                    "a cutoff date is its own cutoff"
                );
            }
            assert_eq!(cutoff(d, Cutoff::FiscalYear, 1), year);
        }
        assert_eq!(
            cutoff(date(2024, 5, 3), Cutoff::FiscalYear, 10),
            date(2024, 9, 30)
        );
        assert_eq!(
            cutoff(date(2024, 11, 3), Cutoff::FiscalYear, 10),
            date(2025, 9, 30)
        );
        assert_eq!(
            cutoff(date(2024, 2, 29), Cutoff::FiscalYear, 3),
            date(2024, 2, 29)
        );
    }
}
