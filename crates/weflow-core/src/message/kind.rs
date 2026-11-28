//! 消息类型：把微信的 `local_type` 换算成导出用的类型码和类型名。

use super::appmsg::{app_message_kind_in, app_type, row_kept_in, row_of_kind, AppType};
use super::xml::{extract_app_message_type, normalize_app_message_content, MsgXml};

/// 消息在导出中的类型码（ChatLab 的 `type`）：可读的系统消息为 80；app 消息取 [`MsgXml`] 给出的 `<type>`
/// 或种类在表中的那一行；其余按 `local_type`。
pub fn convert_message_type_in(local_type: i64, x: &MsgXml<'_>) -> i64 {
    let normalized = x.once();
    if x.readable_system_twice(local_type) {
        return 80;
    }
    let xml_type_raw = x.app_type_twice();
    let xml_type: Option<i64> = xml_type_raw.parse().ok();
    // 只有 app 消息（`local_type` 为 49、带 `<appmsg>`，或 XML 中有非零的 `<type>`）按 XML 的子类型归类。只有
    // `<msg>` 不算：图片、语音、视频、通话和表情消息的 XML 都是 `<msg>…`，把它们当作 app 消息时，它们在 ChatLab
    // 导出中都会成为链接（桌面版的导出有同样的判断）。
    let looks_app = local_type == 49 || normalized.contains("<appmsg");
    if looks_app || xml_type.is_some_and(|t| t != 0) {
        // 取 app 消息类型表中的那一行（#1180），没有时取 XML 判出的种类的那一行（#1263）；都没有的 app 消息算链接
        return app_row(local_type, x).map_or(7, |row| row.chatlab);
    }
    // XML 没有写出类型的 app 消息：按 `local_type` 高位保存的 `<type>`；
    // 表中没有这个 `<type>` 时与存为 49 的一样（#1263）
    if local_type & 0xFFFF_FFFF == 49 {
        return row_kept_in(local_type).map_or(7, |row| row.chatlab);
    }
    match local_type {
        1 => 0,
        3 => 1,
        34 => 2,
        43 => 3,
        47 => 5,
        48 => 8,
        42 => 27,
        50 => 23,
        10000 => 80,
        _ => 99,
    }
}

/// 消息的类型名（导出的「类型」列）：有内容时，可读的系统消息为「系统消息」，app 消息取 [`MsgXml`] 给出的
/// `<type>` 或种类在表中的那一行；其余按 `local_type`。
pub fn message_type_name_in(local_type: i64, x: &MsgXml<'_>) -> &'static str {
    {
        if !x.content().is_empty() {
            if x.readable_system_twice(local_type) {
                return "系统消息";
            }
            // 取 app 消息类型表中的那一行，不论 `local_type` 的高位存了什么（#1180）
            if let Some(row) = app_row(local_type, x) {
                return row.name;
            }
        }
    }
    message_type_name_of(local_type)
}

/// app 消息自己的 `<type>` 在表中的那一行；表中没有这个 `<type>` 时，
/// 取 [`app_message_kind_in`] 判出的种类的那一行，与聊天窗口和 `json` 导出的叫法一致（#1263）。
fn app_row(local_type: i64, x: &MsgXml<'_>) -> Option<AppType> {
    app_type(x.app_type_twice())
        .or_else(|| app_message_kind_in(x, local_type).and_then(row_of_kind))
}

/// 只按 `local_type` 求的类型名：存为 `(<type> << 32) | 49` 的 app 消息取这个 `<type>` 在表中的名称，
/// 表中没有时与存为 49 的一样，为「链接消息」（#1263）。
fn message_type_name_of(local_type: i64) -> &'static str {
    if local_type & 0xFFFF_FFFF == 49 {
        return row_kept_in(local_type).map_or("链接消息", |row| row.name);
    }
    match local_type {
        1 => "文本消息",
        3 => "图片消息",
        34 => "语音消息",
        42 => "名片消息",
        43 => "视频消息",
        47 => "动画表情",
        48 => "位置消息",
        50 => "通话消息",
        10000 => "系统消息",
        _ => "其他消息",
    }
}

/// WeClone 导出的消息类型：文本、图片、表情、视频、语音和位置按 `local_type`，`<type>` 为 6 的 app 消息为
/// `file`，其余为 `text`。
pub fn weclone_type_name(local_type: i64, content: &str) -> &'static str {
    match local_type {
        1 => return "text",
        3 => return "image",
        47 => return "sticker",
        43 => return "video",
        34 => return "voice",
        48 => return "location",
        _ => {}
    }
    let normalized = normalize_app_message_content(content);
    if (local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>"))
        && extract_app_message_type(&normalized) == "6"
    {
        return "file";
    }
    "text"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表中没有的 `<type>` 按 XML 判出的种类命名，与聊天窗口和 `json` 导出一致（#1263）；
    /// 种类在表中没有对应行的（`card`）仍算链接。
    #[test]
    fn a_type_the_table_does_not_know_is_named_by_its_kind() {
        let reply = "<msg><appmsg><title>好</title><type>999</type>\
                     <refermsg><type>1</type><content>hi</content></refermsg></appmsg></msg>";
        assert_eq!(message_type_name_in(49, &MsgXml::new(reply)), "引用消息");
        assert_eq!(convert_message_type_in(49, &MsgXml::new(reply)), 25);
        let finder = "<msg><appmsg><title>直播</title><type>63</type>\
                      <finderUsername>v2_x</finderUsername></appmsg></msg>";
        assert_eq!(message_type_name_in(49, &MsgXml::new(finder)), "视频号消息");
        assert_eq!(convert_message_type_in(49, &MsgXml::new(finder)), 7);
        let card = "<msg><appmsg><title>T</title><type>999</type></appmsg></msg>";
        assert_eq!(message_type_name_in(49, &MsgXml::new(card)), "链接消息");
        assert_eq!(convert_message_type_in(49, &MsgXml::new(card)), 7);
    }

    /// 存为 `(<type> << 32) | 49` 的 app 消息，不论 XML 是否写出类型，
    /// 类型码和类型名都取这个 `<type>` 在表中的那一行；表中没有这个 `<type>` 时与存为 49 的相同（#1263）。
    #[test]
    fn an_app_message_is_told_by_the_type_it_is_kept_as() {
        let mut known = 0;
        for ty in 1..=2100_i64 {
            let kept = (ty << 32) | 49;
            let named = format!("<msg><appmsg><title>T</title><type>{ty}</type></appmsg></msg>");
            let row = app_type(&ty.to_string());
            known += usize::from(row.is_some());
            for content in ["", named.as_str()] {
                let x = MsgXml::new(content);
                let want = row.map_or_else(
                    || {
                        (
                            message_type_name_in(49, &x),
                            convert_message_type_in(49, &x),
                        )
                    },
                    |row| (row.name, row.chatlab),
                );
                let read = (
                    message_type_name_in(kept, &x),
                    convert_message_type_in(kept, &x),
                );
                assert_eq!(read, want, "{ty}: {content}");
            }
        }
        assert!(known > 10, "{known} types");
    }
}
