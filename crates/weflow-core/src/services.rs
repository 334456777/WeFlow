use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::{resolve_account_dir, AppContext, ConfigStore, ProfileConfig};
use crate::error::{AppError, AppResult};

mod analytics;
mod api;
mod chat;
mod cleanup;
mod export_stream;
pub use cleanup::{disk_usage, normalize_account_id, CachePart};
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
            "assetCount": weflow_assets::manifest(&self.ctx.version).entries.len()
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

    /// The wxid of every account found in the data directory (`root`, else the configured `db_path`, else the
    /// default locations), as `config set wxid` expects it (without the `_ab12` suffix of the folder name).
    pub fn db_wxid(&self, root: Option<&str>) -> AppResult<Value> {
        let configured = root
            .map(str::to_string)
            .or_else(|| self.db_path_override.clone())
            .or_else(|| self.profile().ok().and_then(|p| p.db_path.clone()));
        let roots: Vec<PathBuf> = match configured {
            Some(root) => vec![crate::config::expand_home(&root)],
            None => default_db_candidates()
                .into_iter()
                .filter(|p| p.exists())
                .collect(),
        };
        if roots.is_empty() {
            return Err(AppError::config(
                "no WeChat data directory found; pass the directory or run config set db_path",
            ));
        }
        let mut accounts = Vec::new();
        for root in &roots {
            for (wxid, path) in find_accounts(root) {
                accounts.push(json!({ "wxid": wxid, "path": path }));
            }
        }
        if accounts.is_empty() {
            return Err(AppError::runtime(format!(
                "no WeChat account directory found in {}",
                roots[0].display()
            )));
        }
        Ok(json!({ "accounts": accounts }))
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
            let first = results[0]["error"]
                .as_str()
                .unwrap_or("anti-revoke failed")
                .to_string();
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
    pub fn video_info(
        &self,
        md5: &str,
        include_poster: bool,
        format: crate::video::PosterFormat,
    ) -> AppResult<Value> {
        let account_dir = self.account_dir_only()?;
        Ok(crate::video::video_info(
            &account_dir.join("msg").join("video"),
            md5,
            include_poster,
            format,
        )
        .to_json())
    }

    /// Path of the on-disk video for a message md5, if WeChat stored one.
    pub(super) fn video_file_path(&self, md5: &str) -> Option<PathBuf> {
        let account_dir = self.account_dir_only().ok()?;
        let info = crate::video::video_info(
            &account_dir.join("msg").join("video"),
            md5,
            false,
            crate::video::PosterFormat::DataUrl,
        );
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
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|err| AppError::runtime(format!("failed to download image: {err}")))?;
        if !response.status().is_success() {
            return Err(AppError::runtime(format!(
                "image download failed with status {}",
                response.status()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|err| AppError::runtime(format!("failed to read image response: {err}")))?;
        let data = bytes.to_vec();

        let profile = self.profile()?;
        let xor_key = profile.image_xor_key.map(|k| k as u8);
        let aes_key_bytes = profile
            .image_aes_key
            .as_deref()
            .and_then(crate::decrypt::parse_aes_key);

        let version = crate::decrypt::detect_dat_version(&data);
        let (final_data, ext) = match xor_key {
            Some(xor_key) if version > 0 => {
                let result = crate::decrypt::decrypt_dat(&data, xor_key, aes_key_bytes.as_ref())
                    .map_err(|err| AppError::runtime(err.to_string()))?;
                (result.data, result.ext)
            }
            _ => {
                let ext = crate::decrypt::detect_image_extension(&data).to_string();
                (data, ext)
            }
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
            Ok(
                json!({ "url": url, "out": out_path.to_string_lossy(), "ext": ext, "size": final_data.len() }),
            )
        } else {
            let encoded = base64_encode(&final_data);
            Ok(
                json!({ "url": url, "ext": ext, "size": final_data.len(), "data": format!("data:image/{};base64,{}", ext.trim_start_matches('.'), encoded) }),
            )
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
        wcdb.footprint_stats(&options)
            .map_err(|err| AppError::native(err.to_string()))
    }

    /// `key:autoGetDbKey`. On Windows WeChat only produces the key while it opens its databases, so the command
    /// asks the user to quit and reopen WeChat, hooks the new process and waits (default 180 s in total) for the
    /// user to click "Enter WeChat".
    pub fn key_db(&self, pid_override: Option<u32>, timeout_secs: u64) -> AppResult<Value> {
        let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
            .map_err(|err| AppError::native(err.to_string()))?;
        if wxkey.is_available() {
            return self.key_db_hooked(&wxkey, pid_override, timeout_secs);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let result = weflow_native::wxkey::run_key_helper(&self.ctx.runtime_dir, &["--db-key"])
                .map_err(|err| AppError::native(err.to_string()))?;
            let key = result.trim().to_string();
            Ok(json!({ "decrypt_key": key, "method": "key_helper" }))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err(AppError::native(
                "wx_key library not found; key extraction requires the platform-specific native library"
            ))
        }
    }

    /// Hooks WeChat and waits for the database key; shared by Windows and macOS.
    fn key_db_hooked(
        &self,
        wxkey: &weflow_native::wxkey::WxKey,
        pid_override: Option<u32>,
        timeout_secs: u64,
    ) -> AppResult<Value> {
        use crate::output::{end_status_line, status_line};
        use weflow_native::wxkey::DbKeyError;
        let started = std::time::Instant::now();
        let remaining = || timeout_secs.saturating_sub(started.elapsed().as_secs());
        let timeout_error = || {
            AppError::native(format!(
                "timed out after {timeout_secs} s without getting the key; quit WeChat completely, reopen it and click \"Enter WeChat\" in the login window, then run the command again"
            ))
        };

        let pid = match pid_override {
            Some(pid) => pid,
            None if cfg!(windows) => {
                let found = wait_for_fresh_wechat(
                    &mut wechat_pids,
                    &mut || std::thread::sleep(std::time::Duration::from_secs(1)),
                    &mut || started.elapsed().as_secs(),
                    timeout_secs,
                    &mut |event| report_wait(event),
                );
                end_status_line();
                found.ok_or_else(timeout_error)?
            }
            None => find_wechat_pid().ok_or_else(|| {
                AppError::runtime("WeChat process not found (looked for Weixin.exe and WeChat.exe); start WeChat first or pass --pid")
            })?,
        };
        phase(
            json!({ "type": "key_phase", "phase": "login", "pid": pid }),
            if crate::locale::current() == crate::locale::Lang::Zh {
                format!("检测到微信（pid {pid}）请在登录窗口点击「进入微信」。")
            } else {
                format!("WeChat found (pid {pid}). Click \"Enter WeChat\" in the login window.")
            },
        );

        let key = loop {
            if remaining() == 0 {
                end_status_line();
                return Err(timeout_error());
            }
            // wx_key's own progress messages are noise; only its errors are shown
            let mut on_status = |msg: &str, level: i32| {
                if level == 2 {
                    end_status_line();
                    crate::output::event(
                        json!({ "type": "key_status", "level": level, "message": crate::locale::localize(msg.to_string()) }),
                    );
                }
            };
            let mut on_tick = |left: u64| status_line(&wait_text(Wait::Key, left));
            match wxkey.get_db_key_with_tick(
                pid,
                std::time::Duration::from_secs(remaining()),
                &mut on_status,
                &mut on_tick,
            ) {
                Ok(key) => break key,
                Err(DbKeyError::AccessDenied(detail)) => {
                    end_status_line();
                    return Err(AppError::native(format!(
                        "permission denied: cannot open the WeChat process (pid {pid}). Run the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator. ({detail})"
                    )));
                }
                Err(DbKeyError::Timeout | DbKeyError::LoginRequired) => {
                    end_status_line();
                    return Err(timeout_error());
                }
                // the process may not be ready to be hooked right after it started: try again
                Err(DbKeyError::Other(_)) if remaining() > 0 => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                Err(other) => {
                    end_status_line();
                    return Err(AppError::native(other.to_string()));
                }
            }
        };
        end_status_line();
        Ok(json!({ "decrypt_key": key, "method": "wx_key", "pid": pid }))
    }

    /// Explicit Rust file-based acquisition, independent of native helpers and stored image keys.
    pub fn key_image_rust(
        &self,
        user_dir: Option<&str>,
        kvcomm_dirs: &[PathBuf],
        scan_budget: usize,
    ) -> AppResult<Value> {
        if kvcomm_dirs.is_empty() || scan_budget == 0 {
            return Err(AppError::usage(
                "Rust image-key acquisition needs kvcomm directories and a nonzero scan budget",
            ));
        }
        let account = match user_dir {
            Some(dir) => crate::config::expand_home(dir),
            None => self.account_dir_only()?,
        };
        let wxid = self
            .wxid_override
            .as_deref()
            .or_else(|| self.profile().ok().and_then(|p| p.wxid.as_deref()));
        crate::image_keys::acquire_image_keys(kvcomm_dirs, &account, wxid, scan_budget)
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
                    "image_xor_key": xor_key,
                    "image_aes_key": profile.image_aes_key,
                    "method": "config",
                    "note": "from stored config; use key scan-image for live extraction"
                }));
            }
            return Err(AppError::native(
                "image key not available; configure image_xor_key or use the wx_key native library",
            ));
        }
        let raw = wxkey
            .get_image_key()
            .map_err(|err| AppError::native(err.to_string()))?;
        let parsed: Value = serde_json::from_str(&raw)
            .map_err(|_| AppError::native("failed to parse the image key data"))?;
        let accounts = parsed
            .get("accounts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let codes: Vec<u64> = accounts
            .first()
            .and_then(|a| a.get("keys"))
            .and_then(Value::as_array)
            .map(|k| {
                k.iter()
                    .filter_map(|k| k.get("code").and_then(Value::as_u64))
                    .collect()
            })
            .unwrap_or_default();
        if codes.is_empty() {
            return Err(AppError::native("no valid key code found (the kvcomm cache is empty); open a few images in WeChat first"));
        }
        let account_dir = self.account_dir_only().ok();
        let dir_text = user_dir.map(str::to_string).or_else(|| {
            account_dir
                .as_ref()
                .map(|d| d.to_string_lossy().to_string())
        });
        let wxid = self
            .wxid_override
            .clone()
            .or_else(|| self.profile().ok().and_then(|p| p.wxid.clone()));
        let candidates = crate::keys::collect_wxid_candidates(dir_text.as_deref(), wxid.as_deref());
        let template = dir_text
            .as_deref()
            .map(Path::new)
            .filter(|d| d.exists())
            .map(|d| crate::keys::find_template_data(d, 32));
        if let Some((Some(cipher), _)) = &template {
            for cand in &candidates {
                for code in &codes {
                    let (xor, aes) = crate::keys::derive_image_keys(*code, cand);
                    if crate::keys::verify_derived_aes_key(&aes, cipher) {
                        return Ok(
                            json!({ "image_xor_key": xor, "image_aes_key": aes, "verified": true, "wxid": cand, "code": code, "method": "wx_key" }),
                        );
                    }
                }
            }
            return Err(AppError::native("the cached codes do not match this account's wxid; check the configured wxid / the account directory, or use `key scan-image`"));
        }
        let fallback_wxid = candidates
            .first()
            .cloned()
            .or_else(|| {
                accounts
                    .first()
                    .and_then(|a| a.get("wxid").and_then(Value::as_str).map(str::to_string))
            })
            .unwrap_or_else(|| "unknown".into());
        let (xor, aes) = crate::keys::derive_image_keys(codes[0], &fallback_wxid);
        Ok(
            json!({ "image_xor_key": xor, "image_aes_key": aes, "verified": false, "wxid": fallback_wxid, "code": codes[0], "method": "wx_key" }),
        )
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
            let parsed: Value = serde_json::from_str(&result).unwrap_or(json!({ "raw": result }));
            Ok(json!({ "result": parsed, "method": "image_scan_helper", "userDir": user_dir }))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let wxkey = weflow_native::wxkey::WxKey::load(&self.ctx.runtime_dir)
                .map_err(|err| AppError::native(err.to_string()))?;
            if wxkey.is_available() {
                let result = wxkey
                    .get_image_key()
                    .map_err(|err| AppError::native(err.to_string()))?;
                Ok(json!({ "result": result, "method": "wx_key" }))
            } else {
                Err(AppError::native(
                    "image key scanning requires platform-specific native library",
                ))
            }
        }
    }

    /// Export a conversation's messages in one of the desktop app's formats
    /// (json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql).
    pub fn export_messages(&self, req: &MessageExportRequest, out: &Path) -> AppResult<Value> {
        let format = req.format.to_ascii_lowercase();
        if !MESSAGE_EXPORT_FORMATS
            .split(',')
            .any(|f| f.trim() == format)
        {
            return Err(AppError::usage(format!(
                "unsupported message export format: {format}; supported: {MESSAGE_EXPORT_FORMATS}"
            )));
        }
        again_if_keys_changed(|| self.export_streaming(req, out))
    }

    /// [`export_messages`](Self::export_messages) that also copies the selected media (images, voices, videos,
    /// stickers) to `<out dir>/media/<out name>/` and points the exported messages at the copies.
    pub async fn export_messages_with_media(
        &self,
        req: &MessageExportRequest,
        out: &Path,
        media: &crate::api::ApiMediaOptions,
    ) -> AppResult<Value> {
        if !media.enabled {
            return self.export_messages(req, out);
        }
        let (wcdb, mut collected) = again_if_keys_changed(|| self.export_collect(req))?;
        let stats = self
            .attach_export_media(&wcdb, &mut collected, &req.session_id, out, media)
            .await;
        let mut result = self.export_write(req, out, &wcdb, collected)?;
        result["media"] = stats;
        Ok(result)
    }

    fn export_collect(
        &self,
        req: &MessageExportRequest,
    ) -> AppResult<(weflow_native::wcdb::Wcdb, Vec<crate::message::ExportMsg>)> {
        use crate::message::CollectOptions;
        let (_, _, wxid) = self.connection_inputs()?;
        let my_wxid = clean_account_dir_name(&wxid.unwrap_or_default());
        check_sender(req.sender.as_deref(), &my_wxid)?;
        let wcdb = self.open_wcdb()?;
        // One pass over the conversation: keeping its whole database decrypted would only cost memory.
        wcdb.set_low_memory(true);
        let opts = CollectOptions {
            session_id: &req.session_id,
            my_wxid: &my_wxid,
            start: req.start,
            end: req.end,
            sender_filter: req.sender.as_deref(),
        };
        let mut collected = export_stream::collect_export_messages(&wcdb, req, &opts)?;
        collected.sort_by(|a, b| {
            a.create_time
                .cmp(&b.create_time)
                .then(a.local_id.cmp(&b.local_id))
        });
        if collected.is_empty() {
            return Err(no_messages(req));
        }
        Ok((wcdb, collected))
    }

    /// Export that writes the file while the messages are read, so memory stays flat whatever the size of the
    /// conversation. (An export that copies media reads everything first: see `export_collect`.)
    fn export_streaming(&self, req: &MessageExportRequest, out: &Path) -> AppResult<Value> {
        use crate::message::CollectOptions;
        let (_, _, wxid) = self.connection_inputs()?;
        let my_wxid = clean_account_dir_name(&wxid.unwrap_or_default());
        check_sender(req.sender.as_deref(), &my_wxid)?;
        let wcdb = self.open_wcdb()?;
        wcdb.set_low_memory(true);
        let opts = CollectOptions {
            session_id: &req.session_id,
            my_wxid: &my_wxid,
            start: req.start,
            end: req.end,
            sender_filter: req.sender.as_deref(),
        };
        let format = req.format.to_ascii_lowercase();
        let before = file_stamp(out);
        let render_ahead =
            crate::export_msg::EntryFormat::parse(&format).filter(|f| f.renders_ahead());
        let read = match render_ahead {
            // entries that do not depend on each other and are slow to render: the reading workers render them
            // too, this thread only writes them in order
            Some(entries) => {
                let parts = self.exporter_parts(
                    &req.session_id,
                    req.display_pref,
                    req.excel_compact,
                    &wcdb,
                )?;
                let render: &export_stream::WithFinisher<'_, crate::export_msg::RenderedEntries> =
                    &|body| {
                        let mut names = name_book(&wcdb);
                        let mut exporter = parts.exporter(&mut names);
                        body(&mut |msgs| exporter.render_entries(entries, &msgs));
                    };
                export_stream::read_export_pages(&wcdb, req, &opts, render, |pages| {
                    let mut names = name_book(&wcdb);
                    let result = parts.exporter(&mut names).write_rendered(
                        entries,
                        out,
                        &mut |_| pages.next(),
                        false,
                    );
                    Ok(result)
                })
            }
            None => export_stream::read_export_stream(&wcdb, req, &opts, |stream| {
                self.with_exporter(
                    &req.session_id,
                    req.display_pref,
                    req.excel_compact,
                    &wcdb,
                    |ex| ex.write_streamed(&format, out, stream),
                )
            }),
        };
        if !matches!(read, Ok(Ok(Ok(_)))) {
            // the reading or the writing (a full disk) failed after the file was written from the messages read so
            // far: do not leave a truncated export that looks complete
            remove_if_written(out, before);
        }
        let count = read??.map_err(|e| AppError::runtime(e.to_string()))?;
        if count == 0 {
            return Err(no_messages(req));
        }
        Ok(json!({ "out": out, "count": count, "session": req.session_id, "format": format }))
    }

    /// 按会话建好 [`Exporter`](crate::export_msg::Exporter)（联系人名、群昵称、自己的账号），再交给 `f` 使用。
    fn with_exporter<R>(
        &self,
        session_id: &str,
        display_pref: crate::export_msg::DisplayPref,
        excel_compact: bool,
        wcdb: &weflow_native::wcdb::Wcdb,
        f: impl FnOnce(&mut crate::export_msg::Exporter<'_, '_>) -> R,
    ) -> AppResult<R> {
        let parts = self.exporter_parts(session_id, display_pref, excel_compact, wcdb)?;
        let mut names = name_book(wcdb);
        Ok(f(&mut parts.exporter(&mut names)))
    }

    /// What every [`Exporter`](crate::export_msg::Exporter) of one export of `session_id` shares: read once, then
    /// each thread that renders builds its own exporter from it (see [`ExporterParts::exporter`]).
    fn exporter_parts(
        &self,
        session_id: &str,
        display_pref: crate::export_msg::DisplayPref,
        excel_compact: bool,
        wcdb: &weflow_native::wcdb::Wcdb,
    ) -> AppResult<ExporterParts> {
        use crate::export_msg::*;
        let (_, _, wxid) = self.connection_inputs()?;
        let raw_my_wxid = wxid.unwrap_or_default();
        let my_wxid = clean_account_dir_name(&raw_my_wxid);
        let mut names = name_book(wcdb);
        let is_group = session_id.ends_with("@chatroom");
        let (group_nicks, group_members) = if is_group {
            let nick_value = wcdb.group_nicknames(session_id).unwrap_or(Value::Null);
            let nick_obj = nick_value
                .get("nicknames")
                .and_then(Value::as_object)
                .or_else(|| nick_value.as_object());
            let entries: Vec<(String, String)> = nick_obj
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|n| (k.clone(), n.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            let members_value = wcdb.group_members(session_id).unwrap_or(Value::Null);
            (
                build_trusted_group_nicknames(entries),
                extract_member_ids(&members_value),
            )
        } else {
            (std::collections::HashMap::new(), Vec::new())
        };

        let session_display = names.display_name(session_id);
        let session_contact = names.get(session_id);
        let my_display = names.display_name(&my_wxid);
        Ok(ExporterParts {
            session: SessionInfo {
                id: session_id.to_string(),
                display_name: session_display,
                nickname: session_contact
                    .as_ref()
                    .map(|c| c.nickname.clone())
                    .unwrap_or_default(),
                remark: session_contact
                    .as_ref()
                    .map(|c| c.remark.clone())
                    .unwrap_or_default(),
                is_group,
            },
            my_wxid: my_wxid.clone(),
            raw_my_wxid,
            my_display: if my_display == my_wxid {
                String::new()
            } else {
                my_display
            },
            group_nicks,
            group_members,
            settings: Settings {
                display_pref,
                excel_compact,
                ..Default::default()
            },
        })
    }

    fn export_write(
        &self,
        req: &MessageExportRequest,
        out: &Path,
        wcdb: &weflow_native::wcdb::Wcdb,
        collected: Vec<crate::message::ExportMsg>,
    ) -> AppResult<Value> {
        let format = req.format.to_ascii_lowercase();
        let runtime = |e: anyhow::Error| AppError::runtime(e.to_string());
        let before = file_stamp(out);
        let written = self.with_exporter(
            &req.session_id,
            req.display_pref,
            req.excel_compact,
            wcdb,
            |exporter| match format.as_str() {
                "chatlab" => exporter
                    .write_chatlab(&collected, out, false)
                    .map_err(runtime),
                "chatlab-jsonl" => exporter
                    .write_chatlab(&collected, out, true)
                    .map_err(runtime),
                "json" => exporter.write_json(&collected, out, false).map_err(runtime),
                "arkme-json" => exporter.write_json(&collected, out, true).map_err(runtime),
                "excel" | "xlsx" => exporter.write_excel(&collected, out).map_err(runtime),
                "txt" => exporter.write_txt(&collected, out).map_err(runtime),
                "weclone" => exporter.write_weclone(&collected, out).map_err(runtime),
                "html" => exporter.write_html(&collected, out).map_err(runtime),
                "sql" => exporter.write_sql(&collected, out).map_err(runtime),
                other => Err(AppError::usage(format!(
                    "unsupported message export format: {other}; supported: {MESSAGE_EXPORT_FORMATS}"
                ))),
            },
        );
        if !matches!(written, Ok(Ok(_))) {
            // a write that failed part-way (a full disk) leaves no truncated export behind
            remove_if_written(out, before);
        }
        written??;
        Ok(
            json!({ "out": out, "count": collected.len(), "session": req.session_id, "format": format }),
        )
    }

    // ── Media export ─────────────────────────────────────────────────────────

    pub async fn emoji_download(&self, session_id: &str, out: &Path) -> AppResult<Value> {
        let messages = self.messages(session_id, 500, 0)?;
        let metas = crate::media::extract_emoji_urls(&messages);
        let total = metas.len();
        if total == 0 {
            return Ok(json!({ "found": 0, "downloaded": 0 }));
        }
        std::fs::create_dir_all(out)
            .map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;
        let hub = self.clone();
        let results = crate::media::download_emojis(&metas, out, &|current, t| {
            hub.emit_progress("emoji", "downloading emojis", current, t)
        })
        .await
        .map_err(|e| AppError::runtime(e.to_string()))?;
        let downloaded = results
            .iter()
            .filter(|v| v["cached"].as_bool() != Some(true))
            .count();
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
            &self.ctx.version,
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
        let manifest =
            crate::backup::inspect_backup(path).map_err(|e| AppError::runtime(e.to_string()))?;
        Ok(serde_json::to_value(manifest).unwrap_or(Value::Null))
    }

    pub fn backup_restore(&self, path: &Path, target_dir: &Path) -> AppResult<Value> {
        let hub = self.clone();
        crate::backup::restore_backup(path, target_dir, &|current, total| {
            hub.emit_progress("restore", "restoring backup", current, total)
        })
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
        wcdb.open_unchecked(&account_dir, &key, wxid.as_deref())
            .map_err(|err| AppError::native(err.to_string()))?;
        // Proving the key costs a slow key derivation on session.db. Once it has worked, a fingerprint of
        // (key, session.db salt, account) is remembered, and later runs with the same key and database skip it.
        let fingerprint = wcdb
            .session_salt()
            .map(|salt| key_fingerprint(&account_dir, &key, &salt));
        let path = self.ctx.cache_dir().join("verified-keys");
        let known = fingerprint.as_ref().is_some_and(|fp| {
            std::fs::read_to_string(&path).is_ok_and(|s| s.lines().any(|l| l == fp))
        });
        if !known {
            wcdb.check_key()
                .map_err(|err| AppError::native(err.to_string()))?;
            if let Some(fp) = fingerprint {
                remember_fingerprint(&path, &fp);
            }
        }
        Ok(wcdb)
    }

    fn connection_inputs(&self) -> AppResult<(PathBuf, String, Option<String>)> {
        let profile = self.profile()?;
        let db_path = self
            .db_path_override
            .clone()
            .or_else(|| profile.db_path.clone())
            .ok_or_else(|| AppError::config("missing db_path; run config set db_path"))?;
        let key = self
            .decrypt_key_override
            .clone()
            .or_else(|| profile.decrypt_key.clone())
            .ok_or_else(|| AppError::config("missing decrypt_key; run config set decrypt_key"))?;
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

/// Account folders of a WeChat data directory (or the folder itself when it is one), with their wxid.
fn find_accounts(root: &Path) -> Vec<(String, PathBuf)> {
    let wxid_of = |path: &Path| {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string());
        clean_account_dir_name(&name.unwrap_or_default())
    };
    if crate::config::is_account_dir(root) {
        return vec![(wxid_of(root), root.to_path_buf())];
    }
    let mut accounts: Vec<(String, PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && crate::config::is_account_dir(p))
        .map(|p| (wxid_of(&p), p))
        .collect();
    accounts.sort();
    accounts
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
            return stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse().ok());
        }
        let output2 = std::process::Command::new("pgrep")
            .arg("-x")
            .arg("微信")
            .output()
            .ok()?;
        if output2.status.success() {
            let stdout = String::from_utf8_lossy(&output2.stdout);
            return stdout
                .lines()
                .next()
                .and_then(|line| line.trim().parse().ok());
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
        stdout
            .lines()
            .next()
            .and_then(|line| line.trim().parse().ok())
    }
    #[cfg(target_os = "windows")]
    {
        wechat_pids().first().copied()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wait {
    Quit,
    Open,
    Key,
}

enum WaitEvent {
    AskQuit(Vec<u32>),
    AskOpen { was_running: bool },
    Tick { phase: Wait, remaining: u64 },
}

/// The remaining-time line shown while waiting.
fn wait_text(phase: Wait, remaining: u64) -> String {
    let zh = crate::locale::current() == crate::locale::Lang::Zh;
    let what = match (phase, zh) {
        (Wait::Quit, true) => "等待微信退出…",
        (Wait::Quit, false) => "Waiting for WeChat to quit...",
        (Wait::Open, true) => "等待微信启动…",
        (Wait::Open, false) => "Waiting for WeChat to start...",
        (Wait::Key, true) => "等待密钥…",
        (Wait::Key, false) => "Waiting for the key...",
    };
    if zh {
        format!("{what}（剩余 {remaining} 秒自动退出）")
    } else {
        format!("{what} (exits automatically in {remaining} s)")
    }
}

/// One status message: JSON event with `--json`, a plain line otherwise.
fn phase(json_event: Value, text: String) {
    crate::output::end_status_line();
    if crate::output::json_output() {
        crate::output::event(json_event);
    } else {
        eprintln!("{text}");
    }
}

fn report_wait(event: WaitEvent) {
    let zh = crate::locale::current() == crate::locale::Lang::Zh;
    match event {
        WaitEvent::AskQuit(pids) => {
            let list = pids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            phase(
                json!({ "type": "key_phase", "phase": "quit_wechat", "pids": pids }),
                if zh {
                    format!("检测到微信正在运行（pid {list}）请先完全退出微信：\n右下角托盘图标 → 右键 → 退出微信")
                } else {
                    format!("WeChat is running (pid {list}). Quit it completely first:\nsystem tray icon -> right click -> Quit WeChat")
                },
            );
        }
        WaitEvent::AskOpen { was_running } => phase(
            json!({ "type": "key_phase", "phase": "open_wechat", "wasRunning": was_running }),
            match (was_running, zh) {
                (true, true) => "微信已退出，请重新打开微信。".to_string(),
                (true, false) => "WeChat has quit. Open it again.".to_string(),
                (false, true) => "未检测到微信，请打开微信。".to_string(),
                (false, false) => "WeChat is not running. Open it.".to_string(),
            },
        ),
        WaitEvent::Tick { phase, remaining } => {
            crate::output::status_line(&wait_text(phase, remaining))
        }
    }
}

/// Waits until WeChat has been quit (when it is running now) and started again; returns the new process id, or
/// `None` once `timeout_secs` have passed. Time and process list are passed in so the logic can be tested.
fn wait_for_fresh_wechat(
    list: &mut dyn FnMut() -> Vec<u32>,
    sleep: &mut dyn FnMut(),
    elapsed: &mut dyn FnMut() -> u64,
    timeout_secs: u64,
    report: &mut dyn FnMut(WaitEvent),
) -> Option<u32> {
    let running = list();
    let mut phase = if running.is_empty() {
        report(WaitEvent::AskOpen { was_running: false });
        Wait::Open
    } else {
        report(WaitEvent::AskQuit(running));
        Wait::Quit
    };
    loop {
        let used = elapsed();
        if used >= timeout_secs {
            return None;
        }
        let pids = list();
        match phase {
            Wait::Quit if pids.is_empty() => {
                phase = Wait::Open;
                report(WaitEvent::AskOpen { was_running: true });
                continue;
            }
            Wait::Open if !pids.is_empty() => return pids.first().copied(),
            _ => {}
        }
        report(WaitEvent::Tick {
            phase,
            remaining: timeout_secs - used,
        });
        sleep();
    }
}

/// Process ids of the running WeChat (Weixin.exe / WeChat.exe on Windows), in the order the system lists them.
fn wechat_pids() -> Vec<u32> {
    #[cfg(target_os = "windows")]
    {
        let mut pids = Vec::new();
        for image in ["Weixin.exe", "WeChat.exe"] {
            let Ok(output) = std::process::Command::new("tasklist")
                .args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"])
                .output()
            else {
                continue;
            };
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("INFO:"))
            {
                let parts: Vec<&str> = line.split("\",\"").map(|p| p.trim_matches('"')).collect();
                if parts.first().is_some_and(|n| n.eq_ignore_ascii_case(image)) {
                    if let Some(pid) = parts.get(1).and_then(|p| p.parse::<u32>().ok()) {
                        pids.push(pid);
                    }
                }
            }
        }
        pids
    }
    #[cfg(not(target_os = "windows"))]
    {
        find_wechat_pid().into_iter().collect()
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
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

pub const MESSAGE_EXPORT_FORMATS: &str =
    "json, arkme-json, chatlab, chatlab-jsonl, excel, txt, weclone, html, sql";

pub struct MessageExportRequest {
    pub session_id: String,
    pub format: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub sender: Option<String>,
    pub display_pref: crate::export_msg::DisplayPref,
    pub excel_compact: bool,
}

/// The parts of an [`Exporter`](crate::export_msg::Exporter) that do not change during an export (all but its name
/// cache), so threads that render the same export each build one.
struct ExporterParts {
    session: crate::export_msg::SessionInfo,
    my_wxid: String,
    raw_my_wxid: String,
    my_display: String,
    group_nicks: std::collections::HashMap<String, String>,
    group_members: Vec<String>,
    /// Holds the export time: every exporter of one export reports the same.
    settings: crate::export_msg::Settings,
}

impl ExporterParts {
    fn exporter<'a, 'n>(
        &self,
        names: &'a mut crate::export_msg::NameBook<'n>,
    ) -> crate::export_msg::Exporter<'a, 'n> {
        crate::export_msg::Exporter {
            session: self.session.clone(),
            my_wxid: self.my_wxid.clone(),
            raw_my_wxid: self.raw_my_wxid.clone(),
            my_display: self.my_display.clone(),
            group_nicks: self.group_nicks.clone(),
            group_members: self.group_members.clone(),
            names,
            settings: self.settings.clone(),
        }
    }
}

/// What tells whether an export wrote to `path`: its size and modification time, `None` when it does not exist.
type FileStamp = Option<(u64, Option<std::time::SystemTime>)>;

fn file_stamp(path: &Path) -> FileStamp {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// Removes `out` after a failed export if the export wrote to it (compared with `before`, its stamp from before the
/// export), so a truncated file is not taken for a complete export.
fn remove_if_written(out: &Path, before: FileStamp) {
    if file_stamp(out) != before {
        let _ = std::fs::remove_file(out);
    }
}

/// A temporary name next to `path`, unique in this process.
fn temporary_sibling(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}-{n}.tmp", std::process::id()));
    PathBuf::from(name)
}

/// Writes `bytes` to `path` through a temporary file renamed into place. Another thread never sees the file half
/// written (an export's media threads check for an image another one is still writing, issue #52), and two that
/// write it at once both leave a complete file.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// [`write_atomically`] for a copy of `from`.
fn copy_atomically(from: &Path, path: &Path) -> std::io::Result<()> {
    let tmp = temporary_sibling(path);
    let copied = std::fs::copy(from, &tmp).and_then(|_| std::fs::rename(&tmp, path));
    if copied.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    copied
}

/// Runs an export once more when the order of its messages changed while it was read: the second run reads the keys
/// of the tables that broke the `sort_seq` rule from their rows (see `NativeAccount::open_export_cursor`). An export
/// that failed removes what it wrote, so starting over is safe.
fn again_if_keys_changed<T>(mut export: impl FnMut() -> AppResult<T>) -> AppResult<T> {
    match export() {
        Err(e) if e.message.contains(weflow_native::native_msg::KEYS_CHANGED) => export(),
        done => done,
    }
}

/// `--sender` takes the bare wxid: it is the one identifier every message has, unique and fixed. The account folder
/// name (the owner's wxid with a `_xxxx` suffix) would match too, by the identity rule, so it is refused with the
/// bare form to use instead.
fn check_sender(sender: Option<&str>, my_wxid: &str) -> AppResult<()> {
    let Some(sender) = sender.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if sender.len() > my_wxid.len() && crate::message::is_same_wxid(sender, my_wxid) {
        return Err(AppError::usage(format!(
            "--sender takes the bare wxid: use {my_wxid}, not the account folder name {sender}"
        )));
    }
    Ok(())
}

/// The error of an export that found nothing; with `--sender`, a reminder of what it takes.
fn no_messages(req: &MessageExportRequest) -> AppError {
    match req.sender.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(sender) => AppError::runtime(format!(
            "no messages from {sender} in this session in the given range (--sender takes the bare wxid, as `chat contacts` lists it)"
        )),
        None => AppError::runtime("no messages found for this session in the given range"),
    }
}

/// Contact names looked up in `wcdb` as an export needs them (cached).
fn name_book(wcdb: &weflow_native::wcdb::Wcdb) -> crate::export_msg::NameBook<'_> {
    use crate::export_msg::{ContactInfo, NameBook};
    NameBook::new(move |username: &str| {
        wcdb.contact(username)
            .ok()
            .filter(|v| v.is_object() && !v.as_object().is_none_or(|o| o.is_empty()))
            .map(|v| ContactInfo::from_value(username, &v))
    })
}

