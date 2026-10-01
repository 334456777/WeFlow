use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::{resolve_account_dir, AppContext, ConfigStore, ProfileConfig};
use crate::error::{AppError, AppResult};

mod analytics;
mod api;
mod chat;
mod cleanup;
pub use cleanup::normalize_account_id;
mod group;
mod image;
mod insight;
pub use image::{ImagePayload, ImageResult};
pub use insight::Trigger as InsightTrigger;
mod reports;
mod sns;
mod voice;
pub use chat::ResourceQuery;
pub use sns::{SnsExportOptions, SnsMediaFetch, SnsProxyResult, SnsTimelineQuery};

#[derive(Clone)]
pub struct ServiceHub {
    ctx: AppContext,
    config: ConfigStore,
    profile_name: String,
    db_path_override: Option<String>,
    decrypt_key_override: Option<String>,
    wxid_override: Option<String>,
    pub progress_enabled: bool,
    sns_state: std::sync::Arc<std::sync::Mutex<sns::SnsState>>,
    group_state: std::sync::Arc<std::sync::Mutex<group::GroupState>>,
    analytics_state: std::sync::Arc<std::sync::Mutex<analytics::AnalyticsState>>,
    image_state: std::sync::Arc<std::sync::Mutex<image::ImageState>>,
    insight_state: std::sync::Arc<std::sync::Mutex<insight::InsightState>>,
}

impl ServiceHub {
    pub fn new(
        ctx: AppContext,
        config: ConfigStore,
        profile_name: Option<String>,
        db_path_override: Option<String>,
        decrypt_key_override: Option<String>,
        wxid_override: Option<String>,
    ) -> Self {
        let profile_name = profile_name.unwrap_or_else(|| config.current_profile.clone());
        Self {
            ctx,
            config,
            profile_name,
            db_path_override,
            decrypt_key_override,
            wxid_override,
            progress_enabled: false,
            sns_state: Default::default(),
            group_state: Default::default(),
            analytics_state: Default::default(),
            image_state: Default::default(),
            insight_state: Default::default(),
        }
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.ctx.runtime_dir
    }

    pub fn runtime_info(&self) -> Value {
        json!({
            "homeDir": self.ctx.home_dir,
            "configPath": self.ctx.config_path,
            "runtimeDir": self.ctx.runtime_dir,
            "version": self.ctx.version,
            "target": weflow_assets::target_triple(),
            "assetCount": weflow_assets::manifest().entries.len()
        })
    }

    pub fn db_detect(&self) -> Value {
        let candidates = default_db_candidates()
            .into_iter()
            .map(|path| json!({ "path": path, "exists": path.exists() }))
            .collect::<Vec<_>>();
        json!({ "candidates": candidates })
    }

