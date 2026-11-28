//! 按文字或数字读取 JSON 形式的原始行的字段，以及无符号 ID 的十进制写法。

use serde_json::Value;

/// `row` 中 `keys` 里第一个有值的字段。
pub fn row_field<'a>(row: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let obj = row.as_object()?;
    for key in keys {
        // 值为 null 或空字符串时看下一个键
        match obj.get(*key) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.is_empty() => {}
            Some(v) => return Some(v),
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

/// `row` 中 `keys` 里第一个能读成整数的字段：整数直接取，其他值取文字开头的数字（可带正负号）；都读不出时为
/// `fallback`。
pub fn row_int(row: &Value, keys: &[&str], fallback: i64) -> i64 {
    let Some(obj) = row.as_object() else {
        return fallback;
    };
    for key in keys {
        if let Some(v) = obj.get(*key) {
            // 整数列（几乎每一行都是）：结果与下面按文字解析的相同，省去它的两次分配
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

/// 十进制、不带前导零的无符号 ID（小数取整数部分）；为空、不是数字，或取整后不是正数时为 `None`。
pub fn unsigned_token(value: &str) -> Option<String> {
    let raw = value.trim();
    if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        let digits = raw.trim_start_matches('0');
        return (!digits.is_empty()).then(|| digits.to_string());
    }
    match raw.parse::<f64>() {
        // f64 的 `{}` 不用指数写法，整数值打印出来没有小数部分
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
        // null 和空字符串跳过，看下一个键
        assert_eq!(row_int(&row, &["d", "e", "a"], 0), 42);
        assert_eq!(row_int(&row, &["f"], 0), 1);
        // 超出 i64 的范围：与读不出的值一样跳过
        assert_eq!(row_int(&row, &["g", "b"], 0), -7);
        assert_eq!(row_int(&row, &["h"], 0), 5);
        assert_eq!(row_int(&row, &["missing"], 9), 9);
    }
}