/// A sticker: a plain emoji message, or an appmsg of type 8 (a sticker sent as an attachment).
fn is_sticker(m: &crate::message::ExportMsg) -> bool {
    m.local_type == 47 || crate::message::appmsg_emoticon_md5(&m.content).is_some()
}

/// 查表情描述（用户自己的表情文字，否则商店表情的中文描述），写进 `emoji_caption`，
/// 导出时显示为 `[表情：描述]`。描述表（md5 → 描述）整张一次读出，见 `emoticon_captions`。
fn attach_emoji_captions(
    table: &std::collections::HashMap<String, String>,
    messages: &mut [crate::message::ExportMsg],
) {
    for m in messages.iter_mut() {
        let md5 = match m.local_type {
            47 => m.emoji_md5.clone(),
            _ => crate::message::appmsg_emoticon_md5(&m.content),
        };
        let Some(md5) = md5 else { continue };
        m.emoji_caption = table
            .get(&md5.to_lowercase())
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
    }
}

/// `cleanAccountDirName`: `wxid_abc_1234` -> `wxid_abc`, `name_ab12` -> `name`.
/// One-way fingerprint of a key that decrypted an account's `session.db` (the key cannot be recovered from it).
fn key_fingerprint(account_dir: &Path, key: &str, salt: &[u8; 16]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"weflow verified key v1\0");
    h.update(key.trim().to_ascii_lowercase().as_bytes());
    h.update([0]);
    h.update(salt);
    h.update([0]);
    h.update(account_dir.to_string_lossy().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Add a fingerprint to the list of verified keys (a few recent ones are kept). Failing to write only means the
/// key is checked again next time.
fn remember_fingerprint(path: &Path, fingerprint: &str) {
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    lines.retain(|l| l != fingerprint && l.len() == 64);
    lines.push(fingerprint.to_string());
    let keep = &lines[lines.len().saturating_sub(16)..];
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, keep.join("\n") + "\n");
}

