---
id: wechat-legacy-chat-voice-decrypt
title: 聊天语音解密机制
subtitle: 从 Msg_* 语音消息到 VoiceInfo 原始语音的跨库定位链路
articleRole: sub
parentId: 02-核心解析机制
link: '#'
---
# 聊天语音解密机制

微信聊天中的语音消息并不是“在消息行里直接带完整音频”。消息表负责记录“这是一条语音、它属于谁、发生在什么时候”，真正的语音二进制通常存放在媒体库。  
因此，语音解密/还原本质是一个**跨库定位过程**。

## 1. 语音为什么要跨库定位

在微信本地数据库中可以把职责分为两层：

1. `message_*.db / Msg_*`：消息索引层，保存消息类型、时间、会话、发送者等“索引信息”
2. `media_*.db / VoiceInfo*`：媒体正文层，保存语音二进制与媒体侧索引键

这意味着要拿到某一条语音的真实数据，必须先从消息层提取定位键，再去媒体层命中记录。

## 2. 从消息表提取哪些字段

对于 `Msg_*` 中的语音消息，关键字段通常是：

1. `local_type = 34`：确认它是语音消息
2. `local_id`：会话内消息主键
3. `server_id`：服务端消息 ID（命中能力最强）
4. `create_time`：时间键（用于时间窗匹配）
5. `real_sender_id`（或可还原出的发送者用户名）：用于群聊场景区分具体成员

其中 `local_type` 是筛选条件，`local_id/server_id/create_time/发送者` 才是后续去媒体库定位的核心输入。

## 3. 从消息键映射到语音键

消息侧与语音侧的常见映射关系如下：

| 消息侧（Msg_*） | 语音侧（VoiceInfo*） | 作用 |
|---|---|---|
| `local_type = 34` | 无直接对应 | 先筛出语音消息 |
| `local_id` | `local_id` 或 `msg_local_id` | 会话内定位键 |
| `server_id` | `svr_id` 或 `msg_svr_id` | 强一致定位键 |
| `create_time` | `create_time/time` | 时间窗定位键 |
| `real_sender_id` / `sender_username` | `chat_name_id`（经 Name2Id 映射） | 群聊归属定位键 |

重点在最后一行：  
`chat_name_id` 通常不是明文用户名，而是 `Name2Id` 表里的 `rowid`。  
因此在群聊里，经常要先“用户名 -> rowid”，再与语音表关联。

## 4. 微信侧常见定位顺序

为了减少误命中，一般按这个顺序命中语音记录：

1. `svr_id` 精确命中
2. `chat_name_id + create_time`（可叠加 `local_id`）
3. `local_id + 附加保护条件`
4. 仅 `create_time` 时间窗兜底

时间匹配常见会用 `create_time ± 5 秒`，用于容忍客户端写入与同步过程中的细小时间偏差。


## 5. 还原到可播放音频

命中 `VoiceInfo` 后拿到的是语音二进制（Silk 编码），仍需做音频层还原：

1. 提取语音 BLOB
2. 执行 Silk 解码得到 PCM
3. 封装为 WAV 等通用格式，供播放或转写