    pub fn db_scan(&self, root: &str) -> Value {
        let root = crate::config::expand_home(root);
        let mut wxids = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if path.join("db_storage").exists()
                    || path.join("FileStorage/Image").exists()
                    || path.join("FileStorage/Image2").exists()
                {
                    wxids.push(json!({ "wxid": name, "path": path }));
                }
            }
        }
        json!({ "root": root, "accounts": wxids })
    }

    pub fn db_test(&self) -> AppResult<Value> {
        let (account_dir, key, _) = self.connection_inputs()?;
        let mut wcdb = weflow_native::wcdb::Wcdb::new();
        wcdb.test_connection(&account_dir, &key)
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(json!({ "accountDir": account_dir, "connected": true }))
    }

    pub fn sessions(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.sessions()
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.messages(session_id, limit, offset)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn latest(&self, session_id: &str, limit: i32) -> AppResult<Value> {
        self.messages(session_id, limit, 0)
    }

    pub fn search(
        &self,
        keyword: &str,
        session_id: Option<&str>,
        limit: i32,
        offset: i32,
        begin: i32,
        end: i32,
    ) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.search(keyword, session_id, limit, offset, begin, end)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn contacts(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contacts()
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn contact(&self, username: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contact(username)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn update_message(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: i32,
        content: &str,
    ) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.update_message(session_id, local_id, create_time, content)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn delete_message(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: i32,
        db_path_hint: Option<&str>,
    ) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.delete_message(session_id, local_id, create_time, db_path_hint)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn anti_revoke(&self, action: &str, sessions: &[String]) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let mut results = Vec::new();
        for session_id in sessions {
            let result = match action {
                "check" => wcdb.anti_revoke_check(session_id),
                "install" => wcdb.anti_revoke_install(session_id),
                "uninstall" => wcdb.anti_revoke_uninstall(session_id),
                _ => Err(anyhow::anyhow!("unknown anti revoke action")),
            };
            match result {
                Ok(data) => {
                    results.push(json!({ "sessionId": session_id, "success": true, "data": data }))
                }
                Err(err) => results.push(
                    json!({ "sessionId": session_id, "success": false, "error": err.to_string() }),
                ),
            }
        }
        // Per-session results are kept for partial failures, but when every session failed the command itself failed
        // (otherwise a script reading only `success` would treat a refused operation as done).
        if !results.is_empty() && results.iter().all(|r| r["success"] == false) {
            let first = results[0]["error"].as_str().unwrap_or("anti-revoke failed").to_string();
            return Err(AppError::native(first));
        }
        Ok(json!({ "results": results }))
    }

    pub fn contact_type_counts(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contact_type_counts()
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn group_members(&self, chatroom_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let members = wcdb
            .group_members(chatroom_id)
            .map_err(|err| AppError::native(err.to_string()))?;
        let nicknames = wcdb.group_nicknames(chatroom_id).unwrap_or(Value::Null);
        let count = wcdb.group_member_count(chatroom_id).unwrap_or(Value::Null);
        Ok(json!({
            "chatroomId": chatroom_id,
            "members": members,
            "nicknames": nicknames,
            "count": count
        }))
    }

    /// `video:getVideoInfo`
    pub fn video_info(&self, md5: &str, include_poster: bool, format: crate::video::PosterFormat) -> AppResult<Value> {
        let account_dir = self.account_dir_only()?;
        Ok(crate::video::video_info(&account_dir.join("msg").join("video"), md5, include_poster, format).to_json())
    }

    /// Path of the on-disk video for a message md5, if WeChat stored one.
    pub(super) fn video_file_path(&self, md5: &str) -> Option<PathBuf> {
        let account_dir = self.account_dir_only().ok()?;
        let info = crate::video::video_info(&account_dir.join("msg").join("video"), md5, false, crate::video::PosterFormat::DataUrl);
        info.video_url.map(PathBuf::from).filter(|p| p.exists())
    }

    pub fn footprint(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let profile = self.profile()?;
        let options = json!({
            "myWxid": self.wxid_override.as_deref().or(profile.wxid.as_deref()).unwrap_or_default(),
            "beginTimestamp": 0,
            "endTimestamp": 0
        });
        wcdb.footprint_stats(&options)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub(crate) fn sns_block_delete(&self, action: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let result = match action {
            "check" => wcdb.sns_block_delete_check(),
            "install" => wcdb.sns_block_delete_install(),
            "uninstall" => wcdb.sns_block_delete_uninstall(),
            _ => Err(anyhow::anyhow!("unknown sns block-delete action")),
        };
        result.map_err(|err| AppError::native(err.to_string()))
    }

    pub(crate) fn sns_delete(&self, post_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.sns_delete_post(post_id)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub async fn sns_download_image(&self, url: &str, out: Option<&Path>) -> AppResult<Value> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|err| AppError::runtime(format!("failed to create HTTP client: {err}")))?;
        let response = client.get(url).send().await.map_err(|err| {
            AppError::runtime(format!("failed to download image: {err}"))
        })?;
        if !response.status().is_success() {
            return Err(AppError::runtime(format!(
                "image download failed with status {}",
                response.status()
            )));
        }
        let bytes = response.bytes().await.map_err(|err| {
            AppError::runtime(format!("failed to read image response: {err}"))
        })?;
        let data = bytes.to_vec();

        let profile = self.profile()?;
        let xor_key = profile.image_xor_key.map(|k| k as u8);
        let aes_key_bytes = profile.image_aes_key.as_deref().and_then(crate::decrypt::parse_aes_key);

        let version = crate::decrypt::detect_dat_version(&data);
        let (final_data, ext) = if version > 0 && xor_key.is_some() {
            let result = crate::decrypt::decrypt_dat(
                &data,
                xor_key.unwrap(),
                aes_key_bytes.as_ref(),
            )
            .map_err(|err| AppError::runtime(err.to_string()))?;
            (result.data, result.ext)
        } else {
            let ext = crate::decrypt::detect_image_extension(&data).to_string();
            (data, ext)
        };

        if let Some(out_path) = out {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| {
                    AppError::runtime(format!("failed to create {}: {err}", parent.display()))
                })?;
            }
            std::fs::write(out_path, &final_data).map_err(|err| {
                AppError::runtime(format!("failed to write {}: {err}", out_path.display()))
            })?;
            Ok(json!({ "url": url, "out": out_path.to_string_lossy(), "ext": ext, "size": final_data.len() }))
        } else {
            let encoded = base64_encode(&final_data);
            Ok(json!({ "url": url, "ext": ext, "size": final_data.len(), "data": format!("data:image/{};base64,{}", ext.trim_start_matches('.'), encoded) }))
        }
    }

    pub fn biz_accounts(&self) -> AppResult<Value> {
        let contacts = self.contacts()?;
        let sessions = self.sessions()?;
        let accounts = crate::biz::filter_official_contacts(&contacts, &sessions);
        Ok(json!({ "accounts": accounts }))
    }

    pub fn biz_messages(&self, username: &str, limit: i32, offset: i32) -> AppResult<Value> {
        let messages = self.messages(username, limit, offset)?;
        let parsed = crate::biz::parse_biz_messages(&messages);
        Ok(json!({ "username": username, "messages": parsed }))
    }

    pub fn biz_pay_records(&self, limit: i32, offset: i32) -> AppResult<Value> {
        let messages = self.messages("gh_3dfda90e39d6", limit, offset)?;
        let records = crate::biz::parse_pay_records(&messages);
        Ok(json!({ "records": records }))
    }

    pub fn insight_footprint(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let profile = self.profile()?;
        let options = json!({
            "myWxid": self.wxid_override.as_deref().or(profile.wxid.as_deref()).unwrap_or_default(),
            "type": "insight_footprint"
        });
        wcdb.footprint_stats(&options).map_err(|err| AppError::native(err.to_string()))
    }

    /// `key:autoGetDbKey`. On Windows WeChat only produces the key while it opens its databases,
    /// so the command keeps polling (default 180 s, like the desktop app) while the user logs in.
    pub fn key_db(&self, pid_override: Option<u32>, timeout_secs: u64) -> AppResult<Value> {
        let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
            .map_err(|err| AppError::native(err.to_string()))?;
        if wxkey.is_available() {
            let pid = match pid_override {
                Some(p) => p,
                None => find_wechat_pid().ok_or_else(|| {
                    AppError::runtime("WeChat process not found (looked for Weixin.exe and WeChat.exe); start WeChat first or pass --pid")
                })?,
            };
            let show = true;
            let mut on_status = |msg: &str, level: i32| {
                if show {
                    eprintln!("{}", json!({ "type": "key_status", "level": level, "message": msg }));
                }
            };
            eprintln!("{}", json!({ "type": "key_waiting", "pid": pid, "timeoutSeconds": timeout_secs, "hint": "log in to WeChat (or restart it) now; the key appears while WeChat opens its databases" }));
            let key = wxkey
                .get_db_key(pid, std::time::Duration::from_secs(timeout_secs), &mut on_status)
                .map_err(|err| match err {
                    weflow_native::wxkey::DbKeyError::AccessDenied(detail) => AppError::native(format!(
                        "permission denied: cannot open the WeChat process (pid {pid}). Run the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator. ({detail})"
                    )),
                    other => AppError::native(other.to_string()),
                })?;
            return Ok(json!({ "key": key, "method": "wx_key", "pid": pid }));
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let result = weflow_native::wxkey::run_key_helper(&self.ctx.runtime_dir, &["--db-key"])
                .map_err(|err| AppError::native(err.to_string()))?;
            let key = result.trim().to_string();
            return Ok(json!({ "key": key, "method": "key_helper" }));
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(AppError::native(
                "wx_key library not found; key extraction requires the platform-specific native library"
            ))
        }
    }

    /// `key:autoGetImageKey`: codes from the `kvcomm` cache, verified per candidate wxid against a
    /// `_t.dat` template found under `user_dir` (default: the account directory).
    pub fn key_image(&self, user_dir: Option<&str>) -> AppResult<Value> {
        let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
            .map_err(|err| AppError::native(err.to_string()))?;
        if !wxkey.is_available() {
            let profile = self.profile()?;
            if let Some(xor_key) = profile.image_xor_key {
                return Ok(json!({
                    "xorKey": xor_key,
                    "aesKey": profile.image_aes_key,
                    "method": "config",
                    "note": "from stored config; use key scan-image for live extraction"
                }));
            }
            return Err(AppError::native("image key not available; configure image_xor_key or use the wx_key native library"));
        }
        let raw = wxkey.get_image_key().map_err(|err| AppError::native(err.to_string()))?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|_| AppError::native("failed to parse the image key data"))?;
        let accounts = parsed.get("accounts").and_then(Value::as_array).cloned().unwrap_or_default();
        let codes: Vec<u64> = accounts.first().and_then(|a| a.get("keys")).and_then(Value::as_array).map(|k| k.iter().filter_map(|k| k.get("code").and_then(Value::as_u64)).collect()).unwrap_or_default();
        if codes.is_empty() {
            return Err(AppError::native("no valid key code found (the kvcomm cache is empty); open a few images in WeChat first"));
        }
        let account_dir = self.account_dir_only().ok();
        let dir_text = user_dir.map(str::to_string).or_else(|| account_dir.as_ref().map(|d| d.to_string_lossy().to_string()));
        let wxid = self.wxid_override.clone().or_else(|| self.profile().ok().and_then(|p| p.wxid.clone()));
        let candidates = crate::keys::collect_wxid_candidates(dir_text.as_deref(), wxid.as_deref());
        let template = dir_text.as_deref().map(Path::new).filter(|d| d.exists()).map(|d| crate::keys::find_template_data(d, 32));
        if let Some((Some(cipher), _)) = &template {
            for cand in &candidates {
                for code in &codes {
                    let (xor, aes) = crate::keys::derive_image_keys(*code, cand);
                    if crate::keys::verify_derived_aes_key(&aes, cipher) {
                        return Ok(json!({ "xorKey": xor, "aesKey": aes, "verified": true, "wxid": cand, "code": code, "method": "wx_key" }));
                    }
                }
            }
            return Err(AppError::native("the cached codes do not match this account's wxid; check --wxid / the account directory, or use `key scan-image`"));
        }
        let fallback_wxid = candidates.first().cloned().or_else(|| accounts.first().and_then(|a| a.get("wxid").and_then(Value::as_str).map(str::to_string))).unwrap_or_else(|| "unknown".into());
        let (xor, aes) = crate::keys::derive_image_keys(codes[0], &fallback_wxid);
        Ok(json!({ "xorKey": xor, "aesKey": aes, "verified": false, "wxid": fallback_wxid, "code": codes[0], "method": "wx_key" }))
    }

    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub fn key_scan_image(&self, user_dir: &str) -> AppResult<Value> {
        #[cfg(target_os = "macos")]
        let expanded = crate::config::expand_home(user_dir);
        #[cfg(target_os = "macos")]
        {
            let result = weflow_native::wxkey::run_image_scan_helper(
                &self.ctx.runtime_dir,
                &[expanded.to_string_lossy().as_ref()],
            )
            .map_err(|err| AppError::native(err.to_string()))?;
            let parsed: Value = serde_json::from_str(&result)
                .unwrap_or(json!({ "raw": result }));
            Ok(json!({ "result": parsed, "method": "image_scan_helper", "userDir": user_dir }))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
                .map_err(|err| AppError::native(err.to_string()))?;
            if wxkey.is_available() {
                let result = wxkey.get_image_key().map_err(|err| AppError::native(err.to_string()))?;
                Ok(json!({ "result": result, "method": "wx_key" }))
            } else {
                Err(AppError::native("image key scanning requires platform-specific native library"))
            }
        }
    }

    // ── Messages TXT export ──────────────────────────────────────────────────

    pub fn export_messages_txt(
        &self,
        session_id: &str,
        start_ts: Option<i64>,
        end_ts: Option<i64>,
        out: &Path,
    ) -> AppResult<Value> {
        // Build nickname map: global contacts first, then group nicknames (higher priority)
        let mut nickname_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        if let Ok(contacts) = self.contacts() {
            if let Some(items) = contacts.as_array() {
                for c in items {
                    let wxid = c
                        .get("username")
                        .or_else(|| c.get("wxid"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let nef = |field: &str| {
                        c.get(field).and_then(Value::as_str).filter(|s| !s.is_empty())
                    };
                    let name = nef("nickName")
                        .or_else(|| nef("remark"))
                        .or_else(|| nef("alias"))
                        .unwrap_or(&wxid)
                        .to_string();
                    if !wxid.is_empty() {
                        nickname_map.insert(wxid, name);
                    }
                }
            }
        }

        if session_id.ends_with("@chatroom") {
            if let Ok(members) = self.group_members(session_id) {
                if let Some(obj) = members.get("nicknames").and_then(Value::as_object) {
                    for (wxid, name) in obj {
                        if let Some(n) = name.as_str() {
                            if !n.is_empty() {
                                nickname_map.insert(wxid.clone(), n.to_string());
                            }
                        }
                    }
                }
            }
        }

        let wcdb = self.open_wcdb()?;
        // The TXT layout reads five columns: keep just those, so a 200,000-message group stays small.
        let mut all_messages: Vec<Value> = Vec::new();
        fetch_message_pages(&wcdb, session_id, start_ts, end_ts, true, &mut |page| {
            all_messages.extend(page.into_iter().map(|mut row| {
                let mut small = serde_json::Map::new();
                for key in ["create_time", "sender_username", "sender_user_name", "senderUserName", "local_type", "message_content", "WCDB_CT_message_content"] {
                    if let Some(v) = row.get_mut(key) {
                        small.insert(key.to_string(), v.take());
                    }
                }
                Value::Object(small)
            }));
        })?;

        // Sort ascending by create_time for chronological output
        all_messages.sort_by_key(|m| {
            m.get("create_time")
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<i64>().ok())
                        .or_else(|| v.as_i64())
                })
                .unwrap_or(0)
        });

        // Supplemental lookup: for any sender still not in nickname_map, query contact table directly
        {
            let mut missing: std::collections::HashSet<String> = std::collections::HashSet::new();
            for m in &all_messages {
                if let Some(wxid) = m
                    .get("sender_user_name")
                    .or_else(|| m.get("senderUserName"))
                    .and_then(Value::as_str)
                {
                    if !wxid.is_empty() && !nickname_map.contains_key(wxid) {
                        missing.insert(wxid.to_string());
                    }
                }
            }
            for wxid in missing {
                if let Ok(c) = self.contact(&wxid) {
                    let nef = |field: &str| {
                        c.get(field).and_then(Value::as_str).filter(|s| !s.is_empty())
                    };
                    if let Some(name) = nef("nickName").or_else(|| nef("remark")).or_else(|| nef("alias")) {
                        nickname_map.insert(wxid, name.to_string());
                    }
                }
            }
        }

        let count = all_messages.len();
        crate::export::export_txt(&all_messages, &nickname_map, out)
            .map_err(|e| AppError::runtime(e.to_string()))?;

        Ok(json!({ "out": out, "count": count, "session": session_id }))
    }

    /// Export a conversation's messages in one of the desktop app's formats
    /// (json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql).
    pub fn export_messages(&self, req: &MessageExportRequest, out: &Path) -> AppResult<Value> {
        let (wcdb, collected) = self.export_collect(req)?;
        self.export_write(req, out, &wcdb, collected)
    }

    /// [`export_messages`](Self::export_messages) that also copies the selected media (images, voices, videos,
    /// stickers) to `<out dir>/media/<out name>/` and points the exported messages at the copies.
    pub async fn export_messages_with_media(&self, req: &MessageExportRequest, out: &Path, media: &crate::api::ApiMediaOptions) -> AppResult<Value> {
        if !media.enabled {
            return self.export_messages(req, out);
        }
        let (wcdb, mut collected) = self.export_collect(req)?;
        let stats = self.attach_export_media(&wcdb, &mut collected, &req.session_id, out, media).await;
        let mut result = self.export_write(req, out, &wcdb, collected)?;
        result["media"] = stats;
        Ok(result)
    }

    fn export_collect(&self, req: &MessageExportRequest) -> AppResult<(weflow_native::wcdb::Wcdb, Vec<crate::message::ExportMsg>)> {
        use crate::message::{collect_messages, CollectOptions};
        let (_, _, wxid) = self.connection_inputs()?;
        let raw_my_wxid = wxid.unwrap_or_default();
        let my_wxid = clean_account_dir_name(&raw_my_wxid);
        let wcdb = self.open_wcdb()?;

        // Raw JSON rows are several times larger than the parsed messages: turn each page into messages as it
        // arrives instead of holding every row of the conversation.
        let opts = CollectOptions { session_id: &req.session_id, my_wxid: &my_wxid, start: req.start, end: req.end, sender_filter: req.sender.as_deref() };
        let mut collected: Vec<crate::message::ExportMsg> = Vec::new();
        fetch_message_pages(&wcdb, &req.session_id, req.start, req.end, false, &mut |page| collected.extend(collect_messages(&page, &opts)))?;
        collected.sort_by(|a, b| a.create_time.cmp(&b.create_time).then(a.local_id.cmp(&b.local_id)));
        if collected.is_empty() {
            return Err(AppError::runtime("no messages found for this session in the given range"));
        }

        Ok((wcdb, collected))
    }

    fn export_write(&self, req: &MessageExportRequest, out: &Path, wcdb: &weflow_native::wcdb::Wcdb, collected: Vec<crate::message::ExportMsg>) -> AppResult<Value> {
        use crate::export_msg::*;
        let (_, _, wxid) = self.connection_inputs()?;
        let raw_my_wxid = wxid.unwrap_or_default();
        let my_wxid = clean_account_dir_name(&raw_my_wxid);
        let mut names = NameBook::new(|username: &str| {
            wcdb.contact(username)
                .ok()
                .filter(|v| v.is_object() && !v.as_object().map_or(true, |o| o.is_empty()))
                .map(|v| ContactInfo::from_value(username, &v))
        });
        let is_group = req.session_id.ends_with("@chatroom");
        let (group_nicks, group_members) = if is_group {
            let nick_value = wcdb.group_nicknames(&req.session_id).unwrap_or(Value::Null);
            let nick_obj = nick_value.get("nicknames").and_then(Value::as_object).or_else(|| nick_value.as_object());
            let entries: Vec<(String, String)> = nick_obj
                .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|n| (k.clone(), n.to_string()))).collect())
                .unwrap_or_default();
            let members_value = wcdb.group_members(&req.session_id).unwrap_or(Value::Null);
            (build_trusted_group_nicknames(entries), extract_member_ids(&members_value))
        } else {
            (std::collections::HashMap::new(), Vec::new())
        };

        let session_display = names.display_name(&req.session_id);
        let session_contact = names.get(&req.session_id);
        let my_display = names.display_name(&my_wxid);
        let mut exporter = Exporter {
            session: SessionInfo {
                id: req.session_id.clone(),
                display_name: session_display,
                nickname: session_contact.as_ref().map(|c| c.nickname.clone()).unwrap_or_default(),
                remark: session_contact.as_ref().map(|c| c.remark.clone()).unwrap_or_default(),
                is_group,
            },
            my_wxid: my_wxid.clone(),
            raw_my_wxid,
            my_display: if my_display == my_wxid { String::new() } else { my_display },
            group_nicks,
            group_members,
            names: &mut names,
            settings: Settings { display_pref: req.display_pref, excel_compact: req.excel_compact, ..Default::default() },
        };
        let format = req.format.to_ascii_lowercase();
        let result = match format.as_str() {
            "chatlab" => exporter.write_chatlab(&collected, out, false),
            "chatlab-jsonl" => exporter.write_chatlab(&collected, out, true),
            "json" => exporter.write_json(&collected, out, false),
            "arkme-json" => exporter.write_json(&collected, out, true),
            "excel" | "xlsx" => exporter.write_excel(&collected, out),
            "txt" => exporter.write_txt(&collected, out),
            "weclone" => exporter.write_weclone(&collected, out),
            "html" => exporter.write_html(&collected, out),
            "sql" => exporter.write_sql(&collected, out),
            other => return Err(AppError::usage(format!("unsupported message export format: {other}; supported: {MESSAGE_EXPORT_FORMATS}"))),
        };
        result.map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(json!({ "out": out, "count": collected.len(), "session": req.session_id, "format": format }))
    }

    // ── Media export ─────────────────────────────────────────────────────────

    pub async fn emoji_download(&self, session_id: &str, out: &Path) -> AppResult<Value> {
        let messages = self.messages(session_id, 500, 0)?;
        let metas = crate::media::extract_emoji_urls(&messages);
        let total = metas.len();
        if total == 0 {
            return Ok(json!({ "found": 0, "downloaded": 0 }));
        }
        std::fs::create_dir_all(out).map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;
        let hub = self.clone();
        let results = crate::media::download_emojis(
            &metas,
            out,
            &|current, t| hub.emit_progress("emoji", "downloading emojis", current, t),
        )
        .await
        .map_err(|e| AppError::runtime(e.to_string()))?;
        let downloaded = results.iter().filter(|v| v["cached"].as_bool() != Some(true)).count();
        Ok(json!({ "found": total, "downloaded": downloaded, "files": results }))
    }

    // ── Backup ────────────────────────────────────────────────────────────────

    pub fn backup_create(
        &self,
        out: &Path,
        include_images: bool,
        include_voice: bool,
        include_emojis: bool,
    ) -> AppResult<Value> {
        let (account_dir, _, _) = self.connection_inputs()?;
        let options = crate::backup::BackupOptions {
            include_images,
            include_voice,
            include_emojis,
        };
        let hub = self.clone();
        let manifest = crate::backup::create_backup(
            &account_dir,
            &options,
            Some(&self.ctx.home_dir),
            out,
            &|current, total| hub.emit_progress("backup", "creating backup", current, total),
        )
        .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(json!({
            "out": out,
            "entries": manifest.entries.len(),
            "wxid": manifest.wxid,
            "createdAt": manifest.created_at
        }))
    }

    pub fn backup_inspect(&self, path: &Path) -> AppResult<Value> {
        let manifest = crate::backup::inspect_backup(path)
            .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(serde_json::to_value(manifest).unwrap_or(Value::Null))
    }

    pub fn backup_restore(&self, path: &Path, target_dir: &Path) -> AppResult<Value> {
        let hub = self.clone();
        crate::backup::restore_backup(
            path,
            target_dir,
            &|current, total| hub.emit_progress("restore", "restoring backup", current, total),
        )
        .map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(json!({ "restoredTo": target_dir }))
    }

    pub fn unsupported(&self, feature: &str) -> AppResult<Value> {
        Err(AppError::runtime(format!(
            "{feature} is not yet ported to the native Rust CLI"
        )))
    }

    pub fn emit_progress(&self, stage: &str, message: &str, current: usize, total: usize) {
        crate::output::progress(stage, message, current, total);
    }

    fn open_wcdb(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        let (account_dir, key, wxid) = self.connection_inputs()?;
        let mut wcdb = weflow_native::wcdb::Wcdb::new();
        wcdb.open(&account_dir, &key, wxid.as_deref())
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(wcdb)
    }

    fn connection_inputs(&self) -> AppResult<(PathBuf, String, Option<String>)> {
        let profile = self.profile()?;
        let db_path = self
            .db_path_override
            .clone()
            .or_else(|| profile.db_path.clone())
            .ok_or_else(|| {
                AppError::config("missing db_path; pass --db-path or run config set db_path")
            })?;
        let key = self
            .decrypt_key_override
            .clone()
            .or_else(|| profile.decrypt_key.clone())
            .ok_or_else(|| {
                AppError::config(
                    "missing decrypt_key; pass --decrypt-key or run config set decrypt_key",
                )
            })?;
        let wxid = self.wxid_override.clone().or_else(|| profile.wxid.clone());
        let account_dir = match &wxid {
            Some(wxid) => resolve_account_dir(&db_path, wxid),
            None => PathBuf::from(db_path),
        };
        Ok((account_dir, key, wxid))
    }

    fn profile(&self) -> AppResult<&ProfileConfig> {
        self.config
            .profile(Some(&self.profile_name))
            .ok_or_else(|| AppError::config(format!("profile not found: {}", self.profile_name)))
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }
}

