---
id: wechat-legacy-sns-parse
title: 朋友圈解析机制
subtitle: 从 SnsTimeLine 原始记录到可读帖子、媒体与互动信息
articleRole: sub
parentId: 02-核心解析机制
link: '#'
---

# 朋友圈解析机制

朋友圈解析的目标很简单：  
把数据库里的一条原始记录，转换成“可读帖子对象”。

## 1. 最小输入

从 `sns.db` 内的 `SnsTimeLine` 表中提取这些字段：

1. `tid`
2. `user_name`
3. `content`

## 2. 如何通过tid推出时间

`tid` 可以直接反推时间：

`createTime = (tid >> 23) / 1000`

所以按时间区间查询时，优先用 `tid` 边界过滤，而不是先把所有 XML 全量解析后再筛选。

## 3. 一条朋友圈内可以提取的内容

从 `content` 的 `TimelineObject` 里提取：

1. 帖子 ID（唯一标识）
2. 发布者
3. 正文文本（`contentDesc`）
4. 帖子类型（图文/视频）

## 4. 媒体字段的读取方式

每个 `media` 节点建议统一提取：

1. `url`
2. `thumb`
3. `md5`
4. `token`
5. `key`
6. `enc_idx`

这些字段就是后续媒体下载与解密的输入。

## 5. LivePhoto 的单独处理逻辑

如果 `media` 内有 `LivePhoto/liveMedia`：

1. 把静态图和实况视频当成两条媒体。
2. 实况视频走视频下载/解密链路，不要按静态图处理。

## 6. 评论与点赞怎么还原

互动信息通常在 `LocalExtraInfo`：

1. 点赞：点赞用户名列表
2. 评论：评论内容、评论者、被回复对象

评论建议至少输出：

1. `id`
2. `username`
3. `nickname`
4. `content`
5. `ref_comment_id`
6. `ref_username`
7. `ref_nickname`

## 7. URL 归一化

为提高后续下载成功率，建议统一：

1. `http://` -> `https://`
2. 图片 `/150` -> `/0`（优先原图）
3. 无 token 时补 `token=<token>&idx=1`

## 8. 去重规则

同一帖子可能出现重复记录，建议用帖子 ID 去重：

1. 先按 `tid DESC` 读取。
2. 同 ID 只保留第一条（通常就是最新的一条）。

