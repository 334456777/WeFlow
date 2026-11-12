//! The fields of a raw row, as text or numbers.

use serde_json::Value;

pub fn row_field<'a>(row: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let obj = row.as_object()?;
    for key in keys {
        if let Some(v) = obj.get(*key) {
            match v {
                Value::Null => continue,
                Value::String(s) if s.is_empty() => continue,
                _ => return Some(v),
            }
        }
    }
    None
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn row_int(row: &Value, keys: &[&str], fallback: i64) -> i64 {
    let Some(obj) = row.as_object() else {
        return fallback;
    };
    for key in keys {
        if let Some(v) = obj.get(*key) {
            // integer columns (nearly every row): same result as the text parse below, without its two allocations
            if let Some(n) = v.as_i64() {
                return n;
            }
            let text = match v {
                Value::Null => continue,
                Value::String(s) if s.is_empty() => continue,
                other => value_to_string(other),
            };
            let digits: String = text
                .trim()
                .char_indices()
                .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
                .map(|(_, c)| c)
                .collect();
            if let Ok(n) = digits.parse::<i64>() {
                return n;
            }
        }
    }
    fallback
}

/// An unsigned id in decimal without leading zeros (a float is cut to its whole part); `None` when it is empty,
/// zero or not a number.
pub fn unsigned_token(value: &str) -> Option<String> {
    let raw = value.trim();
    if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        let digits = raw.trim_start_matches('0');
        return (!digits.is_empty()).then(|| digits.to_string());
    }
    match raw.parse::<f64>() {
        // a float displays without an exponent, so a whole number prints as an integer
        Ok(n) if n.is_finite() && n >= 1.0 => Some(format!("{}", n.floor())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_int_reads_numbers_and_numeric_text_alike() {
        let row = serde_json::json!({
            "a": 42, "b": -7, "c": "  123abc", "d": null, "e": "", "f": 1.9, "g": u64::MAX, "h": "+5"
        });
        assert_eq!(row_int(&row, &["a"], 0), 42);
        assert_eq!(row_int(&row, &["b"], 0), -7);
        assert_eq!(row_int(&row, &["c"], 0), 123);
        // null and empty text fall through to the next key
        assert_eq!(row_int(&row, &["d", "e", "a"], 0), 42);
        assert_eq!(row_int(&row, &["f"], 0), 1);
        // out of range for i64: skipped like any unparsable value
        assert_eq!(row_int(&row, &["g", "b"], 0), -7);
        assert_eq!(row_int(&row, &["h"], 0), 5);
        assert_eq!(row_int(&row, &["missing"], 9), 9);
    }
}