fn default_db_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        if cfg!(target_os = "macos") {
            candidates.push(home.join("Library/Containers/com.tencent.xinWeChat/Data/Library/Application Support/com.tencent.xinWeChat"));
            candidates.push(home.join("Library/Application Support/com.tencent.xinWeChat"));
            candidates.push(home.join("Library/Containers/com.tencent.WeChat/Data/Library/Application Support/com.tencent.WeChat"));
        } else if cfg!(target_os = "windows") {
            candidates.push(home.join("Documents").join("WeChat Files"));
            candidates.push(home.join("Documents").join("xwechat_files"));
        } else {
            candidates.push(home.join(".xwechat_files"));
            candidates.push(home.join("xwechat_files"));
        }
    }
    candidates
}

pub(crate) fn extract_session_ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(session_id_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn session_id_from_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let object = value.as_object()?;
    for key in [
        "session_id",
        "sessionId",
        "id",
        "username",
        "userName",
        "talker",
        "strUsrName",
    ] {
        if let Some(text) = object.get(key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn find_wechat_pid() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("WeChat")
            .output()
            .ok()?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return stdout.lines().next().and_then(|line| line.trim().parse().ok());
        }
        let output2 = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("微信")
            .output()
            .ok()?;
        if output2.status.success() {
            let stdout = String::from_utf8_lossy(&output2.stdout);
            return stdout.lines().next().and_then(|line| line.trim().parse().ok());
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("wechat")
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().next().and_then(|line| line.trim().parse().ok())
    }
    #[cfg(target_os = "windows")]
    {
        for image in ["Weixin.exe", "WeChat.exe"] {
            let Ok(output) = std::process::Command::new("tasklist").args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"]).output() else { continue };
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("INFO:")) {
                let parts: Vec<&str> = line.split("\",\"").map(|p| p.trim_matches('"')).collect();
                if parts.first().map_or(false, |n| n.eq_ignore_ascii_case(image)) {
                    if let Some(pid) = parts.get(1).and_then(|p| p.parse::<u32>().ok()) {
                        return Some(pid);
                    }
                }
            }
        }
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        result.push(CHARS[(n & 0x3F) as usize] as char);
        i += 3;
    }
    if data.len() - i == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        result.push('=');
    } else if data.len() - i == 1 {
        let n = (data[i] as u32) << 16;
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        result.push('=');
        result.push('=');
    }
    result
}