pub fn clean_account_dir_name(dir_name: &str) -> String {
    use crate::message::rx;
    let trimmed = dir_name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.to_lowercase().starts_with("wxid_") {
        return rx(r"(?i)^(wxid_[^_]+)")
            .captures(trimmed)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| trimmed.to_string());
    }
    rx(r"^(.+)_([a-zA-Z0-9]{4})$")
        .captures(trimmed)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| trimmed.to_string())
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

/// `normalizeTimestamp` of the WCDB wrapper: ms → s, clamped to i32.
pub(crate) fn normalize_timestamp(input: i64) -> i32 {
    if input <= 0 {
        return 0;
    }
    let seconds = if input > 1_000_000_000_000 {
        input / 1000
    } else {
        input
    };
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

#[cfg(test)]
mod export_tests {
    #[test]
    fn atomic_writes_replace_the_file_and_leave_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("weflow-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jpg");
        super::write_atomically(&path, b"first").unwrap();
        super::write_atomically(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let src = dir.join("src.bin");
        std::fs::write(&src, b"copied").unwrap();
        let copy = dir.join("b.jpg");
        super::copy_atomically(&src, &copy).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"copied");
        // a write that fails (no such folder) leaves nothing behind either
        assert!(super::write_atomically(&dir.join("missing").join("c.jpg"), b"x").is_err());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.jpg", "b.jpg", "src.bin"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_account_folders_and_strips_the_suffix() {
        let root = std::env::temp_dir().join(format!("weflow-wxid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("wxid_abc123_ab12/db_storage")).unwrap();
        std::fs::create_dir_all(root.join("someone_a1b2/db_storage")).unwrap();
        std::fs::create_dir_all(root.join("all_users")).unwrap();
        let found: Vec<String> = find_accounts(&root).into_iter().map(|a| a.0).collect();
        assert_eq!(found, ["someone", "wxid_abc123"]);
        // the account folder itself works too
        let one = find_accounts(&root.join("wxid_abc123_ab12"));
        assert_eq!(one[0].0, "wxid_abc123");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Drives `wait_for_fresh_wechat` with a scripted process list; each `sleep` is one second.
    fn run_wait(script: Vec<Vec<u32>>, timeout: u64) -> (Option<u32>, Vec<&'static str>, u64) {
        use std::cell::{Cell, RefCell};
        let calls = Cell::new(0usize);
        let seconds = Cell::new(0u64);
        let events = RefCell::new(Vec::new());
        let found = wait_for_fresh_wechat(
            &mut || {
                let i = calls.get().min(script.len() - 1);
                calls.set(calls.get() + 1);
                script[i].clone()
            },
            &mut || seconds.set(seconds.get() + 1),
            &mut || seconds.get(),
            timeout,
            &mut |event| {
                events.borrow_mut().push(match event {
                    WaitEvent::AskQuit(_) => "ask_quit",
                    WaitEvent::AskOpen { was_running: true } => "ask_open_after_quit",
                    WaitEvent::AskOpen { .. } => "ask_open",
                    WaitEvent::Tick {
                        phase: Wait::Quit, ..
                    } => "tick_quit",
                    WaitEvent::Tick {
                        phase: Wait::Open, ..
                    } => "tick_open",
                    WaitEvent::Tick { .. } => "tick",
                });
            },
        );
        (found, events.into_inner(), seconds.get())
    }

    #[test]
    fn waits_for_quit_then_a_new_start() {
        // running at first, gone after the third check, a new process after two more
        let (found, events, _) = run_wait(
            vec![vec![10], vec![10], vec![10], vec![], vec![], vec![77]],
            180,
        );
        assert_eq!(found, Some(77));
        assert_eq!(
            events,
            [
                "ask_quit",
                "tick_quit",
                "tick_quit",
                "ask_open_after_quit",
                "tick_open"
            ]
        );
    }

    #[test]
    fn skips_the_quit_step_when_wechat_is_not_running() {
        let (found, events, _) = run_wait(vec![vec![], vec![], vec![5]], 180);
        assert_eq!(found, Some(5));
        assert_eq!(events, ["ask_open", "tick_open"]);
    }

    #[test]
    fn gives_up_after_the_timeout() {
        // WeChat never quits
        let (found, events, seconds) = run_wait(vec![vec![10]], 5);
        assert_eq!(found, None);
        assert_eq!(seconds, 5);
        assert_eq!(events.iter().filter(|e| **e == "tick_quit").count(), 5);
    }

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
