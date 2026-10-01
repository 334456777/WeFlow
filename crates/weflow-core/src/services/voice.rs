//! Port of the voice pipeline in `chatService.ts` (`getVoiceData`, `preloadVoiceDataBatch`,
//! `resolveVoiceCache`): pull the SILK blob out of the media databases through WCDB, decode it
//! with the vendored SILK SDK and wrap it into a 24 kHz mono WAV, cached on disk.
use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;
use crate::api::normalize_unsigned_token;
use crate::chat_msg;

const VOICE_SAMPLE_RATE: i32 = 24000;

/// `decodeVoiceBlob` for the hex/base64 strings the WCDB API returns.
fn decode_voice_blob(raw: &str) -> Option<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() % 2 == 0 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        let bytes: Option<Vec<u8>> = (0..trimmed.len()).step_by(2).map(|i| u8::from_str_radix(&trimmed[i..i + 2], 16).ok()).collect();
        return bytes;
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(trimmed).ok()
}

/// `createWavBuffer`: 16-bit PCM WAV header + data.
pub fn create_wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// `getVoiceCacheKey`
pub fn voice_cache_key(session_id: &str, msg_id: &str, create_time: Option<i64>) -> String {
    match create_time.filter(|t| *t != 0) {
        Some(t) => format!("{session_id}_{t}_{msg_id}"),
        None => format!("{session_id}_{msg_id}"),
    }
}

impl ServiceHub {
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

    /// `getVoiceDataFromMediaDb`: strict (no self) candidate set first, then the full set; each
    /// plan tries the single native call before the batch call.
    fn voice_silk_from_media_db(&self, wcdb: &weflow_native::wcdb::Wcdb, session_id: &str, create_time: i64, local_id: i64, svr_id: &str, candidates: &[String]) -> Option<Vec<u8>> {
        let my = self.my_wxid_cleaned();
        let ct = create_time.clamp(0, i32::MAX as i64) as i32;
        let lid = local_id.clamp(0, i32::MAX as i64) as i32;
        let svr: i64 = svr_id.parse().unwrap_or(0);
        let mut plans: Vec<Vec<String>> = Vec::new();
        if candidates.is_empty() {
            plans.push(Vec::new());
        } else {
            let strict: Vec<String> = if my.is_empty() { candidates.to_vec() } else { candidates.iter().filter(|c| **c != my).cloned().collect() };
            if !strict.is_empty() && strict.len() != candidates.len() {
                plans.push(strict);
            }
            plans.push(candidates.to_vec());
        }
        for plan in plans {
            let list = serde_json::to_string(&plan).unwrap_or_else(|_| "[]".into());
            if let Ok(hex) = wcdb.voice_data(session_id, ct, lid, svr, &list) {
                if let Some(bytes) = decode_voice_blob(&hex).filter(|b| !b.is_empty()) {
                    return Some(bytes);
                }
            }
            let svr_value: Value = if svr_id.chars().all(|c| c.is_ascii_digit()) && !svr_id.is_empty() && svr_id.len() <= 15 { json!(svr) } else { json!(svr_id) };
            let request = json!([{ "session_id": session_id, "create_time": ct, "local_id": lid, "svr_id": svr_value, "candidates": plan }]);
            if let Ok(rows) = wcdb.voice_data_batch(&request.to_string()) {
                if let Some(hex) = rows.as_array().and_then(|a| a.first()).and_then(|r| r.get("hex")).and_then(Value::as_str) {
                    if let Some(bytes) = decode_voice_blob(hex).filter(|b| !b.is_empty()) {
                        return Some(bytes);
                    }
                }
            }
        }
        None
    }

    fn silk_to_wav(silk: &[u8]) -> AppResult<Vec<u8>> {
        let pcm = weflow_silk::decode(silk, VOICE_SAMPLE_RATE).map_err(|e| AppError::runtime(format!("SILK decode failed: {e}")))?;
        Ok(create_wav(&pcm, VOICE_SAMPLE_RATE as u32, 1))
    }

    /// `chat:getVoiceData`: returns the WAV bytes of a voice message.
    pub fn voice_data(&self, session_id: &str, msg_id: &str, create_time: Option<i64>, server_id: Option<&str>, sender: Option<&str>) -> AppResult<Vec<u8>> {
        let local_id = crate::api::js_parse_int(msg_id).ok_or_else(|| AppError::usage("invalid message id"))?;
        let wcdb = self.open_wcdb()?;
        let mut create_time = create_time.filter(|t| *t != 0);
        let mut sender: Option<String> = sender.filter(|s| !s.is_empty()).map(str::to_string);
        let mut server = server_id.map(normalize_unsigned_token).unwrap_or_default();
        let strong = create_time.map_or(false, |t| t > 0) && !server.is_empty();
        if !strong {
            if let Ok(row) = wcdb.message_by_id(session_id, local_id.clamp(i32::MIN as i64, i32::MAX as i64) as i32) {
                if row.as_object().map_or(false, |o| !o.is_empty()) {
                    let my = self.my_wxid_cleaned();
                    if let Some(m) = chat_msg::map_rows(std::slice::from_ref(&row), &my).into_iter().next() {
                        // localId is not unique across tables: a non-voice hit never overrides the caller's input
                        if m.local_type == 34 {
                            create_time = Some(m.create_time).filter(|t| *t != 0).or(create_time);
                            sender = m.sender_username.clone().filter(|s| !s.is_empty()).or(sender);
                            if !m.server_id_raw.is_empty() && m.server_id_raw != "0" {
                                server = m.server_id_raw.clone();
                            }
                        }
                    }
                }
            }
        }
        let create_time = create_time.ok_or_else(|| AppError::runtime("message timestamp not found"))?;

        let key = voice_cache_key(session_id, &local_id.to_string(), Some(create_time));
        let cache_file = self.voice_cache_dir().join(format!("{key}.wav"));
        if let Ok(bytes) = std::fs::read(&cache_file) {
            if !bytes.is_empty() {
                return Ok(bytes);
            }
        }
        let candidates = self.voice_candidates(session_id, sender.as_deref());
        let silk = self
            .voice_silk_from_media_db(&wcdb, session_id, create_time, local_id, &server, &candidates)
            .ok_or_else(|| AppError::runtime("voice data not found (play the voice message in WeChat once first)"))?;
        let wav = Self::silk_to_wav(&silk)?;
        if std::fs::create_dir_all(self.voice_cache_dir()).is_ok() {
            let _ = std::fs::write(&cache_file, &wav);
        }
        Ok(wav)
    }