pub const MESSAGE_EXPORT_FORMATS: &str = "json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql";

pub struct MessageExportRequest {
    pub session_id: String,
    pub format: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub sender: Option<String>,
    pub display_pref: crate::export_msg::DisplayPref,
    pub excel_compact: bool,
}

/// `cleanAccountDirName`: `wxid_abc_1234` -> `wxid_abc`, `name_ab12` -> `name`.
pub fn clean_account_dir_name(dir_name: &str) -> String {
    use crate::message::rx;
    let trimmed = dir_name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.to_lowercase().starts_with("wxid_") {
        return rx(r"(?i)^(wxid_[^_]+)").captures(trimmed).map(|c| c[1].to_string()).unwrap_or_else(|| trimmed.to_string());
    }
    rx(r"^(.+)_([a-zA-Z0-9]{4})$").captures(trimmed).map(|c| c[1].to_string()).unwrap_or_else(|| trimmed.to_string())
}

/// Read a session's messages inside `[start, end)` (Unix seconds) with a bounded WCDB cursor,
/// so the cost follows the range size, not how far back the range lies. Reports progress
/// (`total` is the session's message count when the whole history is requested, else unknown).
/// Reads a conversation page by page (oldest first); `on_page` gets the rows of each page that fall inside
/// `[start, end)`.
fn fetch_message_pages(wcdb: &weflow_native::wcdb::Wcdb, session_id: &str, start: Option<i64>, end: Option<i64>, lite: bool, on_page: &mut dyn FnMut(Vec<Value>)) -> AppResult<()> {
    let clamp = |t: i64| t.clamp(0, i32::MAX as i64) as i32;
    let (begin, finish) = (start.map_or(0, clamp), end.map_or(0, clamp));
    let total = if begin == 0 && finish == 0 { wcdb.message_count(session_id).unwrap_or(0).max(0) as usize } else { 0 };
    let cursor = wcdb
        .open_message_cursor(session_id, 2000, true, begin, finish, lite)
        .map_err(|e| AppError::native(e.to_string()))?;
    let mut seen = 0usize;
    let result = (|| -> AppResult<()> {
        loop {
            let (page, more) = wcdb.fetch_message_batch(cursor).map_err(|e| AppError::native(e.to_string()))?;
            let Value::Array(items) = page else { break };
            if items.is_empty() {
                break;
            }
            let kept: Vec<Value> = items
                .into_iter()
                .filter(|m| {
                    let ts = crate::message::get_timestamp_seconds(m);
                    start.map_or(true, |s| ts >= s) && end.map_or(true, |e| ts < e)
                })
                .collect();
            seen += kept.len();
            on_page(kept);
            crate::output::progress("messages", "reading messages", seen, if total == 0 { 0 } else { total.max(seen) });
            if !more {
                break;
            }
        }
        Ok(())
    })();
    let _ = wcdb.close_message_cursor(cursor);
    result
}

