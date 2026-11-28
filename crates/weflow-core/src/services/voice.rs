//! 语音消息：通过 `wcdb` 门面从媒体数据库中取出 SILK 数据，用内置的 SILK SDK 解码，
//! 封装成 24 kHz 单声道的 WAV，缓存在磁盘上。
use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;
use crate::chat_msg;
use crate::message::row::unsigned_token;

const VOICE_SAMPLE_RATE: i32 = 24000;

/// `wcdb` 门面返回的十六进制或 base64 字符串的字节；为空或解不开时为 `None`。
fn decode_voice_blob(raw: &str) -> Option<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len().is_multiple_of(2) && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        let bytes: Option<Vec<u8>> = (0..trimmed.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&trimmed[i..i + 2], 16).ok())
            .collect();
        return bytes;
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()
}

/// 16 位 PCM 的 WAV 文件：文件头，后接 `pcm`。
fn create_wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    // WAV 文件头里的大小是 32 位的；语音消息远远达不到 4 GiB
    let data_len = u32::try_from(pcm.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data_len.saturating_add(36).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// 解码后的语音缓存所用的文件名：会话、发送时间（知道时）和消息 ID。
fn voice_cache_key(session_id: &str, msg_id: &str, create_time: Option<i64>) -> String {
    match create_time.filter(|t| *t != 0) {
        Some(t) => format!("{session_id}_{t}_{msg_id}"),
        None => format!("{session_id}_{msg_id}"),
    }
}

impl ServiceHub {
    /// 解码后的语音存放的目录：缓存目录下的 `Voices`。
    pub fn voice_cache_dir(&self) -> PathBuf {
        self.cache_base().join("Voices")
    }

    fn voice_candidates(&self, session_id: &str, sender: Option<&str>) -> Vec<String> {
        let my = self.my_wxid_cleaned();
        let mut c: Vec<String> = Vec::new();
        for v in [sender.unwrap_or(""), session_id, my.as_str()] {
            if !v.is_empty() && !c.iter().any(|x| x == v) {
                c.push(v.to_string());
            }
        }
        c
    }

    /// 媒体数据库中一条语音消息的 SILK 数据。
    ///
    /// 候选的用户名先用不含自己的集合，再用完整的集合；每一轮先试单条的调用，再试批量的调用。
    fn voice_silk_from_media_db(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session_id: &str,
        create_time: i64,
        local_id: i64,
        svr_id: &str,
        candidates: &[String],
    ) -> Option<Vec<u8>> {
        let my = self.my_wxid_cleaned();
        let ct = crate::num::saturating_i32(create_time.max(0));
        let lid = crate::num::saturating_i32(local_id.max(0));
        let svr: i64 = svr_id.parse().unwrap_or(0);
        let mut plans: Vec<Vec<String>> = Vec::new();
        if candidates.is_empty() {
            plans.push(Vec::new());
        } else {
            let strict: Vec<String> = if my.is_empty() {
                candidates.to_vec()
            } else {
                candidates.iter().filter(|c| **c != my).cloned().collect()
            };
            if !strict.is_empty() && strict.len() != candidates.len() {
                plans.push(strict);
            }
            plans.push(candidates.to_vec());
        }
        for plan in plans {
            if let Ok(hex) = wcdb.voice_data(ct, lid, svr, &plan) {
                if let Some(bytes) = decode_voice_blob(&hex).filter(|b| !b.is_empty()) {
                    return Some(bytes);
                }
            }
            let svr_value: Value = if svr_id.chars().all(|c| c.is_ascii_digit())
                && !svr_id.is_empty()
                && svr_id.len() <= 15
            {
                json!(svr)
            } else {
                json!(svr_id)
            };
            let request = json!([{ "session_id": session_id, "create_time": ct, "local_id": lid, "svr_id": svr_value, "candidates": plan }]);
            if let Ok(rows) = wcdb.voice_data_batch(&request) {
                if let Some(hex) = rows
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|r| r.get("hex"))
                    .and_then(Value::as_str)
                {
                    if let Some(bytes) = decode_voice_blob(hex).filter(|b| !b.is_empty()) {
                        return Some(bytes);
                    }
                }
            }
        }
        None
    }

    fn silk_to_wav(silk: &[u8]) -> AppResult<Vec<u8>> {
        let pcm = weflow_silk::decode(silk, VOICE_SAMPLE_RATE)
            .map_err(|e| AppError::runtime(format!("SILK decode failed: {e}")))?;
        Ok(create_wav(&pcm, VOICE_SAMPLE_RATE as u32, 1))
    }

    /// 语音消息的 WAV 数据，有缓存时取缓存。
    ///
    /// # Errors
    ///
    /// 消息 ID 无效或消息不是语音时返回用法错误；找不到消息的时间或语音数据时返回 `not_found`；
    /// SILK 解码失败时返回运行时错误；账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn voice_data(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
        server_id: Option<&str>,
        sender: Option<&str>,
    ) -> AppResult<Vec<u8>> {
        let local_id = crate::api::js_parse_int(msg_id)
            .ok_or_else(|| AppError::usage("invalid message id"))?;
        let wcdb = self.open_wcdb()?;
        self.voice_data_of(&wcdb, session_id, local_id, create_time, server_id, sender)
    }

    /// 用已经打开的数据库 `wcdb` 读取 [`voice_data`](Self::voice_data)：导出许多条语音时只打开一次数据库，
    /// 而不是每条语音打开一次。
    pub(super) fn voice_data_with(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
        server_id: Option<&str>,
        sender: Option<&str>,
    ) -> AppResult<Vec<u8>> {
        let local_id = crate::api::js_parse_int(msg_id)
            .ok_or_else(|| AppError::usage("invalid message id"))?;
        self.voice_data_of(wcdb, session_id, local_id, create_time, server_id, sender)
    }

    /// 通过 `wcdb` 读取消息 `local_id` 的 [`voice_data`](Self::voice_data)。
    fn voice_data_of(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session_id: &str,
        local_id: i64,
        create_time: Option<i64>,
        server_id: Option<&str>,
        sender: Option<&str>,
    ) -> AppResult<Vec<u8>> {
        let mut create_time = create_time.filter(|t| *t != 0);
        let mut sender: Option<String> = sender.filter(|s| !s.is_empty()).map(str::to_string);
        let mut server = server_id.and_then(unsigned_token).unwrap_or_default();
        let strong = create_time.is_some_and(|t| t > 0) && !server.is_empty();
        if !strong {
            // 多个消息库可能都有这个本地 ID：先按发送时间、再按语音类型选出一条
            if let Ok(Some(row)) =
                chat::message_by_local_id(wcdb, session_id, local_id, create_time, Some(34))
            {
                // 查找时只在这个 ID 对应多条消息时才优先选语音：找到的那条就是要的消息，不论它是什么类型
                if row.local_type & 0xFFFF_FFFF != 34 {
                    return Err(AppError::usage("the message is not a voice message"));
                }
                let my = self.my_wxid_cleaned();
                let table_name = weflow_native::msg::table_name_for(session_id);
                let m = chat_msg::map_msg_row(row, &table_name, &my);
                // 本地 ID 在不同的表之间不唯一：只有找到的是语音时，才用它的时间、发送者和服务器 ID 替换调用方给的值
                if m.local_type == 34 {
                    create_time = Some(m.create_time).filter(|t| *t != 0).or(create_time);
                    sender = m
                        .sender_username
                        .clone()
                        .filter(|s| !s.is_empty())
                        .or(sender);
                    if !m.server_id_raw.is_empty() && m.server_id_raw != "0" {
                        server = m.server_id_raw;
                    }
                }
            }
        }
        // 调用方没给时间时，时间取自语音消息；这时仍然没有时间，说明没找到这条消息（#822）
        let create_time =
            create_time.ok_or_else(|| AppError::not_found("message timestamp not found"))?;

        let key = voice_cache_key(session_id, &local_id.to_string(), Some(create_time));
        let cache_file = self.voice_cache_dir().join(format!("{key}.wav"));
        if let Ok(bytes) = std::fs::read(&cache_file) {
            if !bytes.is_empty() {
                return Ok(bytes);
            }
        }
        let candidates = self.voice_candidates(session_id, sender.as_deref());
        let silk = self
            .voice_silk_from_media_db(
                wcdb,
                session_id,
                create_time,
                local_id,
                &server,
                &candidates,
            )
            .ok_or_else(|| {
                AppError::not_found(
                    "voice data not found\nPlay the voice message in WeChat once first.",
                )
            })?;
        let wav = Self::silk_to_wav(&silk)?;
        if std::fs::create_dir_all(self.voice_cache_dir()).is_ok() {
            let _ = crate::fs_atomic::write_atomically(&cache_file, &wav);
        }
        Ok(wav)
    }

    /// 只从缓存中取消息解码后的语音（`chat voice-cache`），不做解码。
    ///
    /// 先找 [`voice_data`](Self::voice_data) 和 [`voice_preload`](Self::voice_preload) 按消息的发送时间写下的
    /// WAV（时间取 `create_time`，没有时按本地 ID 查语音消息自己的时间），再找不带时间的键下的文件（#1203）。
    pub fn voice_resolve_cache(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> crate::contract::VoiceCache {
        let create_time = create_time.filter(|t| *t > 0).or_else(|| {
            let local_id = crate::api::js_parse_int(msg_id)?;
            let wcdb = self.open_wcdb().ok()?;
            let row =
                chat::message_by_local_id(&wcdb, session_id, local_id, None, Some(34)).ok()??;
            (row.local_type & 0xFFFF_FFFF == 34)
                .then_some(row.create_time)
                .flatten()
                .filter(|t| *t > 0)
        });
        let dir = self.voice_cache_dir();
        let cached = create_time
            .map(Some)
            .into_iter()
            .chain([None])
            .map(|t| dir.join(format!("{}.wav", voice_cache_key(session_id, msg_id, t))))
            .find_map(|file| std::fs::read(file).ok().filter(|b| !b.is_empty()));
        match cached {
            Some(bytes) => {
                use base64::Engine;
                crate::contract::VoiceCache {
                    has_cache: true,
                    data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
                }
            }
            None => crate::contract::VoiceCache {
                has_cache: false,
                data: None,
            },
        }
    }

    /// 分批解码并缓存一组语音消息，返回缓存中已有和新写入的条数。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误；某条语音取不到或解码失败时跳过，不算失败。
    pub fn voice_preload(
        &self,
        session_id: &str,
        messages: &[Value],
    ) -> AppResult<crate::contract::VoicesPrepared> {
        let session_id = session_id.trim();
        if session_id.is_empty() || messages.is_empty() {
            return Ok(crate::contract::VoicesPrepared { prepared: 0 });
        }
        let wcdb = self.open_wcdb()?;
        let dir = self.voice_cache_dir();
        let my = self.my_wxid_cleaned();
        let mut seen = std::collections::HashSet::new();
        let mut pending: Vec<(String, Value)> = Vec::new();
        for item in messages {
            let n = |k: &str| {
                item.get(k)
                    .map(|v| {
                        v.as_str()
                            .and_then(crate::api::js_parse_int)
                            .or_else(|| v.as_f64().map(crate::num::saturating_i64))
                            .unwrap_or(0)
                    })
                    .unwrap_or(0)
                    .max(0)
            };
            let (local_id, create_time) = (n("localId"), n("createTime"));
            if local_id == 0 || create_time == 0 {
                continue;
            }
            let key = voice_cache_key(session_id, &local_id.to_string(), Some(create_time));
            if !seen.insert(key.clone()) {
                continue;
            }
            if std::fs::metadata(dir.join(format!("{key}.wav"))).is_ok_and(|m| m.len() > 0) {
                continue;
            }
            let sender = item
                .get("senderWxid")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let mut candidates: Vec<String> = Vec::new();
            for v in [sender.as_str(), session_id, my.as_str()] {
                if !v.is_empty() && !candidates.iter().any(|c| c == v) {
                    candidates.push(v.to_string());
                }
            }
            let svr = item
                .get("serverId")
                .cloned()
                .filter(|v| !v.is_null() && v != &json!(0) && v != &json!(""))
                .unwrap_or(json!(0));
            pending.push((key, json!({ "session_id": session_id, "create_time": create_time, "local_id": local_id, "svr_id": svr, "candidates": candidates })));
        }
        let mut prepared = seen.len() - pending.len();
        for chunk in pending.chunks(48) {
            let payload = Value::Array(chunk.iter().map(|(_, r)| r.clone()).collect());
            let Ok(rows) = wcdb.voice_data_batch(&payload) else {
                continue;
            };
            for row in rows.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                let idx = row.get("index").and_then(Value::as_i64).unwrap_or(-1);
                let hex = row.get("hex").and_then(Value::as_str).unwrap_or("").trim();
                let Some(idx) = usize::try_from(idx).ok().filter(|&i| i < chunk.len()) else {
                    continue;
                };
                if hex.is_empty() {
                    continue;
                }
                let Some(silk) = decode_voice_blob(hex).filter(|b| !b.is_empty()) else {
                    continue;
                };
                let Ok(wav) = Self::silk_to_wav(&silk) else {
                    continue;
                };
                if std::fs::create_dir_all(&dir).is_ok()
                    && crate::fs_atomic::write_atomically(
                        &dir.join(format!("{}.wav", chunk[idx].0)),
                        &wav,
                    )
                    .is_ok()
                {
                    prepared += 1;
                }
            }
        }
        Ok(crate::contract::VoicesPrepared { prepared })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_decode_from_hex_and_base64() {
        assert_eq!(decode_voice_blob("0aFF"), Some(vec![0x0a, 0xff]));
        assert_eq!(decode_voice_blob("  0aff  "), Some(vec![0x0a, 0xff]));
        assert_eq!(
            decode_voice_blob("AQID"),
            Some(vec![1, 2, 3]),
            "odd-length / non-hex falls back to base64"
        );
        assert_eq!(decode_voice_blob(""), None);
    }

    #[test]
    fn wav_header_matches_the_desktop_layout() {
        let wav = create_wav(&[1, 2, 3, 4], 24000, 1);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 48000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[44..], &[1, 2, 3, 4]);
        assert_eq!(wav.len(), 48);
    }

    #[test]
    fn cache_keys_include_the_timestamp_when_known() {
        assert_eq!(
            voice_cache_key("wxid_bob", "7", Some(1700000000)),
            "wxid_bob_1700000000_7"
        );
        assert_eq!(voice_cache_key("wxid_bob", "7", None), "wxid_bob_7");
        assert_eq!(voice_cache_key("wxid_bob", "7", Some(0)), "wxid_bob_7");
    }
}
