---
id: wechat-db-media
title: media_*.db 字段详解
subtitle: 语音数据字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# media_*.db 字段详解

## 数据库作用

管理语音消息、媒体分片及增量同步游标，是富媒体通信的底层支撑组件。

## 表与字段

### `VoiceInfo`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| chat_name_id | INTEGER 唯一索引 普通索引 | 会话 rowid 关联 `Name2Id.rowid` | 命中 `Name2Id.rowid` => 语音记录归属会话 |
| create_time | INTEGER 唯一索引 | 语音创建时间 用于时间窗定位语音数据 | 记录事件发生的 Unix 时间戳数值（秒级） => 语音消息时间主键之一；查询时常用 `create_time ± 5` 秒窗口 => 兼容时间抖动匹配 |
| local_id | INTEGER 唯一索引 | 对应消息 local_id | `>0` 常见 => 消息 local_id 关联键 |
| svr_id | INTEGER 普通索引 | 服务端语音 ID 优先用于精准命中语音数据 | `>0` => 服务端语音 ID 最强匹配键 |
| voice_data | BLOB | 语音二进制正文 | 非空二进制 => 语音正文数据 |
| data_index | TEXT 默认 '0' 唯一索引 | 同一条语音的分片序号 用于多分片合并 | `'0'` => 单分片语音或首分片；其他字符串序号 => 后续语音分片序号 |


### `Name2Id`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| user_name | TEXT 主键 唯一索引 | 用户名到 rowid 的映射键 为 `chat_name_id` 提供反查关系 | 标准字符串形式的文本内容 => 用户名到 rowid 的映射 |


### `TimeStamp`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| timestamp | INTEGER | 媒体库时间游标 用于增量同步与扫描断点 | 记录事件发生的 Unix 时间戳数值 => 媒体库增量同步游标 |

## 字段取值详解

#### voiceinfo-values
对应表 `VoiceInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| chat_name_id | 命中 `Name2Id.rowid` | 语音记录归属会话 |
| create_time | 记录事件发生的 Unix 时间戳数值（秒级） | 语音消息时间主键之一 |
| create_time | 查询时常用 `create_time ± 5` 秒窗口 | 兼容时间抖动匹配 |
| local_id | `>0` 常见 | 消息 local_id 关联键 |
| svr_id | `>0` | 服务端语音 ID 最强匹配键 |
| voice_data | 非空二进制 | 语音正文数据 |
| data_index | `'0'` | 单分片语音或首分片 |
| data_index | 其他字符串序号 | 后续语音分片序号 |

#### name2id-values
对应表 `Name2Id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| user_name | 标准字符串形式的文本内容 | 用户名到 rowid 的映射 |

#### timestamp-values
对应表 `TimeStamp`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| timestamp | 记录事件发生的 Unix 时间戳数值 | 媒体库增量同步游标 |