fn extract_member_ids(value: &Value) -> Vec<String> {
    let list = value
        .as_array()
        .cloned()
        .or_else(|| value.get("members").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let id = match &item {
            Value::String(s) => s.clone(),
            other => ["username", "userName", "wxid", "user_name"]
                .iter()
                .find_map(|k| other.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string(),
        };
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn cleans_account_dir_names() {
        assert_eq!(clean_account_dir_name("wxid_abc123_ab12"), "wxid_abc123");
        assert_eq!(clean_account_dir_name("someone_a1b2"), "someone");
        assert_eq!(clean_account_dir_name("plain"), "plain");
        assert_eq!(clean_account_dir_name(" "), "");
    }

    #[test]
    fn member_ids_from_various_shapes() {
        let v = json!({"members": [{"username": "a"}, {"userName": "b"}, "c", {"username": "a"}]});
        assert_eq!(extract_member_ids(&v), vec!["a", "b", "c"]);
        assert_eq!(extract_member_ids(&json!(["x", "y"])), vec!["x", "y"]);
        assert!(extract_member_ids(&Value::Null).is_empty());
    }
}

/// `normalizeTimestamp` of the WCDB wrapper: ms → s, clamped to i32.
pub(crate) fn normalize_timestamp(input: i64) -> i32 {
    if input <= 0 {
        return 0;
    }
    let seconds = if input > 1_000_000_000_000 { input / 1000 } else { input };
    seconds.clamp(0, i32::MAX as i64) as i32
}

/// `normalizeRange`: open end → now, end never before begin.
pub(crate) fn normalize_range(begin: i64, end: i64) -> (i32, i32) {
    let b = normalize_timestamp(begin);
    let mut e = normalize_timestamp(end);
    if e <= 0 {
        e = normalize_timestamp(chrono::Utc::now().timestamp_millis());
    }
    if b > 0 && e < b {
        e = b;
    }
    (b, e)
}
