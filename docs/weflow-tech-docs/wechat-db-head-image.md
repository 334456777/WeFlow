---
id: wechat-db-head-image
title: head_image.db 字段详解
subtitle: 头像字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# head_image.db 字段详解

## 数据库作用

记录联系人与群聊的头像二进制数据及更新标记

## 表与字段

### `head_image`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| username | TEXT 主键 唯一索引 | 联系人唯一标识 头像主键 | 账号标识文本（待实库核对） => 头像缓存主键 |
| md5 | TEXT | 头像内容 MD5 摘要 | 采用标准的 32 位十六进制特征摘要 => 头像内容指纹 |
| image_buffer | BLOB | 头像原始二进制数据 | 非空二进制 => 本地头像缓存可直接回填；空值 => 需回退 URL 或远端拉取 |
| update_time | INTEGER | 头像更新时间戳 | 记录事件的 Unix 时间戳数值，值越大代表越新 => 头像缓存版本时间 |

## 字段取值详解

#### headimage-values
对应表 `head_image`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 账号标识文本（待实库核对） | 头像缓存主键 |
| md5 | 采用标准的 32 位十六进制特征摘要 | 头像内容指纹 |
| image_buffer | 非空二进制 | 本地头像缓存可直接回填 |
| image_buffer | 空值 | 需回退 URL 或远端拉取 |
| update_time | 记录事件的 Unix 时间戳数值，值越大代表越新 | 头像缓存版本时间 |
