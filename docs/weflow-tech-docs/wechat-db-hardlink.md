---
id: wechat-db-hardlink
title: hardlink.db 字段详解
subtitle: 图片与视频映射字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# hardlink.db 字段详解

## 数据库作用

建立资源到本地物理文件路径的映射，通过硬链接技术优化跨会话资源的存储与复用效率。

## 表与字段

### `image_hardlink_info_v4`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [md5_hash](#imagehardlinkinfov4-values) | INTEGER 普通索引 | MD5 的整型哈希值 用于快速定位 |
| [md5](#imagehardlinkinfov4-values) | TEXT | 资源内容 MD5 |
| [type](#imagehardlinkinfov4-values) | INTEGER | 资源类型编码 |
| [file_name](#imagehardlinkinfov4-values) | TEXT | 附件文件名 |
| [file_size](#imagehardlinkinfov4-values) | INTEGER | 文件大小 字节 |
| [modify_time](#imagehardlinkinfov4-values) | INTEGER 普通索引 | 文件更新时间戳 |
| [dir1](#imagehardlinkinfov4-values) | INTEGER 普通索引 | 一级目录 rowid |
| [dir2](#imagehardlinkinfov4-values) | INTEGER | 二级目录 rowid |
| [_rowid_](#imagehardlinkinfov4-values) | INTEGER 主键 | 硬链记录主键 |
| [extra_buffer](#imagehardlinkinfov4-values) | BLOB | 微信内部扩展字段 |


### `video_hardlink_info_v4`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [md5_hash](#videohardlinkinfov4-values) | INTEGER 普通索引 | MD5 的整型哈希值 |
| [md5](#videohardlinkinfov4-values) | TEXT | 视频内容 MD5 |
| [type](#videohardlinkinfov4-values) | INTEGER | 资源类型编码 |
| [file_name](#videohardlinkinfov4-values) | TEXT | 视频文件名 |
| [file_size](#videohardlinkinfov4-values) | INTEGER | 文件大小 字节 |
| [modify_time](#videohardlinkinfov4-values) | INTEGER 普通索引 | 文件更新时间戳 |
| [dir1](#videohardlinkinfov4-values) | INTEGER 普通索引 | 一级目录 rowid |
| [dir2](#videohardlinkinfov4-values) | INTEGER | 二级目录 rowid |
| [_rowid_](#videohardlinkinfov4-values) | INTEGER 主键 | 硬链记录主键 |
| [extra_buffer](#videohardlinkinfov4-values) | BLOB | 微信内部扩展字段 |


### `file_hardlink_info_v4`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [md5_hash](#filehardlinkinfov4-values) | INTEGER 普通索引 | MD5 的整型哈希值 |
| [md5](#filehardlinkinfov4-values) | TEXT | 文件内容 MD5 |
| [type](#filehardlinkinfov4-values) | INTEGER | 资源类型编码 |
| [file_name](#filehardlinkinfov4-values) | TEXT | 文件名 |
| [file_size](#filehardlinkinfov4-values) | INTEGER | 文件大小 字节 |
| [modify_time](#filehardlinkinfov4-values) | INTEGER 普通索引 | 文件更新时间戳 |
| [dir1](#filehardlinkinfov4-values) | INTEGER 普通索引 | 一级目录 rowid |
| [dir2](#filehardlinkinfov4-values) | INTEGER | 二级目录 rowid |
| [_rowid_](#filehardlinkinfov4-values) | INTEGER 主键 | 硬链记录主键 |
| [extra_buffer](#filehardlinkinfov4-values) | BLOB | 微信内部扩展字段 |


### `dir2id`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#dir2id-values) | TEXT 主键 唯一索引 | 目录段名称 通过 rowid 与 `dir1` `dir2` 建立映射 |


### `file_checkpoint_v4`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [month_id](#filecheckpointv4-values) | INTEGER 主键 | 文件扫描月份分桶检查点 |


### `video_checkpoint_v4`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [month_id](#videocheckpointv4-values) | INTEGER 主键 | 视频扫描月份分桶检查点 |


### `talker_checkpoint_v4`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [talker_id](#talkercheckpointv4-values) | INTEGER 主键 | 会话 rowid 检查点键 |
| [month_id](#talkercheckpointv4-values) | INTEGER 普通索引 | 对应月份分桶 |


### `db_info`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [Key](#dbinfo-values) | TEXT 主键 唯一索引 | 配置键名 |
| [ValueInt64](#dbinfo-values) | INTEGER | 整型配置值 |
| [ValueDouble](#dbinfo-values) | REAL | 浮点配置值 |
| [ValueStdStr](#dbinfo-values) | TEXT | 字符串配置值 |
| [ValueBlob](#dbinfo-values) | BLOB | 微信内部扩展字段 |

## 字段取值详解

#### imagehardlinkinfov4-values
对应表 `image_hardlink_info_v4`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| md5_hash | 与 `md5` 对应的整型哈希 | 用于快速缩小检索范围 再由 `md5` 做精确确认 |
| md5 | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 | 图片资源主键 用于反查附件路径 |
| type | 微信内部扩展字段 | 资源类型枚举位 暂未确定公开语义 |
| file_name | 常见为 `*.dat` | 附件落盘文件名 参与最终路径拼接 |
| file_size | `>0` | 图片文件大小 字节 |
| modify_time | 记录时间事件的 Unix 时间戳数值 | 文件最后修改时间 |
| dir1 | `>0` 且命中 `dir2id.rowid` | 一级目录段索引 |
| dir2 | `>0` 且命中 `dir2id.rowid` | 二级目录段索引 |
| _rowid_ | 系统自动生成的递增式正整数主键 | 记录主键 |
| extra_buffer | 微信内部扩展字段 | 微信内部扩展数据 |

#### videohardlinkinfov4-values
对应表 `video_hardlink_info_v4`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| md5_hash | 与 `md5` 对应的整型哈希 | 视频硬链快速检索键 |
| md5 | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 | 视频资源主键 |
| type | 微信内部扩展字段 | 资源类型枚举位 暂未确定公开语义 |
| file_name | 常见形式 `md5.ext` | 用于派生真实视频索引键 去扩展名后得到 `resolved_md5` |
| file_size | `>0` | 视频文件大小 字节 |
| modify_time | 记录时间事件的 Unix 时间戳数值 | 视频文件最后修改时间 |
| dir1 | `>0` 且命中 `dir2id.rowid` | 一级目录段索引 |
| dir2 | `>0` 且命中 `dir2id.rowid` | 二级目录段索引 |
| _rowid_ | 系统自动生成的递增式正整数主键 | 记录主键 |
| extra_buffer | 微信内部扩展字段 | 微信内部扩展数据 |

#### filehardlinkinfov4-values
对应表 `file_hardlink_info_v4`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| md5_hash | 与 `md5` 对应的整型哈希 | 文件硬链快速检索键 |
| md5 | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 | 文件资源主键 |
| type | 微信内部扩展字段 | 资源类型枚举位 暂未确定公开语义 |
| file_name | 文件名文本 | 文件落盘名称 |
| file_size | `>0` | 文件大小 字节 |
| modify_time | 记录时间事件的 Unix 时间戳数值 | 文件最后修改时间 |
| dir1 | `>0` 且命中 `dir2id.rowid` | 一级目录段索引 |
| dir2 | `>0` 且命中 `dir2id.rowid` | 二级目录段索引 |
| _rowid_ | 系统自动生成的递增式正整数主键 | 记录主键 |
| extra_buffer | 微信内部扩展字段 | 微信内部扩展数据 |

#### dir2id-values
对应表 `dir2id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 目录段字符串 | 由 `rowid` 被 `dir1` `dir2` 引用 参与拼接 `msg/attach/<dir1>/<dir2>/...` |

#### filecheckpointv4-values
对应表 `file_checkpoint_v4`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| month_id | 月份分桶键 | 文件扫描进度检查点 便于增量补扫 |

#### videocheckpointv4-values
对应表 `video_checkpoint_v4`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| month_id | 月份分桶键 | 视频扫描进度检查点 |

#### talkercheckpointv4-values
对应表 `talker_checkpoint_v4`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| talker_id | 会话 rowid | 按会话记录扫描进度 |
| month_id | 月份分桶键 | 会话维度的分桶检查点 |

#### dbinfo-values
对应表 `db_info`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| Key | 配置键名 | 配置项主键 |
| ValueInt64 | 整型值 | 整型配置承载位 |
| ValueDouble | 浮点值 | 浮点配置承载位 |
| ValueStdStr | 字符串值 | 字符串配置承载位 |
| ValueBlob | 微信内部扩展字段 | 二进制配置承载位 |
