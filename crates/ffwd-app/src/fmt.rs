//! Small formatting helpers (numbers, dates) with no dependencies.

pub fn num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

pub fn date(t: i64, tz: i64) -> String {
    let (y, m, d) = civil((t + tz).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn date_time(t: i64, tz: i64) -> String {
    let local = t + tz;
    let (y, m, d) = civil(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

pub fn relative(t: i64, now: i64, tz: i64) -> String {
    let d = now - t;
    match d {
        i64::MIN..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=604_799 => {
            let n = d / 86_400;
            if n == 1 { "yesterday".into() } else { format!("{n} days ago") }
        }
        _ => date(t, tz),
    }
}

pub fn initials(name: &str) -> String {
    let mut it = name.split_whitespace().filter_map(|w| w.chars().next());
    let a = it.next().unwrap_or('?');
    let b = it.next_back();
    let mut s: String = a.to_uppercase().collect();
    if let Some(b) = b {
        s.extend(b.to_uppercase());
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_and_numbers() {
        assert_eq!(super::date(0, 0), "1970-01-01");
        assert_eq!(super::date(1_763_000_000, 9 * 3600), "2025-11-13");
        assert_eq!(super::num(1234567), "1,234,567");
        assert_eq!(super::num(12), "12");
        assert_eq!(super::initials("dominic renner"), "DR");
    }
}
