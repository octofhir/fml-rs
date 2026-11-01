//! The date and time transforms of the Mapping Language IG: `toDateTime`, `toDate`, `toTime`
//! (parse text with a format) and `unixToDateTime`, `unixToDate`, `unixToTime` (from a Unix
//! timestamp), and `escape`.
//!
//! The IG describes the timestamp as "ticks since epoch" without naming the unit; it is read as
//! a Unix timestamp, in seconds.
//!
//! Results are FHIR primitive values. They keep the precision of the input: `toDateTime('2024-05',
//! 'yyyy-MM')` is `2024-05`, and a time zone appears only when the input had one, so nothing is
//! invented.

/// The parts of a date and time that a format matched.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Parts {
    year: Option<i64>,
    month: Option<u32>,
    day: Option<u32>,
    hour: Option<u32>,
    minute: Option<u32>,
    second: Option<u32>,
    fraction: Option<String>,
    pm: Option<bool>,
    /// `Z`, or an offset such as `+02:00`.
    zone: Option<String>,
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Format codes are runs of these letters; anything else is a literal.
const CODE_LETTERS: &str = "yMdhHmsSaZz";

/// Splits a format into codes (runs of one code letter) and literal text.
fn tokens(format: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    for c in format.chars() {
        let is_code = CODE_LETTERS.contains(c);
        match out.last_mut() {
            Some((true, run)) if is_code && run.starts_with(c) => run.push(c),
            Some((false, literal)) if !is_code => literal.push(c),
            _ => out.push((is_code, c.to_string())),
        }
    }
    out
}

/// Parses `input` with a format of the IG's format codes.
fn parse(input: &str, format: &str) -> Result<Parts, String> {
    let mut parts = Parts::default();
    let mut rest = input;
    let fail = || format!("`{input}` does not match the format `{format}`");
    let digits = |rest: &mut &str, min: usize, max: usize| -> Option<u32> {
        let count = rest
            .chars()
            .take(max)
            .take_while(char::is_ascii_digit)
            .count();
        if count < min {
            return None;
        }
        let (number, tail) = rest.split_at(count);
        *rest = tail;
        number.parse().ok()
    };
    for (is_code, token) in tokens(format) {
        if !is_code {
            rest = rest.strip_prefix(token.as_str()).ok_or_else(fail)?;
            continue;
        }
        match token.as_str() {
            "yyyy" => parts.year = Some(i64::from(digits(&mut rest, 4, 4).ok_or_else(fail)?)),
            "yy" => {
                let year = digits(&mut rest, 2, 2).ok_or_else(fail)?;
                parts.year = Some(i64::from(if year < 50 { 2000 + year } else { 1900 + year }));
            }
            "MM" => parts.month = Some(digits(&mut rest, 2, 2).ok_or_else(fail)?),
            "M" => parts.month = Some(digits(&mut rest, 1, 2).ok_or_else(fail)?),
            "MMM" | "MMMM" => {
                let lower = rest.to_lowercase();
                let (index, length) = MONTHS
                    .iter()
                    .enumerate()
                    .find_map(|(i, name)| {
                        let name = if token == "MMM" { &name[..3] } else { name };
                        lower.starts_with(name).then_some((i, name.len()))
                    })
                    .ok_or_else(fail)?;
                parts.month = Some(index as u32 + 1);
                rest = &rest[length..];
            }
            "dd" => parts.day = Some(digits(&mut rest, 2, 2).ok_or_else(fail)?),
            "d" => parts.day = Some(digits(&mut rest, 1, 2).ok_or_else(fail)?),
            "HH" | "hh" => parts.hour = Some(digits(&mut rest, 2, 2).ok_or_else(fail)?),
            "H" | "h" => parts.hour = Some(digits(&mut rest, 1, 2).ok_or_else(fail)?),
            "mm" => parts.minute = Some(digits(&mut rest, 2, 2).ok_or_else(fail)?),
            "m" => parts.minute = Some(digits(&mut rest, 1, 2).ok_or_else(fail)?),
            "ss" => parts.second = Some(digits(&mut rest, 2, 2).ok_or_else(fail)?),
            "s" => parts.second = Some(digits(&mut rest, 1, 2).ok_or_else(fail)?),
            fraction if fraction.starts_with('S') => {
                let count = rest
                    .chars()
                    .take(fraction.len())
                    .take_while(char::is_ascii_digit)
                    .count();
                if count != fraction.len() {
                    return Err(fail());
                }
                parts.fraction = Some(rest[..count].to_owned());
                rest = &rest[count..];
            }
            "a" | "aa" => {
                let upper = rest.to_uppercase();
                let (pm, length) = if upper.starts_with("AM") {
                    (false, 2)
                } else if upper.starts_with("PM") {
                    (true, 2)
                } else if upper.starts_with('A') {
                    (false, 1)
                } else if upper.starts_with('P') {
                    (true, 1)
                } else {
                    return Err(fail());
                };
                parts.pm = Some(pm);
                rest = &rest[length..];
            }
            "Z" => {
                if let Some(tail) = rest.strip_prefix('Z') {
                    parts.zone = Some("Z".into());
                    rest = tail;
                } else {
                    let sign = rest.chars().next().filter(|c| *c == '+' || *c == '-');
                    let sign = sign.ok_or_else(fail)?;
                    rest = &rest[1..];
                    let hours = digits(&mut rest, 2, 2).ok_or_else(fail)?;
                    if let Some(tail) = rest.strip_prefix(':') {
                        rest = tail;
                    }
                    let minutes = digits(&mut rest, 2, 2).ok_or_else(fail)?;
                    parts.zone = Some(format!("{sign}{hours:02}:{minutes:02}"));
                }
            }
            "z" => {
                return Err(format!(
                    "time zone names (`z`) are not supported; use an offset (`Z`) in `{format}`"
                ));
            }
            other => return Err(format!("unknown format code `{other}` in `{format}`")),
        }
    }
    if !rest.is_empty() {
        return Err(fail());
    }
    if let (Some(pm), Some(hour)) = (parts.pm, parts.hour) {
        if !(1..=12).contains(&hour) {
            return Err(fail());
        }
        parts.hour = Some(match (pm, hour) {
            (false, 12) => 0,
            (true, 12) => 12,
            (true, hour) => hour + 12,
            (false, hour) => hour,
        });
    }
    validate(&parts).map_err(|_| fail())?;
    Ok(parts)
}

