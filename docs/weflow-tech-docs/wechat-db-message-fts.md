---
id: wechat-db-message-fts
title: message_fts.db 字段详解
subtitle: 全文检索字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# message_fts.db 字段详解

## 数据库作用

基于 SQLite FTS 引擎实现的全文检索索引库，用于加速聊天记录的关键词搜索

## 表组与字段

### `table_info`

包含表

- `table_info`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| Key | TEXT 主键 唯一索引 | 索引配置键 | 配置键名 => 索引配置主键 |
| ValueInt64 | INTEGER | 索引整型配置值 | 由系统定义并使用的整型配置项 => 整型配置承载位 |
| ValueDouble | REAL | 索引浮点配置值 | 由系统定义并使用的浮点类配置项 => 浮点配置承载位 |
| ValueStdStr | TEXT | 索引字符串配置值 | 系统使用的标准模式字符串 => 字符串配置承载位 |
| ValueBlob | BLOB | 微信内部预留或扩展字段 | 微信内部预留或扩展字段 => 二进制配置承载位 |
| db_time_stamp | INTEGER | 索引批次时间戳 | 记录事件发生的 Unix 时间戳数值 => 索引批次时间 |
| start_local_id | INTEGER | 批次起始 local_id | `>=0` => 批次起始消息 |
| end_local_id | INTEGER | 批次结束 local_id | `>= start_local_id` => 批次结束消息 |
| session_id | INTEGER | 会话 rowid | 命中 `name2id.rowid` => 会话映射键 |


### `ImgTableInfo`

包含表

- `ImgTableInfo`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| Key | TEXT 主键 唯一索引 | 图像索引配置键 | 配置键名 => 图像索引配置主键 |
| ValueInt64 | INTEGER | 图像索引整型配置值 | 由系统定义并使用的整型配置项 => 整型配置承载位 |
| ValueDouble | REAL | 图像索引浮点配置值 | 由系统定义并使用的浮点类配置项 => 浮点配置承载位 |
| ValueStdStr | TEXT | 图像索引字符串配置值 | 系统使用的标准模式字符串 => 字符串配置承载位 |
| ValueBlob | BLOB | 微信内部预留或扩展字段 | 微信内部预留或扩展字段 => 二进制配置承载位 |


### `ImgFtsAux*`

包含表

- `ImgFtsAux0V0`
- `ImgFtsAux1V0`
- `ImgFtsAux2V0`
- `ImgFtsAux3V0`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| acontent | TEXT | 图像相关检索正文 | 可检索文本 => 图像相关检索正文 |
| message_local_id | INTEGER 联合主键 唯一索引 | 消息 local_id | 消息 local_id => 反查原消息主键 |
| sort_seq | INTEGER 联合主键 唯一索引 | 全局排序序列 | 毫秒序列 => 检索排序主键 |
| local_type | INTEGER | 消息类型编码 | 消息类型编码 => 命中消息类型 |
| session_id | INTEGER 联合主键 唯一索引 | 会话 rowid | 会话 rowid => 会话映射键 |
| sender_id | INTEGER | 发送者 rowid | 发送者 rowid => 发送方映射键 |
| create_time | INTEGER | 消息创建时间 | 记录事件发生的 Unix 时间戳数值（秒级） => 消息时间 |


### `message_fts_v3_aux_*`

包含表

- `message_fts_v3_aux_0`
- `message_fts_v3_aux_1`
- `message_fts_v3_aux_2`
- `message_fts_v3_aux_3`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| message_local_id | INTEGER 联合主键 唯一索引 | 消息 local_id | 消息 local_id => 反查消息主键 |
| sort_seq | INTEGER 联合主键 唯一索引 | 全局排序序列 | 毫秒序列 => 检索排序主键 |
| session_id | INTEGER 联合主键 唯一索引 | 会话 rowid | 会话 rowid => 会话映射键 |


### `message_fts_v4_aux_*`

包含表

- `message_fts_v4_aux_0`
- `message_fts_v4_aux_1`
- `message_fts_v4_aux_2`
- `message_fts_v4_aux_3`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| message_local_id | INTEGER 联合主键 唯一索引 | 消息 local_id | 消息 local_id => 反查消息主键 |
| sort_seq | INTEGER 联合主键 唯一索引 | 全局排序序列 | 毫秒序列 => 检索排序主键 |
| session_id | INTEGER 联合主键 唯一索引 | 会话 rowid | 会话 rowid => 会话映射键 |


### `ImgRangeV0 / message_fts_v3_range / message_fts_v4_range`

包含表

