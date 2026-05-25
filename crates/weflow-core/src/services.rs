use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::{resolve_account_dir, AppContext, ConfigStore, ProfileConfig};
use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct ServiceHub {
    ctx: AppContext,
    config: ConfigStore,
    profile_name: String,
    db_path_override: Option<String>,
    decrypt_key_override: Option<String>,
    wxid_override: Option<String>,
    pub progress_enabled: bool,
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
        }
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
        let mut wcdb = unsafe { weflow_native::wcdb::Wcdb::load(&self.ctx.runtime_dir) }
            .map_err(|err| AppError::native(err.to_string()))?;
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
        Ok(json!({ "results": results }))
    }

    pub fn contact_type_counts(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contact_type_counts()
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn analytics_overall(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let session_ids = self.all_session_ids(&wcdb)?;
        wcdb.aggregate_stats(&session_ids, 0, 0)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn analytics_rankings(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let session_ids = self.all_session_ids(&wcdb)?;
        let message_counts = wcdb
            .session_message_counts(&session_ids)
            .map_err(|err| AppError::native(err.to_string()))?;
        let contact_counts = wcdb
            .contact_type_counts()
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(json!({
            "messageCounts": message_counts,
            "contactTypeCounts": contact_counts
        }))
    }

    pub fn analytics_time(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let session_ids = self.all_session_ids(&wcdb)?;
        wcdb.aggregate_stats(&session_ids, 0, 0)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn analytics_excluded(&self) -> AppResult<Value> {
        Ok(json!({ "sessions": [] }))
    }

    pub fn group_list(&self) -> AppResult<Value> {
        let sessions = self.sessions()?;
        Ok(filter_group_sessions(sessions))
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

    pub fn group_stats(&self, chatroom_id: &str, view: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let stats = wcdb
            .group_stats(chatroom_id, 0, 0)
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(json!({ "chatroomId": chatroom_id, "view": view, "stats": stats }))
    }

    pub fn group_member(&self, chatroom_id: &str, username: &str) -> AppResult<Value> {
        let members = self.group_members(chatroom_id)?;
        let matched = members
            .get("members")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.to_string().contains(username))
            })
            .cloned()
            .unwrap_or(Value::Null);
        Ok(json!({
            "chatroomId": chatroom_id,
            "username": username,
            "member": matched
        }))
    }

    pub fn report_annual_years(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let session_ids = self.all_session_ids(&wcdb)?;
        wcdb.available_years(&session_ids)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn report_annual_generate(&self, year: i32) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let session_ids = self.all_session_ids(&wcdb)?;
        let (begin, end) = year_bounds(year)?;
        wcdb.annual_report_stats(&session_ids, begin, end)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn report_dual_generate(&self, friend: &str, year: i32) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let (begin, end) = year_bounds(year)?;
        wcdb.dual_report_stats(friend, begin, end)
            .map_err(|err| AppError::native(err.to_string()))
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

    pub fn sns_timeline(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.sns_timeline(100, 0, None, None, 0, 0)
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn sns_users(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.sns_usernames()
            .map_err(|err| AppError::native(err.to_string()))
    }

    pub fn sns_stats(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let profile = self.profile()?;
        let export = wcdb
            .sns_export_stats(self.wxid_override.as_deref().or(profile.wxid.as_deref()))
            .map_err(|err| AppError::native(err.to_string()))?;
        let annual = wcdb.sns_annual_stats(0, 0).unwrap_or(Value::Null);
        Ok(json!({ "export": export, "annual": annual }))
    }

    pub fn sns_block_delete(&self, action: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let result = match action {
            "check" => wcdb.sns_block_delete_check(),
            "install" => wcdb.sns_block_delete_install(),
            "uninstall" => wcdb.sns_block_delete_uninstall(),
            _ => Err(anyhow::anyhow!("unknown sns block-delete action")),
        };
        result.map_err(|err| AppError::native(err.to_string()))
    }

    pub fn sns_delete(&self, post_id: &str) -> AppResult<Value> {
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
        let aes_key_bytes = profile.image_aes_key.as_deref().and_then(|k| {
            let hex = if k.len() >= 32 { &k[..32] } else { return None };
            let mut arr = [0u8; 16];
            for i in 0..16 {
                arr[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(arr)
        });

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

    pub async fn insight_test(&self) -> AppResult<Value> {
        let profile = self.profile()?;
        let config = crate::insight::extract_ai_config(profile)?;
        crate::insight::test_ai_connection(&config).await
    }

    pub async fn insight_trigger(&self, session_id: &str) -> AppResult<Value> {
        let profile = self.profile()?;
        let config = crate::insight::extract_ai_config(profile)?;
        let messages = self.messages(session_id, 30, 0)?;
        let messages_text = serde_json::to_string_pretty(&messages)
            .unwrap_or_default();
        let display_name = self
            .contact(session_id)
            .ok()
            .and_then(|v| {
                v.get("nickname")
                    .or_else(|| v.get("alias"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .unwrap_or_else(|| session_id.to_string());
        let insight = crate::insight::generate_insight(
            &config,
            session_id,
            &display_name,
            &messages_text,
            "manual",
        )
        .await?;
        let mut store = crate::insight::InsightStore::load(&self.ctx.home_dir)?;
        let record = crate::insight::InsightRecord {
            id: format!(
                "insight_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ),
            created_at: crate::insight::now_millis(),
            session_id: session_id.to_string(),
            display_name: display_name.clone(),
            trigger_reason: "manual".to_string(),
            insight: insight.clone(),
            read: false,
        };
        store.add(record);
        store.save()?;
        Ok(json!({ "sessionId": session_id, "insight": insight }))
    }

    pub fn insight_records(&self) -> AppResult<Value> {
        let store = crate::insight::InsightStore::load(&self.ctx.home_dir)?;
        Ok(json!({ "records": store.records() }))
    }

    pub fn insight_get(&self, id: &str) -> AppResult<Value> {
        let store = crate::insight::InsightStore::load(&self.ctx.home_dir)?;
        let record = store.get(id).ok_or_else(|| {
            AppError::runtime(format!("insight record not found: {id}"))
        })?;
        Ok(serde_json::to_value(record).unwrap_or(Value::Null))
    }

    pub fn insight_mark_read(&self, id: &str) -> AppResult<Value> {
        let mut store = crate::insight::InsightStore::load(&self.ctx.home_dir)?;
        if !store.mark_read(id) {
            return Err(AppError::runtime(format!("insight record not found: {id}")));
        }
        store.save()?;
        Ok(json!({ "id": id, "read": true }))
    }

    pub fn insight_clear(&self) -> AppResult<Value> {
        let mut store = crate::insight::InsightStore::load(&self.ctx.home_dir)?;
        store.clear();
        store.save()?;
        Ok(json!({ "cleared": true }))
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

    pub fn key_db(&self) -> AppResult<Value> {
        let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
            .map_err(|err| AppError::native(err.to_string()))?;
        if wxkey.is_available() {
            let pid = find_wechat_pid().ok_or_else(|| {
                AppError::runtime("WeChat process not found; please launch WeChat first")
            })?;
            let key = wxkey.get_db_key(pid).map_err(|err| AppError::native(err.to_string()))?;
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

    pub fn key_image(&self) -> AppResult<Value> {
        let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
            .map_err(|err| AppError::native(err.to_string()))?;
        if wxkey.is_available() {
            let result = wxkey.get_image_key().map_err(|err| AppError::native(err.to_string()))?;
            let parsed: Value = serde_json::from_str(&result).unwrap_or(json!({ "raw": result }));
            return Ok(json!({ "imageKey": parsed, "method": "wx_key" }));
        }
        let profile = self.profile()?;
        if let Some(xor_key) = profile.image_xor_key {
            return Ok(json!({
                "imageKey": { "xorKey": xor_key },
                "method": "config",
                "note": "from stored config; use key scan-image for live extraction"
            }));
        }
        Err(AppError::native("image key not available; configure image_xor_key or use wx_key native library"))
    }

    pub fn key_scan_image(&self, user_dir: &str) -> AppResult<Value> {
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

    // ── Media export ─────────────────────────────────────────────────────────

    pub fn export_media_images(
        &self,
        session_filter: Option<&str>,
        out: &Path,
        media_type: &str,
    ) -> AppResult<Value> {
        let (account_dir, _, _) = self.connection_inputs()?;
        let profile = self.profile()?;
        let xor_key = profile.image_xor_key.map(|k| k as u8).unwrap_or(0);
        let aes_key_bytes = profile.image_aes_key.as_deref().and_then(|k| {
            if k.len() < 32 {
                return None;
            }
            let mut arr = [0u8; 16];
            for i in 0..16 {
                arr[i] = u8::from_str_radix(&k[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(arr)
        });

        std::fs::create_dir_all(out).map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;

        let mut results = Vec::new();
        let mut total_files = 0usize;

        let include_images = media_type == "image" || media_type == "all";
        let include_voice = media_type == "voice" || media_type == "all";

        if include_images {
            let entries = crate::media::scan_image_files(&account_dir);
            let hub = self.clone();
            let exported = crate::media::export_images(
                &entries,
                xor_key,
                aes_key_bytes.as_ref(),
                out,
                session_filter,
                &|current, total| hub.emit_progress("images", "exporting images", current, total),
            )
            .map_err(|e| AppError::runtime(e.to_string()))?;
            total_files += exported.len();
            results.extend(exported);
        }

        if include_voice {
            let entries = crate::media::scan_voice_files(&account_dir);
            let hub = self.clone();
            let exported = crate::media::export_voices(
                &entries,
                out,
                session_filter,
                &|current, total| hub.emit_progress("voice", "exporting voice files", current, total),
            )
            .map_err(|e| AppError::runtime(e.to_string()))?;
            total_files += exported.len();
            results.extend(exported);
        }

        Ok(json!({ "exported": total_files, "out": out, "files": results }))
    }

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
        if self.progress_enabled {
            crate::output::progress(stage, message, current, total);
        }
    }

    fn open_wcdb(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        let (account_dir, key, wxid) = self.connection_inputs()?;
        let mut wcdb = unsafe { weflow_native::wcdb::Wcdb::load(&self.ctx.runtime_dir) }
            .map_err(|err| AppError::native(err.to_string()))?;
        wcdb.open(&account_dir, &key, wxid.as_deref())
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(wcdb)
    }

    fn all_session_ids(&self, wcdb: &weflow_native::wcdb::Wcdb) -> AppResult<Vec<String>> {
        let sessions = wcdb
            .sessions()
            .map_err(|err| AppError::native(err.to_string()))?;
        Ok(extract_session_ids(&sessions))
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
            candidates.push(home.join("Documents/WeChat Files"));
            candidates.push(home.join("Documents/xwechat_files"));
        } else {
            candidates.push(home.join(".xwechat_files"));
            candidates.push(home.join("xwechat_files"));
        }
    }
    candidates
}

fn extract_session_ids(value: &Value) -> Vec<String> {
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

fn filter_group_sessions(value: Value) -> Value {
    let Some(items) = value.as_array() else {
        return value;
    };
    Value::Array(
        items
            .iter()
            .filter(|item| {
                session_id_from_value(item)
                    .map(|session_id| session_id.contains("@chatroom"))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
    )
}

fn year_bounds(year: i32) -> AppResult<(i32, i32)> {
    if !(1970..=2100).contains(&year) {
        return Err(AppError::usage("year must be between 1970 and 2100"));
    }
    let begin = unix_timestamp(year, 1, 1)?;
    let end = unix_timestamp(year + 1, 1, 1)? - 1;
    Ok((begin, end))
}

fn unix_timestamp(year: i32, month: u32, day: u32) -> AppResult<i32> {
    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)
        .ok_or_else(|| AppError::usage("date is out of range"))?;
    i32::try_from(seconds).map_err(|_| AppError::usage("date is out of range"))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era * 146_097 + doe - 719_468)
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
        let output = std::process::Command::new("tasklist")
            .arg("/FI")
            .arg("IMAGENAME eq WeChat.exe")
            .arg("/FO")
            .arg("CSV")
            .arg("/NH")
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("WeChat.exe") {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 2 {
                    let pid_str = parts[1].trim().trim_matches('"');
                    return pid_str.parse().ok();
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
