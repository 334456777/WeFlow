//! 假服务的模型：每个模型有自己的回答方式，客户端按名字选模型，就能试遍接口文档描述的每种情况。两种格式共用同一组
//! 模型。

/// 模型对请求的回答方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behaviour {
    /// 回答问题。
    Answer,
    /// 先推理（[`REASONING_TOKENS`] 个 token，计入上限）再回答；与推理模型一样，拒绝 OpenAI 已弃用的 `max_tokens`
    /// 和不等于 1 的 `temperature`。
    Reason,
    /// 回答 `SKIP`，即见解模型没有可说的内容时的回答。
    Skip,
    /// 拒绝回答：OpenAI 格式在消息里给出 `refusal`，Anthropic 格式给出 `stop_reason: "refusal"`。
    Refuse,
    /// 每次都返回 429 和 `retry-after: 0`。
    RateLimit,
    /// 额度用完的 429：OpenAI 格式的错误带 `insufficient_quota`，Anthropic 格式不带 `retry-after`。
    Quota,
    /// 每次都返回 503（OpenAI）或 529（Anthropic）和 `retry-after: 0`。
    Overloaded,
    /// 从第一个请求起每隔一个请求过载一次：重试一次就能得到回答。
    Flaky,
    /// 返回 500，响应体不是 JSON。
    ServerError,
    /// 等待 [`SLOW_ANSWER`] 后再回答。
    Slow,
}

/// 假服务的一个模型。
#[derive(Clone, Copy, Debug)]
pub struct FakeModelInfo {
    /// 模型名，决定回答方式。
    pub id: &'static str,
    /// `GET /v1/models` 显示的名称。
    pub display_name: &'static str,
    /// 回答方式。
    pub behaviour: Behaviour,
}

/// 假服务的全部模型，顺序与 `GET /v1/models` 列出的一致。
pub const MODELS: &[FakeModelInfo] = &[
    FakeModelInfo {
        id: "fake-chat",
        display_name: "Fake Chat",
        behaviour: Behaviour::Answer,
    },
    FakeModelInfo {
        id: "fake-claude",
        display_name: "Fake Claude",
        behaviour: Behaviour::Answer,
    },
    FakeModelInfo {
        id: "fake-reasoner",
        display_name: "Fake Reasoner",
        behaviour: Behaviour::Reason,
    },
    FakeModelInfo {
        id: "fake-slow",
        display_name: "Fake Slow",
        behaviour: Behaviour::Slow,
    },
    FakeModelInfo {
        id: "fake-skip",
        display_name: "Fake Skip",
        behaviour: Behaviour::Skip,
    },
    FakeModelInfo {
        id: "fake-flaky",
        display_name: "Fake Flaky",
        behaviour: Behaviour::Flaky,
    },
    FakeModelInfo {
        id: "fake-refusal",
        display_name: "Fake Refusal",
        behaviour: Behaviour::Refuse,
    },
    FakeModelInfo {
        id: "fake-rate-limit",
        display_name: "Fake Rate Limit",
        behaviour: Behaviour::RateLimit,
    },
    FakeModelInfo {
        id: "fake-quota",
        display_name: "Fake Quota",
        behaviour: Behaviour::Quota,
    },
    FakeModelInfo {
        id: "fake-overloaded",
        display_name: "Fake Overloaded",
        behaviour: Behaviour::Overloaded,
    },
    FakeModelInfo {
        id: "fake-server-error",
        display_name: "Fake Server Error",
        behaviour: Behaviour::ServerError,
    },
];

/// [`Behaviour::Reason`] 在回答前花费的 token 数。
pub const REASONING_TOKENS: u64 = 512;

/// [`Behaviour::Slow`] 回答前等待的时间。
pub const SLOW_ANSWER: std::time::Duration = std::time::Duration::from_secs(2);

/// 名为 `id` 的模型；没有这个模型时为 `None`。
pub fn model(id: &str) -> Option<&'static FakeModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// 假服务计算的 `text` 的 token 数：每个字符算一个。
pub fn tokens(text: &str) -> u64 {
    u64::try_from(text.chars().count()).unwrap_or(u64::MAX)
}

/// 回答方式为 `behaviour` 的模型对 `user`（系统指令为 `system`）的回答，是假服务给出的第 `n` 个回答：`SKIP`、
/// 连接测试要求回复的文字、一段足迹的总结，或者用提问的语言写的几条见解之一。
pub fn answer(behaviour: Behaviour, system: &str, user: &str, n: u64) -> String {
    if behaviour == Behaviour::Skip {
        return "SKIP".into();
    }
    if user.contains("连接成功") {
        return "连接成功".into();
    }
    if user.to_ascii_lowercase().contains("connection successful") {
        return "connection successful".into();
    }
    let chinese = user.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    let footprint = system.contains("足迹") || system.to_ascii_lowercase().contains("footprint");
    let pick = |list: &[&str]| list[usize::try_from(n).unwrap_or(0) % list.len()].to_string();
    match (footprint, chinese) {
        (true, true) => "这段时间你回复了大部分私聊，节奏稳定。建议给几位久未回复的朋友补上一句问候。".into(),
        (true, false) => "You answered most private chats in this range at a steady pace. Send a short note to the friends you have not answered yet.".into(),
        (false, true) => pick(&[
            "最近的话题多在周末安排上，可以主动问问对方这周过得怎么样。",
            "对方回复变慢了，或许正忙，留一句不需要马上回复的关心就好。",
            "你们上次聊到想去的展览，可以顺势约个具体时间。",
        ]),
        (false, false) => pick(&[
            "Most of your recent talk was about weekend plans; ask how their week went.",
            "Their replies have slowed down; leave a note that needs no quick answer.",
            "You mentioned an exhibition you both wanted to see; suggest a date.",
        ]),
    }
}
