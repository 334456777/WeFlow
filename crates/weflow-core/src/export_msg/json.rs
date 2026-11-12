//! The detailed JSON export.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::message::appmsg::{
    extract_arkme_app_message_meta_in, extract_arkme_contact_card_meta,
    quote_or_link_app_message_meta_in,
};
use crate::message::kind::message_type_name_in;
use crate::message::time::format_timestamp;
use crate::message::xml::MsgXml;
use crate::message::*;

use super::arkme::ArkSender;
#[cfg(test)]
use super::arkme::{ArkCompact, ArkGroupMember, ArkMessage};
use super::format::write_pretty;
use super::names::{resolve_group_nickname, ContactInfo, Profile};
use super::spool::{EntryFormat, RenderedEntries, SenderTally, Spool};
use super::Exporter;

/// The key of a detailed-JSON message's first field, whose value the writer sets (see
/// [`Exporter::render_entries`]).
const JSON_LOCAL_ID_KEY: &[u8] = b"\"localId\": ";

/// Writes `entry`, a detailed-JSON message rendered ahead, with `local_id` as the value of its `localId`.
fn write_json_entry(w: &mut impl io::Write, entry: &[u8], local_id: usize) -> io::Result<()> {
    // `localId` is the first field, so the first match is the key and not text inside a later value
    let Some(key) = memchr::memmem::find(entry, JSON_LOCAL_ID_KEY) else {
        return w.write_all(entry);
    };
    let start = key + JSON_LOCAL_ID_KEY.len();
    let digits = entry[start..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    w.write_all(&entry[..start])?;
    write!(w, "{local_id}")?;
    w.write_all(&entry[start + digits..])
}

impl<'a, 'n> Exporter<'a, 'n> {
    /// Detailed JSON: the entries go to a spool file, numbered as they are written; the session header, which comes
    /// first in the file and has the totals, is written at the end.
    pub(super) fn write_rendered_json(
        &mut self,
        out: &Path,
        next: &mut dyn FnMut(&mut Self) -> Option<RenderedEntries>,
        write_if_empty: bool,
    ) -> Result<usize> {
        let mut spool = Spool::create(out)?;
        let mut last_ts: Option<i64> = None;
        let mut count = 0usize;
        while let Some(page) = next(self) {
            last_ts = last_ts.max(page.latest);
            for text in page.entries() {
                count += 1;
                spool.push_with(",\n    ", |w| write_json_entry(w, text, count))?;
            }
        }
        if count == 0 && !write_if_empty {
            return Ok(0);
        }
        self.write_json_file(out, &mut spool, count, last_ts, None)?;
        Ok(count)
    }

    // ─────────────────────────── detailed JSON / arkme-json ───────────────────────────

    fn weflow_header(&self, format: Option<&'static str>) -> WeflowHeader {
        WeflowHeader {
            version: "1.0.3",
            exported_at: self.settings.exported_at,
            generator: "WeFlow",
            format,
        }
    }

    /// The detailed-JSON object of one message; `index` is the number of messages before it. Records the sender's
    /// profile in `profiles` (the first one seen wins).
    pub(super) fn json_message<'m>(
        &mut self,
        msg: &'m ExportMsg,
        arkme: bool,
        index: usize,
        profiles: &mut HashMap<String, Profile>,
    ) -> JsonMessage<'m> {
        let source = crate::xml::tag_element(&msg.content, "msgsource").unwrap_or("");
        let x = MsgXml::new(&msg.content);
        let content = match msg.media.as_ref().filter(|_| msg.local_type != 47) {
            Some(m) => m.relative_path.clone(),
            None => self.message_text_in(msg, &x),
        };
        let quoted = self.quoted_with_names(msg.local_type, &x);
        let sender = msg.sender_username.as_str();
        let contact = self.names.get(sender);
        let nickname = contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                contact
                    .as_ref()
                    .map_or_else(|| sender.to_string(), ContactInfo::display_name)
            });
        let remark = contact.map(|c| c.remark).unwrap_or_default();
        let group_nick = resolve_group_nickname(&self.group_nicks, &[sender]);
        let display = self
            .settings
            .display_pref
            .pick(sender, &nickname, &remark, &group_nick);
        if !profiles.contains_key(sender) {
            profiles.insert(
                sender.to_string(),
                Profile {
                    wxid: sender.to_string(),
                    nickname,
                    remark,
                    group_nickname: group_nick,
                    display_name: display.clone(),
                    ..Default::default()
                },
            );
        }

        // later keys replace the value of an earlier one in place (the quoted sender and text of a quote)
        let mut extra = Map::new();
        // detailed JSON keeps the app-message fields of quotes and links only
        let meta = if arkme {
            extract_arkme_app_message_meta_in(&x, msg.local_type)
        } else {
            quote_or_link_app_message_meta_in(&x, msg.local_type)
        };
        if let Some(meta) = meta {
            extra.extend(meta);
        }
        if let Some(q) = &quoted {
            if let Some(s) = q.quoted_sender.as_ref().filter(|s| !s.is_empty()) {
                extra.insert("quotedSender".into(), json!(s));
            }
            if !q.quoted_preview.is_empty() {
                extra.insert("quotedContent".into(), json!(q.quoted_preview));
            }
        }
        if arkme {
            if let Some(meta) = extract_arkme_contact_card_meta(&msg.content, msg.local_type) {
                extra.extend(meta);
            }
        }

        let (emoji, image, video, location) = (
            msg.local_type == 47,
            msg.local_type == 3,
            msg.local_type == 43,
            msg.local_type == 48,
        );
        JsonMessage {
            local_id: index + 1,
            create_time: msg.create_time,
            formatted_time: format_timestamp(msg.create_time),
            kind: message_type_name_in(msg.local_type, &x),
            local_type: msg.local_type,
            content,
            is_send: u8::from(msg.is_send),
            sender_username: sender,
            sender_display_name: display,
            source,
            sender_avatar_key: sender,
            emoji_md5: msg.emoji_md5.as_deref().filter(|_| emoji),
            emoji_cdn_url: msg.emoji_cdn_url.as_deref().filter(|_| emoji),
            emoji_caption: msg.emoji_caption.as_deref().filter(|_| emoji),
            image_md5: msg.image_md5.as_deref().filter(|_| image),
            image_dat_name: msg.image_dat_name.as_deref().filter(|_| image),
            video_md5: msg.video_md5.as_deref().filter(|_| video),
            platform_message_id: msg.platform_message_id(),
            reply_to_message_id: x.reply_to_message_id(),
            extra,
            location_lat: msg.location_lat.filter(|_| location),
            location_lng: msg.location_lng.filter(|_| location),
            location_poiname: msg.location_poiname.as_deref().filter(|_| location),
            location_label: msg.location_label.as_deref().filter(|_| location),
        }
    }

    /// `{ wxid, nickname, remark, displayName, type, lastTimestamp, messageCount }` of the exported session.
    fn json_session(&mut self, count: usize, last_ts: Option<i64>) -> JsonSession {
        let is_group = self.session.is_group;
        let session_contact = self.names.get(&self.session.id.clone());
        let session_nickname = session_contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.session.display_name.clone());
        let session_remark = session_contact
            .as_ref()
            .map(|c| c.remark.clone())
            .unwrap_or_default();
        let session_group_nick = if is_group {
            resolve_group_nickname(&self.group_nicks, &[self.session.id.as_str()])
        } else {
            String::new()
        };
        let session_display = self.settings.display_pref.pick(
            &self.session.id,
            &session_nickname,
            &session_remark,
            &session_group_nick,
        );
        JsonSession {
            wxid: self.session.id.clone(),
            nickname: session_nickname,
            remark: session_remark,
            display_name: session_display,
            kind: if is_group { "群聊" } else { "私聊" },
            last_timestamp: last_ts,
            message_count: count,
        }
    }

    /// The whole detailed-JSON (or arkme-json) document, with the messages sorted by time.
    #[cfg(test)]
    fn json_document<'m>(&mut self, msgs: &'m [ExportMsg], arkme: bool) -> JsonDocument<'m> {
        let mut profiles: HashMap<String, Profile> = HashMap::new();
        let mut messages: Vec<JsonMessage<'m>> = Vec::with_capacity(msgs.len());
        for (count, msg) in msgs.iter().enumerate() {
            messages.push(self.json_message(msg, arkme, count, &mut profiles));
        }
        messages.sort_by_key(|m| m.create_time);
        let last_ts = messages.iter().map(|m| m.create_time).max();
        let session = self.json_session(messages.len(), last_ts);
        if !arkme {
            return JsonDocument {
                weflow: self.weflow_header(None),
                session,
                senders: None,
                messages: JsonMessages::Detailed(messages),
                group_members: None,
            };
        }

        // arkme-json: compact messages + sender table (+ group members)
        let mut ark = ArkCompact::default();
        let compact: Vec<ArkMessage<'m>> = messages
            .into_iter()
            .map(|m| ark.compact(m, &|s| profiles.get(s).cloned()))
            .collect();
        let group_members = self.session.is_group.then(|| self.ark_group_members(msgs));
        JsonDocument {
            weflow: self.weflow_header(Some("arkme-json")),
            session,
            senders: Some(ark.senders),
            messages: JsonMessages::Arkme(compact),
            group_members,
        }
    }

    /// The whole document, built in memory: what the tests hold the streamed file against.
    #[cfg(test)]
    pub(super) fn build_json(&mut self, msgs: &[ExportMsg], arkme: bool) -> Value {
        serde_json::to_value(self.json_document(msgs, arkme)).unwrap_or_default()
    }

    pub fn write_json(&mut self, msgs: &[ExportMsg], out: &Path, arkme: bool) -> Result<()> {
        // written message by message in the order given, as a streaming export writes them
        if arkme {
            self.stream_arkme_json(msgs, out, true)?;
        } else {
            self.render_and_write(EntryFormat::Json, out, msgs, true)?;
        }
        Ok(())
    }

    /// Writes the detailed-JSON file, or with `arkme` (its sender table and a tally of the senders) the arkme-json
    /// file, around the `count` messages in `spool`.
    pub(super) fn write_json_file(
        &mut self,
        out: &Path,
        spool: &mut Spool,
        count: usize,
        last_ts: Option<i64>,
        arkme: Option<(&[ArkSender], SenderTally)>,
    ) -> Result<()> {
        use std::io::Write;
        let session = self.json_session(count, last_ts);
        let header = self.weflow_header(arkme.is_some().then_some("arkme-json"));
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
        w.write_all(b"{\n  \"weflow\": ")?;
        write_pretty(&mut w, &header, 2)?;
        w.write_all(b",\n  \"session\": ")?;
        write_pretty(&mut w, &session, 2)?;
        let tally = match arkme {
            Some((senders, tally)) => {
                w.write_all(b",\n  \"senders\": ")?;
                write_pretty(&mut w, &senders, 2)?;
                Some(tally)
            }
            None => None,
        };
        w.write_all(b",\n  \"messages\": ")?;
        if count == 0 {
            w.write_all(b"[]")?;
        } else {
            w.write_all(b"[\n    ")?;
            spool.copy_to(&mut w)?;
            w.write_all(b"\n  ]")?;
        }
        if let Some(tally) = tally.filter(|_| self.session.is_group) {
            let members = self.ark_group_members_of(tally);
            w.write_all(b",\n  \"groupMembers\": ")?;
            write_pretty(&mut w, &members, 2)?;
        }
        w.write_all(b"\n}")?;
        w.flush()?;
        Ok(())
    }
}

