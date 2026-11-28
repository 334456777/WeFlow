//! Message types: WeChat's `local_type` to the export's type numbers and names.

use super::appmsg::{app_message_kind_in, app_type, row_kept_in, row_of_kind, AppType};
use super::xml::{extract_app_message_type, normalize_app_message_content, MsgXml};

/// The export's type code of a message: its local type, or for an app message the kind its [`MsgXml`] names.
pub fn convert_message_type_in(local_type: i64, x: &MsgXml<'_>) -> i64 {
    let normalized = x.once();
    if x.readable_system_twice(local_type) {
        return 80;
    }
    let xml_type_raw = x.app_type_twice();
    let xml_type: Option<i64> = xml_type_raw.parse().ok();
    // Only app messages (local_type 49, or any message carrying an `<appmsg>`) are classified by their XML subtype.
    // A bare `<msg>` must not count: image, voice, video, call and sticker messages are all stored as `<msg>...`
    // XML, and treating that as "app message" turned every one of them into a LINK in the ChatLab export
    // (the TypeScript exporter has the same check).
    let looks_app = local_type == 49 || normalized.contains("<appmsg");
    if looks_app || xml_type.is_some_and(|t| t != 0) {
        // the row of the app message types (#1180), else of the kind its XML gives it (#1263); any other app
        // message is a link
        return app_row(local_type, x).map_or(7, |row| row.chatlab);
    }
    // an app message whose XML says nothing: by the `<type>` its local type keeps, as one kept as 49 for a `<type>`
    // the table does not know (#1263)
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

/// The type name of a message: its local type's, or for an app message the name of the kind its [`MsgXml`] names.
pub fn message_type_name_in(local_type: i64, x: &MsgXml<'_>) -> &'static str {
    {
        if !x.content().is_empty() {
            if x.readable_system_twice(local_type) {
                return "系统消息";
            }
            // the row of the app message types, whatever `local_type` keeps (#1180)
            if let Some(row) = app_row(local_type, x) {
                return row.name;
            }
        }
    }
    message_type_name_of(local_type)
}

/// The row of an app message's own `<type>`; for a type the table does not know, the row of the kind
/// [`app_message_kind_in`] gives it, as the chat window and the detailed JSON name it (#1263).
fn app_row(local_type: i64, x: &MsgXml<'_>) -> Option<AppType> {
    app_type(x.app_type_twice())
        .or_else(|| app_message_kind_in(x, local_type).and_then(row_of_kind))
}

/// The type name of a message by the local type alone: an app message kept as `(<type> << 32) | 49` by its
/// `<type>`'s, as one kept as 49 for a `<type>` the table does not know (#1263).
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

    /// A type the table does not know is named by the kind its XML gives it, as the chat window and the detailed
    /// JSON name it (#1263); one with no kind of its own stays a link.
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

    /// An app message kept as `(<type> << 32) | 49` is typed and named by the row of that `<type>`, whether its XML
    /// names the type or says nothing; as one kept as 49 for a `<type>` the table does not know (#1263).
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
