---
id: wechat-db-message
title: message_*.db 字段详解
subtitle: 消息分片字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# message_*.db 字段详解

## 数据库作用

存储单聊与群聊会话的结构化消息文本、元数据，是消息的核心存储库

## 表与字段

### `Msg_*`

> 说明 `Msg_*` 表名由会话标识（即wxid或chatroomid）经过MD5生成，即结构为：Msg_<md5>，每张表承载一个会话的部分消息流，和一个人的聊天信息可能分布在每一个message_*.db文件中,但表名字是相同的

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#message-x-values) | INTEGER 主键 | 消息本地主键 会话内核心定位键 |
| [server_id](#message-x-values) | INTEGER 普通索引 | 服务端消息 ID 用于云端对账与回补 |
| [local_type](#message-x-values) | INTEGER 普通索引 | 消息类型编码 如文本 图片 语音 视频 表情 |
| [sort_seq](#message-x-values) | INTEGER 普通索引 | 全局排序序列 跨分表聚合时优先使用 |
| [real_sender_id](#message-x-values) | INTEGER 普通索引 | 真实发送者 rowid 关联 `Name2Id.rowid` |
| [create_time](#message-x-values) | INTEGER | 消息创建时间戳 |
| [status](#message-x-values) | INTEGER | 消息状态值 反映发送生命周期 |
| [upload_status](#message-x-values) | INTEGER | 上传状态值 |
| [download_status](#message-x-values) | INTEGER | 下载状态值 |
| [server_seq](#message-x-values) | INTEGER | 服务端同步序列号 |
| [origin_source](#message-x-values) | INTEGER | 来源状态位 |
| [source](#message-x-values) | TEXT | 来源文本 记录消息来源上下文 |
| [message_content](#message-x-values) | TEXT | 原始消息内容 |
| [compress_content](#message-x-values) | TEXT | 压缩消息内容 常用于富文本载荷 |
| [packed_info_data](#message-x-values) | BLOB | 微信内部扩展字段 |
| [WCDB_CT_message_content](#message-x-values) | INTEGER 默认 NULL | 微信内部扩展字段 |
| [WCDB_CT_source](#message-x-values) | INTEGER 默认 NULL | 微信内部扩展字段 |


### `Name2Id`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [user_name](#name2id-values) | TEXT 主键 唯一索引 | 用户名到 rowid 的映射键 供 `real_sender_id` 与 `chat_name_id` 反查 |
| [is_session](#name2id-values) | INTEGER | 会话标记 区分会话键与普通用户键 |


### `TimeStamp`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [timestamp](#timestamp-values) | INTEGER | 消息库时间游标 用于增量同步边界 |


### `SendInfo`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [chat_name_id](#sendinfo-values) | INTEGER 唯一索引 | 会话 rowid |
| [msg_local_id](#sendinfo-values) | INTEGER 唯一索引 | 消息 local_id |


### `DeleteInfo`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [chat_name_id](#deleteinfo-values) | INTEGER 唯一索引 普通索引 | 会话 rowid |
| [delete_table_name](#deleteinfo-values) | TEXT 唯一索引 普通索引 | 删除记录对应的目标表名 |


### `DeleteResInfo`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#deleteresinfo-values) | INTEGER 主键 | 删除资源记录主键 |
| [session_name_id](#deleteresinfo-values) | INTEGER 普通索引 | 会话 rowid |
| [msg_create_time](#deleteresinfo-values) | INTEGER 普通索引 | 原消息创建时间 |
| [msg_local_id](#deleteresinfo-values) | INTEGER 普通索引 | 原消息 local_id |
| [res_path](#deleteresinfo-values) | TEXT | 被删除资源的文件路径 |


### `HistoryAddMsgInfo`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [session_name_id](#historyaddmsginfo-values) | INTEGER 联合主键 唯一索引 | 会话 rowid |
| [history_id](#historyaddmsginfo-values) | INTEGER 联合主键 唯一索引 | 历史批次 ID |
| [server_id](#historyaddmsginfo-values) | INTEGER 联合主键 唯一索引 | 服务端消息 ID |
| [is_revoke](#historyaddmsginfo-values) | INTEGER | 历史消息撤回标记 |


### `HistorySysMsgInfo`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [session_name_id](#historysysmsginfo-values) | INTEGER 联合主键 唯一索引 | 会话 rowid |
| [history_id](#historysysmsginfo-values) | INTEGER 联合主键 唯一索引 | 历史批次 ID |
| [server_id](#historysysmsginfo-values) | INTEGER 联合主键 唯一索引 | 服务端系统消息 ID |
| [is_revoke](#historysysmsginfo-values) | INTEGER | 系统消息撤回标记 |


### `wcdb_builtin_compression_record`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tableName](#wcdbbuiltincompressionrecord-values) | TEXT 主键 非空 唯一索引 | 压缩配置对应表名 |
| [columns](#wcdbbuiltincompressionrecord-values) | TEXT 非空 | 参与压缩的列集合 |
| [rowid](#wcdbbuiltincompressionrecord-values) | INTEGER | 压缩记录行标识 |

## 字段取值详解

#### message-x-values
对应表 `Msg_*）`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 会话内系统自动生成的递增式正整数主键 | 单会话消息主键 |
| server_id | `>0` 常见 | 服务端消息主键 用于云端对账 |
| local_type | `1` | 文本消息 |
| local_type | `3` | 图片消息 |
| local_type | `34` | 语音消息 |
| local_type | `42` | 名片消息 |
| local_type | `43` | 视频消息 |
| local_type | `47` | 动画表情消息 |
| local_type | `48` | 位置消息 |
| local_type | `49` | AppMsg 容器消息 需再看 XML 子类型 |
| local_type | `50` | 通话消息 |
| local_type | `10000` | 系统消息 |
| local_type | `244813135921` | 引用消息 |
| local_type | `266287972401` | 拍一拍消息 |
| local_type | `81604378673` | 聊天记录消息 |
| local_type | `154618822705` | 小程序消息 |
| local_type | `8594229559345` | 红包消息 |
| local_type | `8589934592049` | 转账消息 |
| local_type | `34359738417` `103079215153` `25769803825` | 文件消息 |
| sort_seq | `>0` | 全局排序主键 毫秒序列 |
| sort_seq | 缺失时回退 `create_time * 1000` | 统一跨分片排序 |
| real_sender_id | `real_sender_id == myRowId` | 当前账号发送 |
| real_sender_id | 其他 rowid | 他人发送 |
| create_time | 记录事件发生的 Unix 时间戳数值（秒级） | 消息发生时间 |
| status | 微信内部扩展字段 | 发送生命周期状态枚举 |
| upload_status | 微信内部扩展字段 | 上传状态枚举 |
| download_status | 微信内部扩展字段 | 下载状态枚举 |
| server_seq | 递增序列 | 服务端同步序列 |
| origin_source | 微信内部扩展字段 | 来源状态位 |
| source | 标准字符串形式的文本内容 | 来源描述文本 |
| message_content | XML 中 `<type>2000</type>` | 转账消息 |
| message_content | XML 中 `<type>2001</type>` | 红包消息 |
| message_content | XML 中 `<type>3</type>` | 音乐消息 |
| message_content | XML 中 `<type>5</type>` 或 `<type>49</type>` | 链接消息 |
| message_content | XML 中 `<type>6</type>` | 文件消息 |
| message_content | XML 中 `<type>19</type>` | 聊天记录消息 |
| message_content | XML 中 `<type>33</type>` 或 `<type>36</type>` | 小程序消息 |
| message_content | XML 中 `<type>51</type>` | 视频号消息 |
| message_content | XML 中 `<type>57</type>` | 引用回复消息 |
| message_content | XML 中 `<type>87</type>` | 群公告消息 |
| message_content | XML 中 `<type>115</type>` | 礼物消息 |
| message_content | 通话 XML `<room_type>0</room_type>` | 视频通话 |
| message_content | 通话 XML `<room_type>1</room_type>` | 语音通话 |
| message_content | 转账 XML `<paysubtype>1</paysubtype>` | 发起侧转账 |
| message_content | 转账 XML `<paysubtype>3</paysubtype>` | 收款侧转账 |
| compress_content | XML 或压缩载荷文本 | 富文本和结构化内容容器 |
| packed_info_data | 微信内部扩展字段 | 微信内部扩展字段 |
| WCDB_CT_message_content | 微信内部扩展字段 | 微信内部扩展字段 |
| WCDB_CT_source | 微信内部扩展字段 | 微信内部扩展字段 |

#### name2id-values
对应表 `Name2Id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| user_name | 唯一文本 | 用户名到 rowid 的映射 |
| is_session | 微信内部扩展字段 | 会话键标记枚举 |

#### timestamp-values
对应表 `TimeStamp`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| timestamp | 记录事件发生的 Unix 时间戳数值 | 消息库增量同步游标 |

#### sendinfo-values
对应表 `SendInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| chat_name_id | 命中会话 rowid | 发送关联会话键 |
| msg_local_id | 命中消息 local_id | 发送关联消息键 |

#### deleteinfo-values
对应表 `DeleteInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| chat_name_id | 命中会话 rowid | 删除记录归属会话 |
| delete_table_name | 表名文本 | 删除记录作用目标表 |

#### deleteresinfo-values
对应表 `DeleteResInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 系统自动生成的递增式正整数主键 | 删除资源记录主键 |
| session_name_id | 会话 rowid | 资源归属会话 |
| msg_create_time | 记录事件发生的 Unix 时间戳数值 | 原消息时间 |
| msg_local_id | 消息 local_id | 原消息主键 |
| res_path | 文件路径文本 | 被删除资源路径 |

#### historyaddmsginfo-values
对应表 `HistoryAddMsgInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| session_name_id | 会话 rowid | 历史补偿归属会话 |
| history_id | 批次号 | 历史补偿批次 |
| server_id | 服务端消息 ID | 历史消息主键 |
| is_revoke | 微信内部扩展字段 | 撤回标记枚举 |

#### historysysmsginfo-values
对应表 `HistorySysMsgInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| session_name_id | 会话 rowid | 历史系统消息归属会话 |
| history_id | 批次号 | 历史系统消息批次 |
| server_id | 服务端系统消息 ID | 历史系统消息主键 |
| is_revoke | 微信内部扩展字段 | 撤回标记枚举 |

#### wcdbbuiltincompressionrecord-values
对应表 `wcdb_builtin_compression_record`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tableName | 表名文本 | 压缩配置作用对象 |
| columns | 列名集合文本 | 参与压缩字段集合 |
| rowid | 系统自动生成的递增式正整数主键 | 压缩配置记录键 |
