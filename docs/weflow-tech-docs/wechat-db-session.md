---
id: wechat-db-session
title: session.db 字段详解
subtitle: 会话与未读状态字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# session.db 字段详解

## 数据库作用

会话列表的信息记录，记录了每个会话的基本信息，包括会话类型、未读消息数、最近消息时间等

## 表与字段

### `SessionTable`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#sessiontable-values) | TEXT 主键 唯一索引 | 会话唯一标识 单聊 群聊 公众号都以该键组织 |
| [type](#sessiontable-values) | INTEGER 普通索引 | 会话类型编码 用于会话分组与筛选 |
| [unread_count](#sessiontable-values) | INTEGER | 会话当前未读计数 |
| [unread_first_msg_srv_id](#sessiontable-values) | INTEGER | 当前未读区间首条消息的服务端 ID |
| [is_hidden](#sessiontable-values) | INTEGER | 会话隐藏标记 控制是否在主列表展示 |
| [summary](#sessiontable-values) | TEXT | 会话摘要文本 通常显示最近一条消息预览 |
| [draft](#sessiontable-values) | TEXT | 输入框草稿内容 |
| [status](#sessiontable-values) | INTEGER | 会话状态位 记录置顶等会话状态 |
| [last_timestamp](#sessiontable-values) | INTEGER | 最近消息时间戳 |
| [sort_timestamp](#sessiontable-values) | INTEGER | 会话排序时间戳 会话列表主排序键 |
| [last_clear_unread_timestamp](#sessiontable-values) | INTEGER | 最近一次清空未读的时间戳 |
| [last_msg_locald_id](#sessiontable-values) | INTEGER | 最近消息的本地 local_id 指针 |
| [last_msg_type](#sessiontable-values) | INTEGER | 最近消息主类型 |
| [last_msg_sub_type](#sessiontable-values) | INTEGER | 最近消息子类型 |
| [last_msg_sender](#sessiontable-values) | TEXT 普通索引 | 最近消息发送方标识 |
| [last_sender_display_name](#sessiontable-values) | TEXT | 最近发送方展示名缓存 |
| [last_msg_ext_type](#sessiontable-values) | INTEGER | 最近消息扩展类型 |
| [unread_first_pat_msg_local_id](#sessiontable-values) | INTEGER | 未读拍一拍首条 local_id |
| [unread_first_pat_msg_sort_seq](#sessiontable-values) | INTEGER | 未读拍一拍首条 sort_seq |


### `Name2Id`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [user_name](#name2id-values) | TEXT 主键 唯一索引 | 会话标识到 rowid 的映射键 为未读等子表提供整型外键 |


### `SessionUnreadListTable_1`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username_id](#sessionunreadlisttable1-values) | INTEGER 联合主键 唯一索引 普通索引 | 会话 rowid |
| [server_id](#sessionunreadlisttable1-values) | INTEGER 联合主键 唯一索引 | 未读消息服务端 ID |
| [create_time](#sessionunreadlisttable1-values) | INTEGER 普通索引 | 未读消息时间戳 用于未读区间排序 |


### `SessionUnreadStatTable_1`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username_id](#sessionunreadstattable1-values) | INTEGER 主键 | 会话 rowid |
| [unread_stat](#sessionunreadstattable1-values) | INTEGER | 未读统计状态值 汇总未读计数或状态位 |


### `SessionDeleteTable`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#sessiondeletetable-values) | TEXT 主键 唯一索引 | 被删除会话标识 |
| [delete_time](#sessiondeletetable-values) | INTEGER | 删除时间戳 |


### `SessionNoContactInfoTable`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#sessionnocontactinfotable-values) | TEXT 主键 唯一索引 | 无联系人资料的会话标识 |
| [session_title](#sessionnocontactinfotable-values) | TEXT | 该类会话的兜底标题文本 |

## 字段取值详解

#### sessiontable-values
对应表 `SessionTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 以 `@chatroom` 结尾 | 群会话标识 |
| username | 以 `gh_` 开头 | 公众号会话标识 |
| username | 其他常规值 | 单聊或系统会话标识 |
| type | 微信内部扩展字段 | 会话类型枚举 |
| unread_count | `0` | 当前无未读 |
| unread_count | `>0` | 当前存在未读 |
| unread_first_msg_srv_id | `>0` 常见 | 未读区间首条消息服务端 ID |
| is_hidden | `1` | 会话在主列表隐藏 |
| is_hidden | `0` | 会话在主列表可见 |
| summary | 标准字符串形式的文本内容 | 会话最近摘要 |
| draft | 非空文本 | 输入框草稿内容 |
| status | 微信内部扩展字段 | 会话状态位组合 |
| last_timestamp | 记录事件发生的 Unix 时间戳数值 | 最近消息时间 |
| sort_timestamp | 系统使用该数值进行降序排列，数值越大在界面中展示越靠前 | 会话列表主排序键 |
| last_clear_unread_timestamp | 记录事件发生的 Unix 时间戳数值 | 最近清空未读时间 |
| last_msg_locald_id | `>0` 常见 | 最近消息 local_id 指针 |
| last_msg_type | `1` | 最近消息为文本 |
| last_msg_type | `3` | 最近消息为图片 |
| last_msg_type | `34` | 最近消息为语音 |
| last_msg_type | `43` | 最近消息为视频 |
| last_msg_type | `47` | 最近消息为动画表情 |
| last_msg_type | `49` | 最近消息为链接或卡片容器 |
| last_msg_type | `50` | 最近消息为通话 |
| last_msg_type | `10000` | 最近消息为系统消息 |
| last_msg_sub_type | 微信内部扩展字段 | 最近消息子类型枚举 |
| last_msg_sender | 用户名文本 | 最近消息发送方 |
| last_sender_display_name | 标准字符串形式的文本内容 | 最近发送方展示名缓存 |
| last_msg_ext_type | 微信内部扩展字段 | 最近消息扩展类型 |
| unread_first_pat_msg_local_id | `>0` 常见 | 未读拍一拍首条 local_id |
| unread_first_pat_msg_sort_seq | `>0` 常见 | 未读拍一拍首条排序序列 |

#### name2id-values
对应表 `Name2Id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| user_name | 唯一文本 | 会话用户名到 rowid 的映射 |

#### sessionunreadlisttable1-values
对应表 `SessionUnreadListTable_1`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username_id | 命中 `Name2Id.rowid` | 会话外键 |
| server_id | `>0` 常见 | 未读消息服务端 ID |
| create_time | 记录事件发生的 Unix 时间戳数值 | 未读消息时间 用于未读序列排序 |

#### sessionunreadstattable1-values
对应表 `SessionUnreadStatTable_1`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username_id | 命中 `Name2Id.rowid` | 会话外键 |
| unread_stat | 微信内部扩展字段 | 未读统计状态位 |

#### sessiondeletetable-values
对应表 `SessionDeleteTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 会话标识文本 | 被删除会话键 |
| delete_time | 记录事件发生的 Unix 时间戳数值 | 删除时间 |

#### sessionnocontactinfotable-values
对应表 `SessionNoContactInfoTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 会话标识文本 | 无联系人资料会话键 |
| session_title | 标准字符串形式的文本内容 | 兜底标题 |