/// The `weflow` header of a detailed-JSON / arkme-json export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WeflowHeader {
    version: &'static str,
    exported_at: i64,
    generator: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'static str>,
}

/// The `session` object of a detailed-JSON / arkme-json export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSession {
    wxid: String,
    nickname: String,
    remark: String,
    display_name: String,
    #[serde(rename = "type")]
    kind: &'static str,
    last_timestamp: Option<i64>,
    message_count: usize,
}

/// One message of the detailed-JSON export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JsonMessage<'m> {
    pub(super) local_id: usize,
    pub(super) create_time: i64,
    pub(super) formatted_time: String,
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) local_type: i64,
    pub(super) content: String,
    pub(super) is_send: u8,
    pub(super) sender_username: &'m str,
    sender_display_name: String,
    pub(super) source: &'m str,
    sender_avatar_key: &'m str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) emoji_md5: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) emoji_cdn_url: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) emoji_caption: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_md5: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_dat_name: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_md5: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) platform_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reply_to_message_id: Option<String>,
    /// The app-message and contact-card fields ([`extract_arkme_app_message_meta_in`],
    /// [`extract_arkme_contact_card_meta`]) and the quoted sender and text: which keys appear depends on the message
    /// XML, so they stay a JSON map, in the order they were found.
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) location_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) location_lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) location_poiname: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) location_label: Option<&'m str>,
}

