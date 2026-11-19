//! The fake's models: each answers in its own way, so that a client can be tried against every case the documents
//! describe by the model it names. The same models answer in both formats.

/// What a model does with a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behaviour {
    /// Answers the question.
    Answer,
    /// Reasons first ([`REASONING_TOKENS`] tokens, counted against the limit), then answers; refuses OpenAI's
    /// deprecated `max_tokens` and a `temperature` other than 1, as reasoning models do.
    Reason,
    /// Answers `SKIP`, as an insight model does when it has nothing to say.
    Skip,
    /// Declines: `refusal` in OpenAI's message, `stop_reason: "refusal"` in Anthropic's.
    Refuse,
    /// 429 with `retry-after: 0`, every time.
    RateLimit,
    /// 429 for a used-up quota: `insufficient_quota` in OpenAI's error, no `retry-after` in Anthropic's.
    Quota,
    /// 503 (OpenAI) or 529 (Anthropic) with `retry-after: 0`, every time.
    Overloaded,
    /// Overloaded on every other request, from the first: one retry gets the answer.
    Flaky,
    /// 500 with a body that is not JSON.
    ServerError,
    /// Answers after [`SLOW_ANSWER`].
    Slow,
}

/// A model of the fake.
#[derive(Clone, Copy, Debug)]
pub struct FakeModelInfo {
    /// The model's name, which picks its behaviour.
    pub id: &'static str,
    /// The name `GET /v1/models` shows.
    pub display_name: &'static str,
    /// How it answers.
    pub behaviour: Behaviour,
}

/// The fake's models, in the order `GET /v1/models` lists them.
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

/// The tokens [`Behaviour::Reason`] spends before it answers.
pub const REASONING_TOKENS: u64 = 512;

/// How long [`Behaviour::Slow`] takes.
pub const SLOW_ANSWER: std::time::Duration = std::time::Duration::from_secs(2);

pub fn model(id: &str) -> Option<&'static FakeModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// The tokens of `text`, as the fake counts them: one a character.
pub fn tokens(text: &str) -> u64 {
    u64::try_from(text.chars().count()).unwrap_or(u64::MAX)
}

/// The answer of a model that behaves as `behaviour` to `user` (with the instructions `system`), the `n`-th the fake
/// gives: `SKIP`, the words a connection test asks for, a footprint review, or one of a few insights in the language
/// of the question.
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
