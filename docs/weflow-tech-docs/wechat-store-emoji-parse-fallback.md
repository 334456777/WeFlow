---
id: wechat-store-emoji-parse-fallback
title: 商店表情解析与无 URL 处理
subtitle: 从消息列提取 md5/url，到 emoticon.db 回填与本地缓存兜底
articleRole: sub
parentId: 02-核心解析机制
link: '#'
---

# 商店表情解析与无 URL 处理

微信聊天里的表情消息（`local_type = 47`）通常不是“消息里直接放完整图片”，而是放一组索引字段。  
你要做的是先从消息提取索引，再用 `md5` 去定位可下载地址或本地缓存。

## 1. 先从消息列提取 5 个关键字段

一条表情消息里常见这 5 个字段：

1. `md5`：表情主键，后续定位都靠它
2. `cdnurl`：可直接下载的主地址
3. `thumburl`：缩略图地址
4. `encrypturl`：加密地址（有些消息会给）
5. `aeskey`：配套密钥（有些消息会给）

这一步的目标是得到：

1. 至少有 `md5`
2. 最好同时有 `cdnurl`

## 2. 表情的标准解析链路

当消息本身带 `cdnurl` 时，流程最短：

1. 用 `cdnurl` 下载表情文件
2. 用 `md5` 作为稳定文件名或索引键
3. 按文件头判断格式（GIF/PNG/JPG/WEBP），确定后缀

如果你还需要“表情语义文案”（例如把表情渲染成“[让我看看]”这类文本），可在 `emoticon.db` 里按 `md5` 查：

1. `kStoreEmoticonCaptionsTable.md5_` 对应 `caption_`

## 3. 没有标记 URL 的表情包怎么处理

很多转发/历史消息只给 `md5`，不带 `cdnurl`。这时按下面顺序处理：

1. 先查 `emoticon.db` 的 `kNonStoreEmoticonTable`，用 `md5` 回填 `cdn_url`
2. 若数据库也没命中，再去本地表情缓存目录按 `md5` 找文件
3. 两者都失败，则这条表情当前无法恢复正文（只能保留消息占位）

`emoticon.db` 常见位置是：`账号目录/db_storage/emoticon/emoticon.db`

SQL示例：

```sql
SELECT cdn_url
FROM kNonStoreEmoticonTable
WHERE md5 = '你的32位md5'
COLLATE NOCASE
LIMIT 1;
```

查询示例：

```sql
SELECT caption_
FROM kStoreEmoticonCaptionsTable
WHERE md5_ = '你的32位md5'
COLLATE NOCASE
LIMIT 1;
```

## 4. 本地缓存兜底怎么做

当 `cdn_url` 查不到时，直接按 `md5` 在缓存目录尝试这些后缀：

1. `.gif`
2. `.png`
3. `.webp`
4. `.jpg`
5. `.jpeg`

只要命中其中一个文件，就能直接显示/导出，不需要再走网络下载。

