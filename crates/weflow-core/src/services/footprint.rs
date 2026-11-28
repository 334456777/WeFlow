//! 足迹：一段日期内账号所有者和谁聊过、回复过谁、在哪里被 @。

use serde_json::{json, Value};

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// `begin` 到 `end` 之间账号所有者和谁聊过、回复过谁、在哪里被 @。
    ///
    /// `begin`、`end` 是 Unix 秒，两端都包含，为 0 时不限。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn footprint(&self, begin: i64, end: i64) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let profile = self.profile()?;
        let options = json!({
            "myWxid": profile.wxid.as_deref().unwrap_or_default(),
            "beginTimestamp": begin,
            "endTimestamp": end
        });
        wcdb.footprint_stats(&options)
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// 把 [`Self::footprint`] 的结果写成 CSV 或 Excel 导出的行，带上会话的名字和 @ 账号所有者的人的名字（#315）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn footprint_timeline(
        &self,
        data: &Value,
    ) -> AppResult<(Vec<&'static str>, Vec<Vec<Value>>)> {
        let wcdb = self.open_wcdb()?;
        let mut ids: Vec<String> = Vec::new();
        for (list, keys) in [
            ("private_segments", &["session_id"][..]),
            ("mentions", &["session_id", "sender_username"][..]),
        ] {
            for item in data[list].as_array().into_iter().flatten() {
                for key in keys {
                    if let Some(id) = item[*key]
                        .as_str()
                        .filter(|id| !id.is_empty() && !ids.iter().any(|i| i == id))
                    {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        let (names, _) = self.names_and_avatars(&wcdb, &ids);
        Ok(crate::export::footprint_timeline(data, &names))
    }

    /// `insight footprint`：与 [`Self::footprint`] 相同的统计，读成契约类型。
    ///
    /// # Errors
    ///
    /// 同 [`Self::footprint`]；数据库层的回答不符合契约类型时返回运行时错误。
    pub fn insight_footprint(&self, begin: i64, end: i64) -> AppResult<crate::contract::Footprint> {
        crate::contract::Footprint::of(self.footprint(begin, end)?)
    }
}