fn validate(parts: &Parts) -> Result<(), ()> {
    let check = |value: Option<u32>, range: std::ops::RangeInclusive<u32>| match value {
        Some(value) if !range.contains(&value) => Err(()),
        _ => Ok(()),
    };
    check(parts.month, 1..=12)?;
    check(parts.hour, 0..=23)?;
    check(parts.minute, 0..=59)?;
    check(parts.second, 0..=59)?;
    if let (Some(year), Some(month), Some(day)) = (parts.year, parts.month, parts.day)
        && (day < 1 || day > days_in_month(year, month))
    {
        return Err(());
    }
    Ok(())
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

fn date_text(parts: &Parts) -> Option<String> {
    let mut text = format!("{:04}", parts.year?);
    if let Some(month) = parts.month {
        text.push_str(&format!("-{month:02}"));
        if let Some(day) = parts.day {
            text.push_str(&format!("-{day:02}"));
        }
    }
    Some(text)
}

fn time_text(parts: &Parts) -> Option<String> {
    let mut text = format!(
        "{:02}:{:02}:{:02}",
        parts.hour?,
        parts.minute.unwrap_or(0),
        parts.second.unwrap_or(0)
    );
    if let Some(fraction) = &parts.fraction {
        text.push('.');
        text.push_str(fraction);
    }
    Some(text)
}

/// `toDateTime(source [, format])`: a FHIR dateTime.
pub fn to_date_time(source: &str, format: Option<&str>) -> Result<String, String> {
    let Some(format) = format else {
        return fhir_value(source, is_fhir_date_time, "dateTime");
    };
    let parts = parse(source, format)?;
    let mut text = date_text(&parts)
        .ok_or_else(|| format!("the format `{format}` has no year, which a dateTime needs"))?;
    if let Some(time) = time_text(&parts) {
        if parts.day.is_none() {
            return Err(format!("the format `{format}` has a time but no day"));
        }
        text.push('T');
        text.push_str(&time);
        if let Some(zone) = &parts.zone {
            text.push_str(zone);
        }
    }
    Ok(text)
}

/// `toDate(source [, format])`: a FHIR date.
pub fn to_date(source: &str, format: Option<&str>) -> Result<String, String> {
    let Some(format) = format else {
        return fhir_value(source, is_fhir_date, "date");
    };
    date_text(&parse(source, format)?)
        .ok_or_else(|| format!("the format `{format}` has no year, which a date needs"))
}

/// `toTime(source [, format])`: a FHIR time.
pub fn to_time(source: &str, format: Option<&str>) -> Result<String, String> {
    let Some(format) = format else {
        return fhir_value(source, is_fhir_time, "time");
    };
    time_text(&parse(source, format)?)
        .ok_or_else(|| format!("the format `{format}` has no hour, which a time needs"))
}

fn fhir_value(source: &str, valid: fn(&str) -> bool, type_name: &str) -> Result<String, String> {
    if valid(source) {
        Ok(source.to_owned())
    } else {
        Err(format!("`{source}` is not a FHIR {type_name}"))
    }
}

fn is_fhir_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let number = |p: &str, n: usize| p.len() == n && p.chars().all(|c| c.is_ascii_digit());
    match parts.as_slice() {
        [y] => number(y, 4),
        [y, m] => number(y, 4) && number(m, 2),
        [y, m, d] => number(y, 4) && number(m, 2) && number(d, 2),
        _ => false,
    }
}

