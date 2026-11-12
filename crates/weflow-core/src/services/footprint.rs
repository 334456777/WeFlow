//! The footprint: who the account talked with and replied to over a range of days.

use serde_json::{json, Value};

use super::ServiceHub;
use crate::error::{AppError, AppResult};

impl ServiceHub {
    /// Who I talked with and replied to, and where I was @-mentioned, between `begin` and `end` (Unix seconds,
    /// both inclusive; 0 for no bound).
    pub fn footprint(&self, begin: i64, end: i64) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let profile = self.profile()?;
        let options = json!({
            "myWxid": profile.wxid.as_deref().unwrap_or_default(),
            "beginTimestamp": begin,
            "endTimestamp": end
        });
        wcdb.footprint_stats(&options)
            .map_err(|err| AppError::native(err.to_string()))
    }

    /// [`Self::footprint`]'s answer as the rows of a CSV or Excel export, with the names of the conversations and
    /// of who @-mentioned me (#315).
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
        let (names, _) = self.names_and_avatars_pub(&wcdb, &ids);
        Ok(crate::export::footprint_timeline(data, &names))
    }

    /// `insight footprint`: the same statistics as [`Self::footprint`].
    pub fn insight_footprint(&self, begin: i64, end: i64) -> AppResult<crate::contract::Footprint> {
        crate::contract::Footprint::of(self.footprint(begin, end)?)
    }
}