- `ImgRangeV0`
- `message_fts_v3_range`
- `message_fts_v4_range`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| db_time_stamp | INTEGER 联合主键 唯一索引 | 索引批次时间戳 | 记录事件发生的 Unix 时间戳数值 => 索引批次时间 |
| start_local_id | INTEGER | 批次起始 local_id | `>=0` => 批次起始消息 |
| end_local_id | INTEGER | 批次结束 local_id | `>= start_local_id` => 批次结束消息 |
| session_id | INTEGER 联合主键 唯一索引 | 会话 rowid | 会话 rowid => 批次归属会话 |


### `deleteImgFtsV0 / message_fts_v3_session_delete_info / message_fts_v4_session_delete_info`

包含表

- `deleteImgFtsV0`
- `message_fts_v3_session_delete_info`
- `message_fts_v4_session_delete_info`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| session_id | INTEGER 联合主键 唯一索引 | 会话 rowid | 会话 rowid => 删除区间归属会话 |
| start_local_id | INTEGER | 删除区间起始 local_id | `>=0` => 删除区间起点 |
| end_local_id | INTEGER | 删除区间结束 local_id | `>= start_local_id` => 删除区间终点 |
| db_time_stamp | INTEGER 联合主键 唯一索引 | 删除批次时间戳 | 记录事件发生的 Unix 时间戳数值 => 删除批次时间 |


### `ImgFtsV0`

包含表

- `ImgFtsV0`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| session_id | INTEGER 联合主键 唯一索引 | 会话 rowid | 会话 rowid => 会话映射键 |
| local_id | INTEGER 联合主键 唯一索引 | 消息 local_id | 消息 local_id => 反查消息主键 |
| create_time | INTEGER | 消息创建时间 | 记录事件发生的 Unix 时间戳数值（秒级） => 消息时间 |
| sort_seq | INTEGER 联合主键 唯一索引 | 全局排序序列 | 毫秒序列 => 检索排序主键 |


### `message_fts_v3_*_content`

包含表

- `message_fts_v3_0_content`
- `message_fts_v3_1_content`
- `message_fts_v3_2_content`
- `message_fts_v3_3_content`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| id | INTEGER 主键 | 内容行主键 与 aux 行号对应 | 系统自动生成的递增式正整数 rowid 主键 => 与 aux 行号对应的内容主键 |
| c0 | ANY | 主检索列 | 主文本列 => 主检索内容 |
| c1 | ANY | 扩展检索列 1 | 扩展列 => 扩展检索内容 |
| c2 | ANY | 扩展检索列 2 | 扩展列 => 扩展检索内容 |
| c3 | ANY | 扩展检索列 3 | 扩展列 => 扩展检索内容 |
| c4 | ANY | 扩展检索列 4 | 扩展列 => 扩展检索内容 |
| c5 | ANY | 扩展检索列 5 | 扩展列 => 扩展检索内容 |


### `message_fts_v4_*_content`

包含表

- `message_fts_v4_0_content`
- `message_fts_v4_1_content`
- `message_fts_v4_2_content`
- `message_fts_v4_3_content`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| id | INTEGER 主键 | 内容行主键 与 aux 行号对应 | 系统自动生成的递增式正整数 rowid 主键 => 与 aux 行号对应的内容主键 |
| c0 | ANY | 主检索列 | 主文本列 => 主检索内容 |
| c1 | ANY | 扩展检索列 1 | 扩展列 => 扩展检索内容 |
| c2 | ANY | 扩展检索列 2 | 扩展列 => 扩展检索内容 |
| c3 | ANY | 扩展检索列 3 | 扩展列 => 扩展检索内容 |
| c4 | ANY | 扩展检索列 4 | 扩展列 => 扩展检索内容 |
| c5 | ANY | 扩展检索列 5 | 扩展列 => 扩展检索内容 |
| c6 | ANY | 扩展检索列 6 | 扩展列 => 扩展检索内容 |


### `FTS *_data`

包含表

- `ImgFts0V0_data`
- `ImgFts1V0_data`
- `ImgFts2V0_data`
- `ImgFts3V0_data`
- `message_fts_v3_0_data`
- `message_fts_v3_1_data`
- `message_fts_v3_2_data`
- `message_fts_v3_3_data`
- `message_fts_v4_0_data`
- `message_fts_v4_1_data`
- `message_fts_v4_2_data`
- `message_fts_v4_3_data`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| id | INTEGER 主键 | 倒排块主键 | 段块 ID => 倒排块主键 |
| block | BLOB | FTS 倒排块二进制数据 | 二进制块 => FTS 倒排索引数据 |