/// The messages of a detailed-JSON or arkme-json document.
#[cfg(test)]
#[derive(Serialize)]
#[serde(untagged)]
enum JsonMessages<'m> {
    Detailed(Vec<JsonMessage<'m>>),
    Arkme(Vec<ArkMessage<'m>>),
}

/// A whole detailed-JSON or arkme-json export (`senders` and `groupMembers` are arkme's).
#[cfg(test)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDocument<'m> {
    weflow: WeflowHeader,
    session: JsonSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    senders: Option<Vec<ArkSender>>,
    messages: JsonMessages<'m>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_members: Option<Vec<ArkGroupMember>>,
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;

    #[test]
    fn detailed_json_and_arkme() {
        let v = with_exporter(false, |ex, msgs| ex.build_json(msgs, false));
        assert_eq!(v["weflow"]["generator"], "WeFlow");
        assert_eq!(v["session"]["type"], "私聊");
        assert_eq!(v["session"]["messageCount"], 5);
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m[0]["formattedTime"].as_str().unwrap().len(), 19);
        assert_eq!(m[0]["type"], "文本消息");
        assert_eq!(m[3]["appMsgKind"], "link");
        assert_eq!(m[3]["linkUrl"], "https://example.com/x");
        assert_eq!(m[4]["quotedSender"], "Me");

        let a = with_exporter(true, |ex, msgs| ex.build_json(msgs, true));
        assert_eq!(a["weflow"]["format"], "arkme-json");
        let senders = a["senders"].as_array().unwrap();
        assert_eq!(senders[0]["senderID"], 1);
        assert!(a["messages"][0].get("senderID").is_some());
        assert!(a["messages"][0].get("senderUsername").is_none());
        assert!(a["groupMembers"].as_array().unwrap().len() >= 2);
    }
}