    /// `chat:resolveVoiceCache`: the desktop app only consults its in-memory cache here; the CLI has
    /// no long-lived memory, so a hit means a cached WAV for the key without a timestamp exists.
    pub fn voice_resolve_cache(&self, session_id: &str, msg_id: &str) -> Value {
        let file = self.voice_cache_dir().join(format!("{}.wav", voice_cache_key(session_id, msg_id, None)));
        match std::fs::read(&file) {
            Ok(bytes) if !bytes.is_empty() => {
                use base64::Engine;
                json!({ "success": true, "hasCache": true, "data": base64::engine::general_purpose::STANDARD.encode(bytes) })
            }
            _ => json!({ "success": true, "hasCache": false }),
        }
    }

    /// `chat:preloadVoiceDataBatch`: decodes and caches a list of voice messages in chunks.
    pub fn voice_preload(&self, session_id: &str, messages: &[Value]) -> AppResult<Value> {
        let session_id = session_id.trim();
        if session_id.is_empty() || messages.is_empty() {
            return Ok(json!({ "success": true, "prepared": 0 }));
        }
        let wcdb = self.open_wcdb()?;
        let dir = self.voice_cache_dir();
        let my = self.my_wxid_cleaned();
        let mut seen = std::collections::HashSet::new();
        let mut pending: Vec<(String, Value)> = Vec::new();
        for item in messages {
            let n = |k: &str| item.get(k).map(|v| v.as_str().and_then(crate::api::js_parse_int).or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)).unwrap_or(0).max(0);
            let (local_id, create_time) = (n("localId"), n("createTime"));
            if local_id == 0 || create_time == 0 {
                continue;
            }
            let key = voice_cache_key(session_id, &local_id.to_string(), Some(create_time));
            if !seen.insert(key.clone()) {
                continue;
            }
            if std::fs::metadata(dir.join(format!("{key}.wav"))).map_or(false, |m| m.len() > 0) {
                continue;
            }
            let sender = item.get("senderWxid").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let mut candidates: Vec<String> = Vec::new();
            for v in [sender.as_str(), session_id, my.as_str()] {
                if !v.is_empty() && !candidates.iter().any(|c| c == v) {
                    candidates.push(v.to_string());
                }
            }
            let svr = item.get("serverId").cloned().filter(|v| !v.is_null() && v != &json!(0) && v != &json!("")).unwrap_or(json!(0));
            pending.push((key, json!({ "session_id": session_id, "create_time": create_time, "local_id": local_id, "svr_id": svr, "candidates": candidates })));
        }
        let mut prepared = seen.len() - pending.len();
        for chunk in pending.chunks(48) {
            let payload = Value::Array(chunk.iter().map(|(_, r)| r.clone()).collect());
            let Ok(rows) = wcdb.voice_data_batch(&payload.to_string()) else { continue };
            for row in rows.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                let idx = row.get("index").and_then(Value::as_i64).unwrap_or(-1);
                let hex = row.get("hex").and_then(Value::as_str).unwrap_or("").trim();
                if idx < 0 || idx as usize >= chunk.len() || hex.is_empty() {
                    continue;
                }
                let Some(silk) = decode_voice_blob(hex).filter(|b| !b.is_empty()) else { continue };
                let Ok(wav) = Self::silk_to_wav(&silk) else { continue };
                if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(format!("{}.wav", chunk[idx as usize].0)), &wav).is_ok() {
                    prepared += 1;
                }
            }
        }
        Ok(json!({ "success": true, "prepared": prepared }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_decode_from_hex_and_base64() {
        assert_eq!(decode_voice_blob("0aFF"), Some(vec![0x0a, 0xff]));
        assert_eq!(decode_voice_blob("  0aff  "), Some(vec![0x0a, 0xff]));
        assert_eq!(decode_voice_blob("AQID"), Some(vec![1, 2, 3]), "odd-length / non-hex falls back to base64");
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
        assert_eq!(voice_cache_key("wxid_bob", "7", Some(1700000000)), "wxid_bob_1700000000_7");
        assert_eq!(voice_cache_key("wxid_bob", "7", None), "wxid_bob_7");
        assert_eq!(voice_cache_key("wxid_bob", "7", Some(0)), "wxid_bob_7");
    }
}
