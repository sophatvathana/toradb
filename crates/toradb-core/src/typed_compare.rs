//! Type-aware comparison of metadata values stored as strings.
use std::cmp::Ordering;

use crate::schema::ColumnType;

pub fn typed_cmp(ty: ColumnType, a: &str, b: &str) -> Option<Ordering> {
    match ty {
        ColumnType::Int => {
            let x: i64 = a.trim().parse().ok()?;
            let y: i64 = b.trim().parse().ok()?;
            Some(x.cmp(&y))
        }
        ColumnType::Float => {
            let x: f64 = a.trim().parse().ok()?;
            let y: f64 = b.trim().parse().ok()?;
            x.partial_cmp(&y)
        }
        ColumnType::Bool => {
            let x = parse_bool(a)?;
            let y = parse_bool(b)?;
            Some(x.cmp(&y))
        }
        ColumnType::Date => {
            let x = parse_date_days(a)?;
            let y = parse_date_days(b)?;
            Some(x.cmp(&y))
        }
        ColumnType::Timestamp => match (parse_timestamp_millis(a), parse_timestamp_millis(b)) {
            (Some(x), Some(y)) => Some(x.cmp(&y)),
            _ => Some(a.cmp(b)),
        },
        _ => Some(a.cmp(b)),
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" | "y" => Some(true),
        "false" | "f" | "0" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn parse_date_days(s: &str) -> Option<i64> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let sep = bytes[4];
    if (sep != b'-' && sep != b'/') || bytes[7] != sep {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

pub fn parse_timestamp_millis(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let days = parse_date_days(s)?;
    let mut millis = days * 86_400_000;
    if s.len() > 10 {
        let sep = s.as_bytes()[10];
        if sep != b'T' && sep != b' ' {
            return None;
        }
        let time = &s[11..];
        let mut parts = time.split(':');
        let hh: i64 = parts.next()?.parse().ok()?;
        let mm: i64 = parts.next()?.parse().ok()?;
        let mut ss = 0i64;
        let mut frac_ms = 0i64;
        if let Some(sec) = parts.next() {
            let mut sec_parts = sec.split('.');
            ss = sec_parts.next()?.parse().ok()?;
            if let Some(frac) = sec_parts.next() {
                let mut f = String::from(frac);
                f.truncate(3);
                while f.len() < 3 {
                    f.push('0');
                }
                frac_ms = f.parse().ok()?;
            }
        }
        if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) || !(0..=60).contains(&ss) {
            return None;
        }
        millis += hh * 3_600_000 + mm * 60_000 + ss * 1_000 + frac_ms;
    }
    Some(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_orders_numerically_not_lexically() {
        assert_eq!(typed_cmp(ColumnType::Int, "9", "10"), Some(Ordering::Less));
        assert_eq!(
            typed_cmp(ColumnType::Text, "9", "10"),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn float_orders_numerically() {
        assert_eq!(
            typed_cmp(ColumnType::Float, "2.5", "10.1"),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn date_orders_chronologically() {
        assert_eq!(
            typed_cmp(ColumnType::Date, "2024-01-02", "2024-01-01"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            typed_cmp(ColumnType::Date, "2023-12-31", "2024-01-01"),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn timestamp_orders_by_time() {
        assert_eq!(
            typed_cmp(
                ColumnType::Timestamp,
                "2024-01-01T10:00:00",
                "2024-01-01T09:59:59"
            ),
            Some(Ordering::Greater)
        );
        assert_eq!(
            typed_cmp(
                ColumnType::Timestamp,
                "2024-01-01 00:00:00Z",
                "2024-01-02T00:00:00Z"
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn bool_orders_false_before_true() {
        assert_eq!(
            typed_cmp(ColumnType::Bool, "false", "true"),
            Some(Ordering::Less)
        );
        assert_eq!(
            typed_cmp(ColumnType::Bool, "1", "0"),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn unparseable_typed_value_returns_none() {
        assert_eq!(typed_cmp(ColumnType::Int, "abc", "10"), None);
        assert_eq!(
            typed_cmp(ColumnType::Date, "not-a-date", "2024-01-01"),
            None
        );
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(ColumnType::parse("INTEGER"), ColumnType::Int);
        assert_eq!(ColumnType::parse("bigint"), ColumnType::Int);
        assert_eq!(ColumnType::parse("DOUBLE"), ColumnType::Float);
        assert_eq!(ColumnType::parse("BOOLEAN"), ColumnType::Bool);
        assert_eq!(ColumnType::parse("DATETIME"), ColumnType::Timestamp);
        assert_eq!(ColumnType::parse("VARCHAR(255)"), ColumnType::Text);
        assert_eq!(ColumnType::parse("DECIMAL(10,2)"), ColumnType::Float);
        assert_eq!(ColumnType::parse("wibble"), ColumnType::Text);
    }
}
