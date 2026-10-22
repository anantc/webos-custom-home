use serde::{Deserialize, Serialize};

pub const COLLECTIONS: [&str; 10] = [
    "spring",
    "summer",
    "monsoon",
    "autumn",
    "winter",
    "diwali",
    "halloween",
    "christmas",
    "thanksgiving",
    "holi",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date(pub i32, pub u32, pub u32);
impl Date {
    pub fn parse(s: &str) -> Option<Self> {
        let fields: Vec<_> = s.split('-').collect();
        if fields.len() != 3 {
            return None;
        }
        let date = Self(
            fields[0].parse().ok()?,
            fields[1].parse().ok()?,
            fields[2].parse().ok()?,
        );
        let leap = date.0 % 4 == 0 && (date.0 % 100 != 0 || date.0 % 400 == 0);
        let days = match date.1 {
            2 => {
                if leap {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return None,
        };
        (date.0 >= 2020 && date.0 <= 2200 && date.2 >= 1 && date.2 <= days).then_some(date)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Holiday {
    pub collection: String,
    pub start: String,
    pub end: String,
    pub source: String,
}

pub fn collection(date: Date, holidays: &[Holiday]) -> &str {
    // Year-specific windows take precedence when a lunar festival overlaps Halloween.
    for h in holidays {
        if let (Some(start), Some(end)) = (Date::parse(&h.start), Date::parse(&h.end)) {
            if date >= start && date <= end {
                return &h.collection;
            }
        }
    }
    let Date(year, month, day) = date;
    if (month == 10 && day >= 27) || (month == 11 && day == 1) {
        return "halloween";
    }
    if month == 12 && (19..=27).contains(&day) {
        return "christmas";
    }
    // Gregorian weekday of November 1 (Sunday=0); Thanksgiving is its fourth Thursday.
    let y = year - 1;
    let jan1 = (year + y / 4 - y / 100 + y / 400) % 7;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let nov1 = (jan1 + 304 + i32::from(leap)) % 7;
    let thanksgiving = 22 + (4 - nov1 + 7) % 7;
    if month == 11 && day as i32 >= thanksgiving - 2 && day as i32 <= thanksgiving + 2 {
        return "thanksgiving";
    }
    match month {
        2..=3 => "spring",
        4..=5 => "summer",
        6..=9 => "monsoon",
        10..=11 => "autumn",
        _ => "winter",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_and_lunar_priority() {
        let holidays = vec![Holiday {
            collection: "diwali".into(),
            start: "2027-10-27".into(),
            end: "2027-10-30".into(),
            source: String::new(),
        }];
        assert_eq!(collection(Date(2027, 10, 28), &holidays), "diwali");
        assert_eq!(collection(Date(2028, 10, 28), &holidays), "halloween");
        assert_eq!(collection(Date(2027, 10, 26), &holidays), "autumn");
        assert_eq!(collection(Date(2026, 11, 1), &[]), "halloween");
        assert_eq!(collection(Date(2026, 11, 2), &[]), "autumn");
        assert_eq!(collection(Date(2026, 12, 19), &[]), "christmas");
        assert_eq!(collection(Date(2027, 1, 1), &[]), "winter");
        for (month, expected) in [
            (2, "spring"),
            (4, "summer"),
            (6, "monsoon"),
            (9, "monsoon"),
            (10, "autumn"),
        ] {
            assert_eq!(collection(Date(2026, month, 15), &[]), expected);
        }
    }
    #[test]
    fn thanksgiving_moves_each_year() {
        for (year, day) in [(2026, 26), (2027, 25), (2028, 23), (2029, 22), (2030, 28)] {
            assert_eq!(collection(Date(year, 11, day), &[]), "thanksgiving");
            assert_eq!(collection(Date(year, 11, day - 3), &[]), "autumn");
            assert_eq!(collection(Date(year, 11, day + 3), &[]), "autumn");
        }
    }
    #[test]
    fn rejects_invalid_calendar_dates() {
        assert!(Date::parse("2026-02-29").is_none());
        assert!(Date::parse("2028-02-29").is_some());
        assert!(Date::parse("2026-13-01").is_none());
    }
}
