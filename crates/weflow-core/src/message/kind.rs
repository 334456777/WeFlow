//! Message types: WeChat's `local_type` to the export's type numbers and names.

use super::xml::{extract_app_message_type, normalize_app_message_content, MsgXml};

/// The export's type code of a message: its local type, or for an app message the kind its [`MsgXml`] names.
pub fn convert_message_type_in(local_type: i64, x: &MsgXml) -> i64 {
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
        let sub = xml_type.unwrap_or(0);
        match sub {
            6 => return 4,
            19 => return 7,
            33 | 36 => return 24,
            57 => return 25,
            2000 => return 99,
            5 | 49 => return 7,
            _ => {
                if xml_type.is_some_and(|t| t != 0) || looks_app {
                    return 7;
                }
            }
        }
    }
    match local_type {
        1 => 0,
        3 => 1,
        34 => 2,
        43 => 3,
        49 | 34359738417 | 103079215153 | 25769803825 => 7,
        47 => 5,
        48 => 8,
        42 => 27,
        50 => 23,
        10000 => 80,
        _ => 99,
    }
}

/// The type name of a message: its local type's, or for an app message the name of the kind its [`MsgXml`] names.
pub fn message_type_name_in(local_type: i64, x: &MsgXml) -> &'static str {
    {
        if !x.content().is_empty() {
            if x.readable_system_twice(local_type) {
                return "系统消息";
            }
            match x.app_type_twice() {
                "3" => return "音乐消息",
                "87" => return "群公告",
                "2000" => return "转账消息",
                "5" => return "链接消息",
                "6" => return "文件消息",
                "19" => return "聊天记录",
                "33" | "36" => return "小程序消息",
                "57" => return "引用消息",
                _ => {}
            }
        }
    }
    message_type_name_of(local_type)
}

/// The type name of a message by the local type alone.
fn message_type_name_of(local_type: i64) -> &'static str {
    match local_type {
        1 => "文本消息",
        3 => "图片消息",
        34 => "语音消息",
        42 => "名片消息",
        43 => "视频消息",
        47 => "动画表情",
        48 => "位置消息",
        49 => "链接消息",
        50 => "通话消息",
        10000 => "系统消息",
        244813135921 => "引用消息",
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