fn is_fhir_time(text: &str) -> bool {
    parse(text, "HH:mm:ss").is_ok()
        || text.split_once('.').is_some_and(|(time, fraction)| {
            parse(time, "HH:mm:ss").is_ok()
                && !fraction.is_empty()
                && fraction.chars().all(|c| c.is_ascii_digit())
        })
}

fn is_fhir_date_time(text: &str) -> bool {
    let Some((date, rest)) = text.split_once('T') else {
        return is_fhir_date(text);
    };
    if date.len() != 10 || !is_fhir_date(date) {
        return false;
    }
    let (time, zone) = if let Some(time) = rest.strip_suffix('Z') {
        (time, "Z")
    } else if rest.len() > 6 && matches!(&rest[rest.len() - 6..rest.len() - 5], "+" | "-") {
        rest.split_at(rest.len() - 6)
    } else {
        (rest, "")
    };
    let zone_ok = zone.is_empty() || zone == "Z" || parse(zone, "Z").is_ok();
    is_fhir_time(time) && zone_ok
}

/// The civil date of a day number counted from 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The date, time and zone of a Unix timestamp in seconds, in `timezone` (`+10:00`; UTC when
/// absent, written `Z`).
fn from_unix(seconds: i64, timezone: Option<&str>) -> Result<(String, String, String), String> {
    let (offset, zone) = match timezone {
        None | Some("Z") => (0, "Z".to_owned()),
        Some(text) => {
            let parts = parse(text, "Z")
                .map_err(|_| format!("`{text}` is not a time zone offset such as +10:00"))?;
            let zone = parts.zone.unwrap_or_default();
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let hours: i64 = zone[1..3].parse().unwrap_or(0);
            let minutes: i64 = zone[4..6].parse().unwrap_or(0);
            (sign * (hours * 3600 + minutes * 60), zone)
        }
    };
    let local = seconds + offset;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let in_day = local.rem_euclid(86_400);
    let time = format!(
        "{:02}:{:02}:{:02}",
        in_day / 3600,
        (in_day % 3600) / 60,
        in_day % 60
    );
    Ok((format!("{year:04}-{month:02}-{day:02}"), time, zone))
}

/// `unixToDateTime(seconds [, timezone])`.
pub fn unix_to_date_time(seconds: i64, timezone: Option<&str>) -> Result<String, String> {
    let (date, time, zone) = from_unix(seconds, timezone)?;
    Ok(format!("{date}T{time}{zone}"))
}

/// `unixToDate(seconds [, timezone])`.
pub fn unix_to_date(seconds: i64, timezone: Option<&str>) -> Result<String, String> {
    Ok(from_unix(seconds, timezone)?.0)
}

/// `unixToTime(seconds [, timezone])`.
pub fn unix_to_time(seconds: i64, timezone: Option<&str>) -> Result<String, String> {
    Ok(from_unix(seconds, timezone)?.1)
}