### `FTS *_docsize`

包含表

- `ImgFts0V0_docsize`
- `ImgFts1V0_docsize`
- `ImgFts2V0_docsize`
- `ImgFts3V0_docsize`
- `message_fts_v3_0_docsize`
- `message_fts_v3_1_docsize`
- `message_fts_v3_2_docsize`
- `message_fts_v3_3_docsize`
- `message_fts_v4_0_docsize`
- `message_fts_v4_1_docsize`
- `message_fts_v4_2_docsize`
- `message_fts_v4_3_docsize`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| id | INTEGER 主键 | 文档长度记录主键 | 文档 ID => 文档长度记录主键 |
| sz | BLOB | FTS 文档长度编码 | 编码二进制 => 文档长度统计数据 |


### `FTS *_config`

包含表

- `ImgFts0V0_config`
- `ImgFts1V0_config`
- `ImgFts2V0_config`
- `ImgFts3V0_config`
- `message_fts_v3_0_config`
- `message_fts_v3_1_config`
- `message_fts_v3_2_config`
- `message_fts_v3_3_config`
- `message_fts_v4_0_config`
- `message_fts_v4_1_config`
- `message_fts_v4_2_config`
- `message_fts_v4_3_config`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| k | ANY 主键 非空 唯一索引 | FTS 配置键 | 配置键 => FTS 配置主键 |
| v | ANY | FTS 配置值 | 配置值 => FTS 配置内容 |


### `FTS *_idx`

包含表

- `ImgFts0V0_idx`
- `ImgFts1V0_idx`
- `ImgFts2V0_idx`
- `ImgFts3V0_idx`
- `message_fts_v3_0_idx`
- `message_fts_v3_1_idx`
- `message_fts_v3_2_idx`
- `message_fts_v3_3_idx`
- `message_fts_v4_0_idx`
- `message_fts_v4_1_idx`
- `message_fts_v4_2_idx`
- `message_fts_v4_3_idx`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| segid | ANY 联合主键 非空 唯一索引 | 倒排段 ID | 段 ID => 倒排段编号 |
| term | ANY 联合主键 非空 唯一索引 | 分词词项 | 分词词项 => 倒排词条键 |
| pgno | ANY | 倒排页号 | 页号 => 倒排页定位 |


### `name2id`

包含表

- `name2id`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| username | TEXT 主键 唯一索引 | 会话用户名到 rowid 的映射键 检索结果需要靠它还原会话标识 | 唯一文本 => 会话用户名映射键 |


### `FTS 虚拟主表`

包含表

- `ImgFts0V0` `ImgFts1V0` `ImgFts2V0` `ImgFts3V0`
- `message_fts_v3_0` `message_fts_v3_1` `message_fts_v3_2` `message_fts_v3_3`
- `message_fts_v4_0` `message_fts_v4_1` `message_fts_v4_2` `message_fts_v4_3`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| 虚拟列集合 | FTS 虚表结构 | 全文检索入口与分词索引容器 | 由 FTS 模块定义 => 全文检索入口 |

## 字段取值详解

#### tableinfo-values
对应表组 `table_info`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| Key | 配置键名 | 索引配置主键 |
| ValueInt64 | 由系统定义并使用的整型配置项 | 整型配置承载位 |
| ValueDouble | 由系统定义并使用的浮点类配置项 | 浮点配置承载位 |
| ValueStdStr | 系统使用的标准模式字符串 | 字符串配置承载位 |
| ValueBlob | 微信内部预留或扩展字段 | 二进制配置承载位 |
| db_time_stamp | 记录事件发生的 Unix 时间戳数值 | 索引批次时间 |
| start_local_id | `>=0` | 批次起始消息 |
| end_local_id | `>= start_local_id` | 批次结束消息 |
| session_id | 命中 `name2id.rowid` | 会话映射键 |

#### imgtableinfo-values
对应表组 `ImgTableInfo`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| Key | 配置键名 | 图像索引配置主键 |
| ValueInt64 | 由系统定义并使用的整型配置项 | 整型配置承载位 |
| ValueDouble | 由系统定义并使用的浮点类配置项 | 浮点配置承载位 |
| ValueStdStr | 系统使用的标准模式字符串 | 字符串配置承载位 |
| ValueBlob | 微信内部预留或扩展字段 | 二进制配置承载位 |

