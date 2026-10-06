// Civil date arithmetic, written on its own so the ground truth does not
// depend on the implementation it exists to test.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use fileroom::conventions::{Date, Instant};
use fileroom::schedule::{Cutoff, Period, Unit};

pub fn days_from_civil(d: Date) -> i64 {
    let (y, m, day) = (i64::from(d.year), i64::from(d.month), i64::from(d.day));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> Date {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Date::new(y as u16, m as u8, d as u8).expect("civil")
}

pub fn add_days(d: Date, n: i64) -> Date {
    civil_from_days(days_from_civil(d) + n)
}

pub fn add_months(d: Date, n: i64) -> Date {
    let total = i64::from(d.year) * 12 + i64::from(d.month) - 1 + n;
    let year = total.div_euclid(12) as u16;
    let month = (total.rem_euclid(12) + 1) as u8;
    let last = Date::new(year, month, 1).unwrap().days_in_month();
    Date::new(year, month, d.day.min(last)).unwrap()
}

pub fn add_period(d: Date, p: Period) -> Date {
    match p.unit {
        Unit::Years => add_months(d, 12 * i64::from(p.count)),
        Unit::Months => add_months(d, i64::from(p.count)),
        Unit::Days => add_days(d, i64::from(p.count)),
    }
}

pub fn end_of_month(d: Date) -> Date {
    Date::new(d.year, d.month, d.days_in_month()).unwrap()
}

pub fn cutoff(d: Date, c: Cutoff, fiscal_year_start_month: u8) -> Date {
    match c {
        Cutoff::None => d,
        Cutoff::Month => end_of_month(d),
        Cutoff::Quarter => {
            let last_month = ((d.month - 1) / 3) * 3 + 3;
            end_of_month(Date::new(d.year, last_month, 1).unwrap())
        }
        Cutoff::CalendarYear => Date::new(d.year, 12, 31).unwrap(),
        Cutoff::FiscalYear => {
            if fiscal_year_start_month == 1 {
                return Date::new(d.year, 12, 31).unwrap();
            }
            let start_this_year = Date::new(d.year, fiscal_year_start_month, 1).unwrap();
            let next_start = if d >= start_this_year {
                Date::new(d.year + 1, fiscal_year_start_month, 1).unwrap()
            } else {
                start_this_year
            };
            add_days(next_start, -1)
        }
    }
}

pub fn instant(d: Date, hour: u8, minute: u8, second: u8) -> Instant {
    Instant {
        date: d,
        hour,
        minute,
        second,
        nanosecond: 0,
    }
}

pub fn unix_ms(i: Instant) -> u64 {
    let days = days_from_civil(i.date);
    ((days * 86_400 + i64::from(i.hour) * 3600 + i64::from(i.minute) * 60 + i64::from(i.second))
        * 1000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_clamps() {
        let d = Date::new(2024, 2, 29).unwrap();
        assert_eq!(civil_from_days(days_from_civil(d)), d);
        assert_eq!(add_months(d, 12), Date::new(2025, 2, 28).unwrap());
        assert_eq!(
            add_months(Date::new(2024, 1, 31).unwrap(), 1),
            Date::new(2024, 2, 29).unwrap()
        );
        assert_eq!(
            add_days(Date::new(2023, 12, 31).unwrap(), 1),
            Date::new(2024, 1, 1).unwrap()
        );
        assert_eq!(
            cutoff(Date::new(2024, 5, 3).unwrap(), Cutoff::FiscalYear, 10),
            Date::new(2024, 9, 30).unwrap()
        );
        assert_eq!(
            cutoff(Date::new(2024, 11, 3).unwrap(), Cutoff::FiscalYear, 10),
            Date::new(2025, 9, 30).unwrap()
        );
        assert_eq!(
            cutoff(Date::new(2024, 5, 3).unwrap(), Cutoff::Quarter, 10),
            Date::new(2024, 6, 30).unwrap()
        );
        assert_eq!(
            cutoff(Date::new(2024, 5, 3).unwrap(), Cutoff::FiscalYear, 1),
            Date::new(2024, 12, 31).unwrap()
        );
    }
}