/// `escape(source, from, to)`: re-escapes text between `html`, `xml`, `json` and `none`.
pub fn escape(source: &str, from: &str, to: &str) -> Result<String, String> {
    let plain = match from {
        "html" | "xml" => unescape_markup(source),
        "json" => serde_json::from_str::<String>(&format!("\"{source}\""))
            .map_err(|e| format!("`{source}` is not JSON-escaped text: {e}"))?,
        "none" | "" => source.to_owned(),
        other => return Err(format!("unknown escape format `{other}`")),
    };
    Ok(match to {
        "html" | "xml" => plain
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;"),
        "json" => {
            let quoted = serde_json::to_string(&plain).expect("a string serializes");
            quoted[1..quoted.len() - 1].to_owned()
        }
        "none" | "" => plain,
        other => return Err(format!("unknown escape format `{other}`")),
    })
}

fn unescape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            numeric if numeric.starts_with("#x") || numeric.starts_with("#X") => {
                u32::from_str_radix(&numeric[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            numeric if numeric.starts_with('#') => {
                numeric[1..].parse().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_formats() {
        assert_eq!(
            to_date_time("2024-05-01", Some("yyyy-MM-dd")).unwrap(),
            "2024-05-01"
        );
        assert_eq!(
            to_date_time("20240501134502", Some("yyyyMMddHHmmss")).unwrap(),
            "2024-05-01T13:45:02"
        );
        assert_eq!(
            to_date_time("01/05/2024 1:45 PM +0200", Some("dd/MM/yyyy h:mm a Z")).unwrap(),
            "2024-05-01T13:45:00+02:00"
        );
        assert_eq!(
            to_date_time("2024-05-01T12:00:00.123Z", Some("yyyy-MM-ddTHH:mm:ss.SSSZ")).unwrap(),
            "2024-05-01T12:00:00.123Z"
        );
        assert_eq!(
            to_date("3 June 80", Some("d MMMM yy")).unwrap(),
            "1980-06-03"
        );
        assert_eq!(to_date("2024-05", Some("yyyy-MM")).unwrap(), "2024-05");
        assert_eq!(to_time("1345", Some("HHmm")).unwrap(), "13:45:00");
        assert_eq!(to_time("12:05 AM", Some("hh:mm a")).unwrap(), "00:05:00");
    }

    #[test]
    fn rejects_input_that_does_not_match() {
        assert!(to_date("2024-13-01", Some("yyyy-MM-dd")).is_err());
        assert!(to_date("2023-02-29", Some("yyyy-MM-dd")).is_err());
        assert!(to_date("2024-05-01x", Some("yyyy-MM-dd")).is_err());
        assert!(to_date_time("2024", Some("yy")).is_err());
        assert!(to_date_time("12:00 PST", Some("HH:mm z")).is_err());
    }

    #[test]
    fn default_formats_are_the_fhir_ones() {
        assert_eq!(
            to_date_time("2024-05-01T10:00:00+02:00", None).unwrap(),
            "2024-05-01T10:00:00+02:00"
        );
        assert_eq!(to_date_time("2024", None).unwrap(), "2024");
        assert!(to_date_time("2024-05-01 10:00", None).is_err());
        assert_eq!(to_date("2024-05-01", None).unwrap(), "2024-05-01");
        assert_eq!(to_time("10:00:00.5", None).unwrap(), "10:00:00.5");
        assert!(to_time("25:00:00", None).is_err());
    }

    #[test]
    fn converts_unix_timestamps() {
        assert_eq!(unix_to_date_time(0, None).unwrap(), "1970-01-01T00:00:00Z");
        assert_eq!(
            unix_to_date_time(1_714_570_200, Some("+10:00")).unwrap(),
            "2024-05-01T23:30:00+10:00"
        );
        assert_eq!(unix_to_date(951_782_400, None).unwrap(), "2000-02-29");
        assert_eq!(unix_to_time(-1, None).unwrap(), "23:59:59");
        assert_eq!(unix_to_date(-86_400, Some("-05:00")).unwrap(), "1969-12-30");
    }

    #[test]
    fn escapes_between_formats() {
        assert_eq!(
            escape("a < b & \"c\"", "none", "html").unwrap(),
            "a &lt; b &amp; &quot;c&quot;"
        );
        assert_eq!(
            escape("&lt;b&gt; &#65;&#x42;", "html", "none").unwrap(),
            "<b> AB"
        );
        assert_eq!(
            escape("line\\nnext \\\"q\\\"", "json", "xml").unwrap(),
            "line\nnext &quot;q&quot;"
        );
        assert_eq!(escape("tab\there", "none", "json").unwrap(), "tab\\there");
        assert!(escape("x", "rtf", "html").is_err());
    }
}