#### imgftsaux-values
对应表组 `ImgFtsAux*`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| acontent | 可检索文本 | 图像相关检索正文 |
| message_local_id | 消息 local_id | 反查原消息主键 |
| sort_seq | 毫秒序列 | 检索排序主键 |
| local_type | 消息类型编码 | 命中消息类型 |
| session_id | 会话 rowid | 会话映射键 |
| sender_id | 发送者 rowid | 发送方映射键 |
| create_time | 记录事件发生的 Unix 时间戳数值（秒级） | 消息时间 |

#### messageftsv3aux-values
对应表组 `message_fts_v3_aux_*`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| message_local_id | 消息 local_id | 反查消息主键 |
| sort_seq | 毫秒序列 | 检索排序主键 |
| session_id | 会话 rowid | 会话映射键 |

#### messageftsv4aux-values
对应表组 `message_fts_v4_aux_*`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| message_local_id | 消息 local_id | 反查消息主键 |
| sort_seq | 毫秒序列 | 检索排序主键 |
| session_id | 会话 rowid | 会话映射键 |

#### imgrangev0-messageftsv3range-messageftsv4range-values
对应表组 `ImgRangeV0 / message_fts_v3_range / message_fts_v4_range`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| db_time_stamp | 记录事件发生的 Unix 时间戳数值 | 索引批次时间 |
| start_local_id | `>=0` | 批次起始消息 |
| end_local_id | `>= start_local_id` | 批次结束消息 |
| session_id | 会话 rowid | 批次归属会话 |

#### deleteimgftsv0-messageftsv3sessiondeleteinfo-messageftsv4sessiondeleteinfo-values
对应表组 `deleteImgFtsV0 / message_fts_v3_session_delete_info / message_fts_v4_session_delete_info`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| session_id | 会话 rowid | 删除区间归属会话 |
| start_local_id | `>=0` | 删除区间起点 |
| end_local_id | `>= start_local_id` | 删除区间终点 |
| db_time_stamp | 记录事件发生的 Unix 时间戳数值 | 删除批次时间 |

#### imgftsv0-values
对应表组 `ImgFtsV0`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| session_id | 会话 rowid | 会话映射键 |
| local_id | 消息 local_id | 反查消息主键 |
| create_time | 记录事件发生的 Unix 时间戳数值（秒级） | 消息时间 |
| sort_seq | 毫秒序列 | 检索排序主键 |

#### messageftsv3content-values
对应表组 `message_fts_v3_*_content`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数 rowid 主键 | 与 aux 行号对应的内容主键 |
| c0 | 主文本列 | 主检索内容 |
| c1 | 扩展列 | 扩展检索内容 |
| c2 | 扩展列 | 扩展检索内容 |
| c3 | 扩展列 | 扩展检索内容 |
| c4 | 扩展列 | 扩展检索内容 |
| c5 | 扩展列 | 扩展检索内容 |

#### messageftsv4content-values
对应表组 `message_fts_v4_*_content`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数 rowid 主键 | 与 aux 行号对应的内容主键 |
| c0 | 主文本列 | 主检索内容 |
| c1 | 扩展列 | 扩展检索内容 |
| c2 | 扩展列 | 扩展检索内容 |
| c3 | 扩展列 | 扩展检索内容 |
| c4 | 扩展列 | 扩展检索内容 |
| c5 | 扩展列 | 扩展检索内容 |
| c6 | 扩展列 | 扩展检索内容 |

#### fts-data-values
对应表组 `FTS *_data`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 段块 ID | 倒排块主键 |
| block | 二进制块 | FTS 倒排索引数据 |

#### fts-docsize-values
对应表组 `FTS *_docsize`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 文档 ID | 文档长度记录主键 |
| sz | 编码二进制 | 文档长度统计数据 |

#### fts-config-values
对应表组 `FTS *_config`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| k | 配置键 | FTS 配置主键 |
| v | 配置值 | FTS 配置内容 |

#### fts-idx-values
对应表组 `FTS *_idx`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| segid | 段 ID | 倒排段编号 |
| term | 分词词项 | 倒排词条键 |
| pgno | 页号 | 倒排页定位 |

#### name2id-values
对应表组 `name2id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 唯一文本 | 会话用户名映射键 |

#### fts-虚拟主表-values
对应表组 `FTS 虚拟主表`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| 虚拟列集合 | 由 FTS 模块定义 | 全文检索入口 |
| 版本扫描顺序 | `v4` 优先 `v3` 兜底 | 检索命中优先级 |
| 时间过滤 | `begin_timestamp * 1000` `end_timestamp * 1000` | 与 `sort_seq` 毫秒序列对齐 |
| 结果时间还原 | `create_time = sort_seq / 1000` | 秒级时间回放 |
