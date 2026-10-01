---
id: wechat-db-sns
title: sns.db 字段详解
subtitle: 朋友圈字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# sns.db 字段详解

## 数据库作用

维护朋友圈时间线的 XML 结构化内容、评论互动数据及本地缓存状态。

## 表与字段

### `SnsTimeLine`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snstimeline-values) | INTEGER 主键 唯一索引 | 朋友圈主键 雪花型时间键 可直接用于时间排序 |
| [user_name](#snstimeline-values) | TEXT | 动态发布者标识 |
| [content](#snstimeline-values) | TEXT | 朋友圈 XML 正文 包含文本 媒体 点赞 评论等主体信息 |
| [pack_info_buf](#snstimeline-values) | TEXT | 微信内部扩展字段 |


### `SnsAdTimeLine`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snsadtimeline-values) | INTEGER 主键 唯一索引 | 广告动态主键 |
| [username](#snsadtimeline-values) | TEXT | 发布者标识 |
| [content](#snsadtimeline-values) | TEXT | 广告动态 XML 正文 |
| [create_time](#snsadtimeline-values) | INTEGER 普通索引 | 动态创建时间 |
| [ad_content](#snsadtimeline-values) | TEXT | 广告内容文本 |
| [ad_create_time](#snsadtimeline-values) | INTEGER | 广告创建时间 |
| [exposure_time](#snsadtimeline-values) | INTEGER | 广告曝光时间 |
| [exposure_count](#snsadtimeline-values) | INTEGER | 广告曝光次数 |
| [remind_source_info](#snsadtimeline-values) | TEXT | 广告提醒来源信息 |
| [remind_self_info](#snsadtimeline-values) | TEXT | 与当前账号相关的提醒信息 |
| [extra_data](#snsadtimeline-values) | TEXT | 扩展广告数据 |


### `SnsMessage_tmp3`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#snsmessagetmp3-values) | INTEGER 主键 普通索引 | 互动消息本地主键 |
| [create_time](#snsmessagetmp3-values) | INTEGER 普通索引 | 互动消息创建时间 |
| [type](#snsmessagetmp3-values) | INTEGER | 互动类型编码 如点赞 评论 提醒 |
| [feed_id](#snsmessagetmp3-values) | INTEGER | 关联动态 ID |
| [is_unread](#snsmessagetmp3-values) | INTEGER | 未读标记 |
| [from_username](#snsmessagetmp3-values) | TEXT | 发起方标识 |
| [from_nickname](#snsmessagetmp3-values) | TEXT | 发起方昵称 |
| [to_username](#snsmessagetmp3-values) | TEXT | 目标方标识 |
| [to_nickname](#snsmessagetmp3-values) | TEXT | 目标方昵称 |
| [content](#snsmessagetmp3-values) | TEXT | 互动 XML 内容 |
| [serialized_comment](#snsmessagetmp3-values) | TEXT | 评论序列化文本 |
| [serialized_ref](#snsmessagetmp3-values) | TEXT | 引用序列化文本 |
| [comment_id](#snsmessagetmp3-values) | INTEGER | 评论 ID |
| [client_id](#snsmessagetmp3-values) | TEXT | 客户端任务 ID |
| [comment64_id](#snsmessagetmp3-values) | INTEGER | 64 位评论 ID |
| [comment_flag](#snsmessagetmp3-values) | INTEGER | 评论状态位 |
| [serialized_comment_buf](#snsmessagetmp3-values) | BLOB | 微信内部扩展字段 |
| [serialized_ref_buf](#snsmessagetmp3-values) | BLOB | 微信内部扩展字段 |
| [del_status](#snsmessagetmp3-values) | INTEGER | 删除状态 |
| [is_relative_me](#snsmessagetmp3-values) | INTEGER | 是否与当前账号相关 |


### `SnsDraft`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#snsdraft-values) | INTEGER 主键 | 草稿主键 |
| [create_time](#snsdraft-values) | INTEGER | 草稿创建时间 |
| [ui_type](#snsdraft-values) | INTEGER 唯一索引 | 草稿对应发布界面类型 |
| [content](#snsdraft-values) | TEXT | 草稿 XML 内容 |
| [client_id](#snsdraft-values) | TEXT | 客户端任务 ID |
| [status](#snsdraft-values) | INTEGER | 草稿状态 |


### `SnsPublishTask`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#snspublishtask-values) | INTEGER 主键 | 发布任务主键 |
| [create_time](#snspublishtask-values) | INTEGER | 任务创建时间 |
| [ui_type](#snspublishtask-values) | INTEGER 唯一索引 | 发布界面类型 |
| [content](#snspublishtask-values) | TEXT | 发布任务 XML 内容 |
| [client_id](#snspublishtask-values) | TEXT | 客户端任务 ID |
| [status](#snspublishtask-values) | INTEGER | 发布状态 |


### `SnsPendingDraftDeletionTable`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#snspendingdraftdeletiontable-values) | INTEGER 主键 | 待删草稿主键 |
| [create_time](#snspendingdraftdeletiontable-values) | INTEGER | 创建时间 |
| [ui_type](#snspendingdraftdeletiontable-values) | INTEGER | 发布界面类型 |
| [content](#snspendingdraftdeletiontable-values) | TEXT | 待删草稿 XML 内容 |
| [client_id](#snspendingdraftdeletiontable-values) | TEXT | 客户端任务 ID |
| [status](#snspendingdraftdeletiontable-values) | INTEGER | 任务状态 |
| [tid](#snspendingdraftdeletiontable-values) | TEXT | 关联朋友圈主键 |


### `SnsTopItem_1`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snstopitem1-values) | INTEGER | 置顶项关联动态主键 |
| [username](#snstopitem1-values) | TEXT | 置顶项所属用户 |
| [summary](#snstopitem1-values) | TEXT | 微信内部扩展字段 |
| [create_time](#snstopitem1-values) | INTEGER | 置顶项创建时间 |
| [last_read_time](#snstopitem1-values) | INTEGER | 最近阅读时间 |
| [is_read](#snstopitem1-values) | INTEGER | 已读标记 |


### `SnsMainTimeLineBreakFlag`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snsmaintimelinebreakflag-values) | INTEGER | 朋友圈主键 |
| [tid_heigh_bit](#snsmaintimelinebreakflag-values) | INTEGER 联合主键 唯一索引 | tid 高位分段键 |
| [tid_low_bit](#snsmaintimelinebreakflag-values) | INTEGER 联合主键 唯一索引 | tid 低位分段键 |
| [break_flag](#snsmaintimelinebreakflag-values) | INTEGER | 主时间线分页断点标记 |


### `SnsUserTimeLineBreakFlagV2`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snsusertimelinebreakflagv2-values) | INTEGER | 朋友圈主键 |
| [tid_heigh_bit](#snsusertimelinebreakflagv2-values) | INTEGER 普通索引 | tid 高位分段键 |
| [tid_low_bit](#snsusertimelinebreakflagv2-values) | INTEGER 普通索引 | tid 低位分段键 |
| [break_flag](#snsusertimelinebreakflagv2-values) | INTEGER | 用户时间线分页断点标记 |
| [user_name](#snsusertimelinebreakflagv2-values) | TEXT 普通索引 | 用户标识 |


### `SnsErrorMessage`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [local_id](#snserrormessage-values) | INTEGER 主键 | 错误记录主键 |
| [error_type](#snserrormessage-values) | INTEGER 普通索引 | 错误类型编码 |
| [creat_time](#snserrormessage-values) | INTEGER 普通索引 | 错误发生时间 |
| [tid](#snserrormessage-values) | INTEGER 普通索引 | 关联朋友圈主键 |
| [packed_info_data](#snserrormessage-values) | TEXT | 微信内部扩展字段 |


### `SnsIgnoredDataItem`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snsignoreddataitem-values) | INTEGER 主键 | 被忽略的动态主键 |


### `SnsNoteVoice`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [tid](#snsnotevoice-values) | INTEGER 联合主键 唯一索引 | 动态主键 |
| [data_id](#snsnotevoice-values) | TEXT 联合主键 唯一索引 | 语音条目 ID |
| [buff](#snsnotevoice-values) | TEXT | 语音数据内容 |

## 字段取值详解

#### snstimeline-values
对应表 `SnsTimeLine`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | `createTime = (tid >> 23) / 1000` | 可由主键反推出发布时间 |
| tid | 高 41 位时间 低 23 位序列 | 雪花型主键结构 |
| user_name | 用户名文本 | 动态发布者 |
| content | XML 中 `ContentObject.type = 1` | 图片动态 |
| content | XML 中 `ContentObject.type = 15` | 视频动态 |
| content | XML 中其他 type | 微信内部扩展字段 |
| pack_info_buf | 微信内部扩展字段 | 微信内部扩展字段 |

#### snsadtimeline-values
对应表 `SnsAdTimeLine`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | 雪花型主键 | 广告动态主键 |
| username | 用户名文本 | 广告发布方 |
| content | 广告 XML 文本 | 广告主体内容 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 动态创建时间 |
| ad_content | 标准字符串形式的文本内容 | 广告内容 |
| ad_create_time | 记录事件发生的 Unix 时间戳数值 | 广告创建时间 |
| exposure_time | 记录事件发生的 Unix 时间戳数值 | 广告曝光时间 |
| exposure_count | `>=0` | 曝光次数 |
| remind_source_info | 标准字符串形式的文本内容 | 来源提醒信息 |
| remind_self_info | 标准字符串形式的文本内容 | 与当前账号相关提醒 |
| extra_data | 微信内部扩展字段 | 扩展载荷 |

#### snsmessagetmp3-values
对应表 `SnsMessage_tmp3`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 系统自动生成的递增式正整数主键 | 互动消息主键 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 互动时间 |
| type | 微信内部扩展字段 | 互动类型枚举 |
| feed_id | 朋友圈 ID | 关联动态 |
| is_unread | `1` | 互动未读 |
| is_unread | `0` | 互动已读 |
| from_username | 用户名文本 | 发起方 |
| from_nickname | 标准字符串形式的文本内容 | 发起方昵称 |
| to_username | 用户名文本 | 目标方 |
| to_nickname | 标准字符串形式的文本内容 | 目标方昵称 |
| content | XML 文本 | 互动主体 |
| serialized_comment | 标准字符串形式的文本内容 | 评论序列化内容 |
| serialized_ref | 标准字符串形式的文本内容 | 引用序列化内容 |
| comment_id | 数值 ID | 评论标识 |
| client_id | 客户端任务 ID | 客户端追踪键 |
| comment64_id | 64 位数值 ID | 评论长整型标识 |
| comment_flag | 微信内部扩展字段 | 评论状态位 |
| serialized_comment_buf | 微信内部扩展字段 | 微信内部扩展字段 |
| serialized_ref_buf | 微信内部扩展字段 | 微信内部扩展字段 |
| del_status | 微信内部扩展字段 | 删除状态枚举 |
| is_relative_me | `1` | 与当前账号相关 |
| is_relative_me | `0` | 与当前账号无直接关系 |

#### snsdraft-values
对应表 `SnsDraft`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 系统自动生成的递增式正整数主键 | 草稿主键 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 草稿创建时间 |
| ui_type | 微信内部扩展字段 | 发布界面类型 |
| content | XML 文本 | 草稿内容 |
| client_id | 客户端任务 ID | 客户端追踪键 |
| status | 微信内部扩展字段 | 草稿状态枚举 |

#### snspublishtask-values
对应表 `SnsPublishTask`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 系统自动生成的递增式正整数主键 | 发布任务主键 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 任务创建时间 |
| ui_type | 微信内部扩展字段 | 发布界面类型 |
| content | XML 文本 | 发布任务内容 |
| client_id | 客户端任务 ID | 客户端追踪键 |
| status | 微信内部扩展字段 | 发布状态枚举 |

#### snspendingdraftdeletiontable-values
对应表 `SnsPendingDraftDeletionTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 系统自动生成的递增式正整数主键 | 待删任务主键 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 创建时间 |
| ui_type | 微信内部扩展字段 | 发布界面类型 |
| content | XML 文本 | 待删草稿内容 |
| client_id | 客户端任务 ID | 客户端追踪键 |
| status | 微信内部扩展字段 | 任务状态枚举 |
| tid | 朋友圈 ID 文本 | 关联动态主键 |

#### snstopitem1-values
对应表 `SnsTopItem_1`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | 朋友圈 ID | 置顶动态键 |
| username | 用户名文本 | 置顶项所属用户 |
| summary | 微信内部扩展字段 | 置顶摘要 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 置顶创建时间 |
| last_read_time | 记录事件发生的 Unix 时间戳数值 | 最近阅读时间 |
| is_read | `1` | 已读 |
| is_read | `0` | 未读 |

#### snsmaintimelinebreakflag-values
对应表 `SnsMainTimeLineBreakFlag`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | 朋友圈 ID | 对应动态主键 |
| tid_heigh_bit | tid 高位切片 | 时间线分段键 |
| tid_low_bit | tid 低位切片 | 时间线分段键 |
| break_flag | 微信内部扩展字段 | 主时间线分页断点位 |

#### snsusertimelinebreakflagv2-values
对应表 `SnsUserTimeLineBreakFlagV2`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | 朋友圈 ID | 对应动态主键 |
| tid_heigh_bit | tid 高位切片 | 用户时间线分段键 |
| tid_low_bit | tid 低位切片 | 用户时间线分段键 |
| break_flag | 微信内部扩展字段 | 用户时间线分页断点位 |
| user_name | 用户名文本 | 分页断点归属用户 |

#### snserrormessage-values
对应表 `SnsErrorMessage`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| local_id | 系统自动生成的递增式正整数主键 | 错误记录主键 |
| error_type | 微信内部扩展字段 | 错误类型枚举 |
| creat_time | 记录事件发生的 Unix 时间戳数值 | 错误发生时间 |
| tid | 朋友圈 ID | 关联动态 |
| packed_info_data | 微信内部扩展字段 | 微信内部扩展字段 |

#### snsignoreddataitem-values
对应表 `SnsIgnoredDataItem`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | 朋友圈 ID | 被忽略动态主键 |

#### snsnotevoice-values
对应表 `SnsNoteVoice`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| tid | 朋友圈 ID | 语音归属动态 |
| data_id | 语音条目 ID | 动态内语音资源标识 |
| buff | 文本或编码数据 | 语音内容载荷 |
