//! Dates and times in the lexical form a logical model declares with
//! `elementdefinition-date-format` (`YYYYMMDDHHMMSS.UUUU[+|-ZZzz]`), and the FHIR form maps work
//! with (`YYYY-MM-DDTHH:MM:SS.UUUU+ZZ:zz`).
//!
//! The conversion keeps the precision as written: `202001` becomes `2020-01`, `202001011230`
//! becomes `2020-01-01T12:30`, and a time zone stays where it is. Going back gives the same text,
//! so reading and writing a document changes nothing. FHIR's own `dateTime` needs seconds and a
//! zone once there is a time; turning the value into a valid FHIR `dateTime` is the map's choice
//! (the precision the document had is information the map may want).

/// The FHIR-style form of a compact timestamp, or `None` when `text` is not one.
pub fn from_compact(text: &str) -> Option<String> {
    let (main, zone) = split_zone(text)?;
    let (digits, fraction) = match main.split_once('.') {
        Some((digits, fraction)) => (digits, Some(fraction)),
        None => (main, None),
    };
    if !digits.bytes().all(|b| b.is_ascii_digit())
        || !matches!(digits.len(), 4 | 6 | 8 | 10 | 12 | 14)
    {
        return None;
    }
    if fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
        || (fraction.is_some() && digits.len() != 14)
    {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&digits[0..4]);
    if digits.len() >= 6 {
        out.push('-');
        out.push_str(&digits[4..6]);
    }
    if digits.len() >= 8 {
        out.push('-');
        out.push_str(&digits[6..8]);
    }
    if digits.len() >= 10 {
        out.push('T');
        out.push_str(&digits[8..10]);
    }
    if digits.len() >= 12 {
        out.push(':');
        out.push_str(&digits[10..12]);
    }
    if digits.len() >= 14 {
        out.push(':');
        out.push_str(&digits[12..14]);
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    if let Some((sign, hours, minutes)) = zone {
        out.push(sign);
        out.push_str(hours);
        out.push(':');
        out.push_str(minutes);
    }
    Some(out)
}

/// The compact form of a FHIR-style date or time (the inverse of [`from_compact`]); `Z` is
/// written as `+0000`. `None` when `text` is not in that form.
pub fn to_compact(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    // Date and time digits, each group with the separator before it.
    let groups: [(Option<u8>, usize); 6] = [
        (None, 4),
        (Some(b'-'), 2),
        (Some(b'-'), 2),
        (Some(b'T'), 2),
        (Some(b':'), 2),
        (Some(b':'), 2),
    ];
    for (position, (separator, width)) in groups.iter().enumerate() {
        if index >= bytes.len() {
            break;
        }
        if let Some(separator) = separator {
            if bytes[index] != *separator {
                if position == 0 {
                    return None;
                }
                break;
            }
            index += 1;
        }
        let digits = text.get(index..index + width)?;
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push_str(digits);
        index += width;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == start || out.len() != 14 {
            return None;
        }
        out.push('.');
        out.push_str(&text[start..end]);
        index = end;
    }
    let rest = &text[index..];
    match rest {
        "" => {}
        "Z" => out.push_str("+0000"),
        _ => {
            let (sign, zone) = rest.split_at(1);
            if !(sign == "+" || sign == "-") {
                return None;
            }
            let (hours, minutes) = zone.split_once(':')?;
            if hours.len() != 2
                || minutes.len() != 2
                || !hours
                    .bytes()
                    .chain(minutes.bytes())
                    .all(|b| b.is_ascii_digit())
            {
                return None;
            }
            out.push_str(sign);
            out.push_str(hours);
            out.push_str(minutes);
        }
    }
    Some(out)
}

type Zone<'a> = Option<(char, &'a str, &'a str)>;

/// Splits a trailing `+ZZzz` / `-ZZzz` zone off a compact timestamp.
fn split_zone(text: &str) -> Option<(&str, Zone<'_>)> {
    match text.rfind(['+', '-']) {
        Some(at) => {
            let zone = &text[at + 1..];
            if zone.len() != 4 || !zone.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let sign = text[at..].chars().next()?;
            Some((&text[..at], Some((sign, &zone[0..2], &zone[2..4]))))
        }
        None => Some((text, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_precision_both_ways() {
        for (compact, fhir) in [
            ("2020", "2020"),
            ("202001", "2020-01"),
            ("20200115", "2020-01-15"),
            ("2020011509", "2020-01-15T09"),
            ("202001150930", "2020-01-15T09:30"),
            ("20200115093005", "2020-01-15T09:30:05"),
            ("20200115093005.1234", "2020-01-15T09:30:05.1234"),
            ("20200115093005-0500", "2020-01-15T09:30:05-05:00"),
            ("202001150930+0130", "2020-01-15T09:30+01:30"),
            ("20200115-0500", "2020-01-15-05:00"),
        ] {
            assert_eq!(from_compact(compact).as_deref(), Some(fhir), "{compact}");
            assert_eq!(to_compact(fhir).as_deref(), Some(compact), "{fhir}");
        }
        assert_eq!(
            to_compact("2020-01-15T09:30:05Z").as_deref(),
            Some("20200115093005+0000")
        );
    }

    #[test]
    fn rejects_what_is_not_a_timestamp() {
        for text in [
            "",
            "20",
            "2020011",
            "2020-01",
            "abcd",
            "20200115.5",
            "2020+05",
        ] {
            assert_eq!(from_compact(text), None, "{text}");
        }
        for text in ["", "2020/01", "2020-1", "2020-01-15T09:30:05+0500", "x"] {
            assert_eq!(to_compact(text), None, "{text}");
        }
    }
}
