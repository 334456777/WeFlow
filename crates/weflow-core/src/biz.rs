use serde_json::{json, Value};

pub fn extract_xml_value(xml: &str, tag_name: &str) -> String {
    let pattern = format!("<{}>", tag_name);
    let close_pattern = format!("</{}>", tag_name);
    let Some(start) = xml.find(&pattern) else {
        return String::new();
    };
    let content_start = start + pattern.len();
    let Some(end) = xml[content_start..].find(&close_pattern) else {
        return String::new();
    };
    let content = &xml[content_start..content_start + end];
    content
        .replace("<![CDATA[", "")
        .replace("]]>", "")
        .trim()
        .to_string()
}

fn fallback_or_empty(primary: &str, extract_fn: impl FnOnce() -> String) -> String {
    if primary.is_empty() {
        extract_fn()
    } else {
        primary.to_string()
    }
}

pub fn parse_biz_content_list(xml_str: &str) -> Vec<Value> {
    if xml_str.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::new();
    let mut search_from = 0;
    while let Some(start) = xml_str[search_from..].find("<item>") {
        let abs_start = search_from + start + 6;
        if let Some(end) = xml_str[abs_start..].find("</item>") {
            let item_xml = &xml_str[abs_start..abs_start + end];
            let title = extract_xml_value(item_xml, "title");
            if !title.is_empty() {
                let cover = fallback_or_empty(
                    &extract_xml_value(item_xml, "cover"),
                    || extract_xml_value(item_xml, "thumburl"),
                );
                let summary = fallback_or_empty(
                    &extract_xml_value(item_xml, "summary"),
                    || extract_xml_value(item_xml, "digest"),
                );
                items.push(json!({
                    "title": title,
                    "url": extract_xml_value(item_xml, "url"),
                    "cover": cover,
                    "summary": summary,
                }));
            }
            search_from = abs_start + end + 7;
        } else {
            break;
        }
    }
    items
}

pub fn parse_pay_xml(xml_str: &str) -> Option<Value> {
    if xml_str.is_empty() {
        return None;
    }
    let title = extract_xml_value(xml_str, "title");
    let description = extract_xml_value(xml_str, "des");
    if title.is_empty() && description.is_empty() {
        return None;
    }
    let merchant_name = {
        let mn = extract_xml_value(xml_str, "display_name");
        if mn.is_empty() { "微信支付".to_string() } else { mn }
    };
    let pub_time_str = extract_xml_value(xml_str, "pub_time");
    let pub_time: i64 = pub_time_str.parse().unwrap_or(0);
    Some(json!({
        "title": title,
        "description": description,
        "merchantName": merchant_name,
        "merchantIcon": extract_xml_value(xml_str, "icon_url"),
        "timestamp": pub_time
    }))
}

pub fn filter_official_contacts(contacts: &Value, sessions: &Value) -> Vec<Value> {
    let session_map = build_session_map(sessions);
    let Some(items) = contacts.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|c| {
            let contact_type = c
                .get("type")
                .or_else(|| c.get("contactType"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            contact_type == 3 || contact_type == 99
        })
        .map(|c| {
            let username = c
                .get("username")
                .or_else(|| c.get("userName"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let last_time = session_map
                .get(username)
                .and_then(|s| s.get("lastTime").or_else(|| s.get("createTime")))
                .cloned()
                .unwrap_or(Value::Null);
            let mut contact = c.clone();
            if let Some(obj) = contact.as_object_mut() {
                obj.insert("lastTime".to_string(), last_time);
            }
            contact
        })
        .collect()
}

fn build_session_map(sessions: &Value) -> std::collections::HashMap<String, Value> {
    let mut map = std::collections::HashMap::new();
    let Some(items) = sessions.as_array() else {
        return map;
    };
    for item in items {
        let key = item
            .get("username")
            .or_else(|| item.get("userName"))
            .or_else(|| item.get("session_id"))
            .or_else(|| item.get("sessionId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !key.is_empty() {
            map.insert(key, item.clone());
        }
    }
    map
}

pub fn parse_biz_messages(messages: &Value) -> Vec<Value> {
    let Some(items) = messages.as_array() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for msg in items {
        let content = msg
            .get("content")
            .or_else(|| msg.get("rawContent"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if content.is_empty() {
            continue;
        }
        let title = extract_xml_value(content, "title");
        let des = extract_xml_value(content, "des");
        let url = extract_xml_value(content, "url");
        let cover = fallback_or_empty(
            &extract_xml_value(content, "cover"),
            || extract_xml_value(content, "thumburl"),
        );
        let content_list_str = extract_xml_value(content, "content_list");
        let content_list = parse_biz_content_list(&content_list_str);

        result.push(json!({
            "localId": msg.get("localId").or_else(|| msg.get("local_id")).unwrap_or(&Value::Null),
            "createTime": msg.get("createTime").or_else(|| msg.get("create_time")).unwrap_or(&Value::Null),
            "title": title,
            "description": des,
            "url": url,
            "cover": cover,
            "contentList": content_list,
        }));
    }
    result
}

pub fn parse_pay_records(messages: &Value) -> Vec<Value> {
    let Some(items) = messages.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|msg| {
            let content = msg
                .get("content")
                .or_else(|| msg.get("rawContent"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let record = parse_pay_xml(content)?;
            let mut rec = record;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert(
                    "localId".to_string(),
                    msg.get("localId")
                        .or_else(|| msg.get("local_id"))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                obj.insert(
                    "createTime".to_string(),
                    msg.get("createTime")
                        .or_else(|| msg.get("create_time"))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            Some(rec)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_xml_value() {
        let xml = r#"<xml><title>Hello</title><des>World</des></xml>"#;
        assert_eq!(extract_xml_value(xml, "title"), "Hello");
        assert_eq!(extract_xml_value(xml, "des"), "World");
        assert_eq!(extract_xml_value(xml, "missing"), "");
    }

    #[test]
    fn strips_cdata() {
        let xml = r#"<xml><title><![CDATA[My Title]]></title></xml>"#;
        assert_eq!(extract_xml_value(xml, "title"), "My Title");
    }

    #[test]
    fn parses_pay_xml() {
        let xml = r#"<xml><title>Payment</title><des>¥100</des><display_name>Shop</display_name><pub_time>1700000000</pub_time></xml>"#;
        let result = parse_pay_xml(xml).unwrap();
        assert_eq!(result["title"], "Payment");
        assert_eq!(result["description"], "¥100");
        assert_eq!(result["merchantName"], "Shop");
        assert_eq!(result["timestamp"], 1700000000);
    }

    #[test]
    fn parses_pay_xml_empty() {
        assert!(parse_pay_xml("").is_none());
        assert!(parse_pay_xml("<xml><a>1</a></xml>").is_none());
    }

    #[test]
    fn filters_official_contacts() {
        let contacts = json!([
            {"username": "gh_abc", "type": 3, "nickname": "Official"},
            {"username": "friend_1", "type": 1, "nickname": "Friend"}
        ]);
        let sessions = json!([
            {"username": "gh_abc", "lastTime": 1700000000}
        ]);
        let result = filter_official_contacts(&contacts, &sessions);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["username"], "gh_abc");
        assert_eq!(result[0]["lastTime"], 1700000000);
    }

    #[test]
    fn parses_biz_content_list() {
        let xml = r#"<content_list><item><title>A1</title><url>http://a1</url><cover>http://img</cover></item><item><title>A2</title><url>http://a2</url></item></content_list>"#;
        let result = parse_biz_content_list(xml);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["title"], "A1");
        assert_eq!(result[1]["title"], "A2");
    }
}
