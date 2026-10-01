---
id: wechat-legacy-message-taxonomy
title: 扩展消息类型
subtitle: 文本、媒体、系统与扩展消息的统一判型规则
articleRole: sub
parentId: wechat-legacy-parser-main
link: '#'
---
# local_type 与 appmsg 子类型

## 1. 判型原则

1. 第一层用 `local_type` 判大类。
2. 第二层对 `local_type=49` 或含 `<appmsg>` 的内容解析 `<type>` 子类型。
3. 特殊大整数 `local_type` 直接按固定语义归类。

## 2. 一级类型（local_type）

| local_type | 语义 |
|---|---|
| 1 | 文本 |
| 3 | 图片 |
| 34 | 语音 |
| 42 | 名片 |
| 43 | 视频 |
| 47 | 动画表情 |
| 48 | 位置 |
| 49 | appmsg 容器 |
| 50 | 通话 |
| 10000 | 系统消息 |
| 244813135921 | 引用消息 |
| 266287972401 | 拍一拍 |
| 81604378673 | 聊天记录 |
| 154618822705 | 小程序 |
| 8594229559345 | 红包 |
| 8589934592049 | 转账 |
| 34359738417 | 文件 |
| 103079215153 | 文件 |
| 25769803825 | 文件 |

## 3. 二级类型（appmsg/type）

当消息满足任一条件时进入二级解析：

1. `local_type == 49`
2. 内容包含 `<appmsg>` 或 HTML 实体版本

推荐子类型映射：

| appmsg/type | 语义 |
|---|---|
| 3 | 音乐 |
| 5 | 链接卡片 |
| 6 | 文件 |
| 19 | 聊天记录转发 |
| 33 | 小程序 |
| 36 | 小程序 |
| 49 | 链接卡片 |
| 57 | 引用回复 |
| 87 | 群公告 |
| 2000 | 转账 |
| 2001 | 红包 |

## 4. 判型流程（推荐）

1. 先看 `local_type` 是否命中固定大类。
2. 若 `local_type` 为 49 或内容含 appmsg，再读 `appmsg/type`。
3. `appmsg/type` 有值时覆盖默认 `49=链接` 的粗分类。
4. 当所有子类型匹配失败时，回落使用一级类型的默认占位文案。

## 5. 语义文案标准

标准占位文案：

1. 图片：`[图片]`
2. 语音：`[语音消息]`
3. 视频：`[视频]`
4. 位置：`[位置]`
5. 系统：`[系统消息]`
6. 转账：`[转账]`
7. 红包：`[红包]`
8. 引用：`[引用消息]`

## 6. 统计 SQL

## 6.1 会话内 local_type 分布

```sql
SELECT local_type, COUNT(*) AS cnt
FROM message_0
GROUP BY local_type
ORDER BY cnt DESC;
```

多分片库可按同结构分别统计后合并。

## 6.2 49 类消息子类型分布

```sql
SELECT
  CASE
    WHEN message_content LIKE '%<type>57</type>%' THEN '57'
    WHEN message_content LIKE '%<type>19</type>%' THEN '19'
    WHEN message_content LIKE '%<type>6</type>%'  THEN '6'
    WHEN message_content LIKE '%<type>5</type>%'  THEN '5'
    ELSE 'other'
  END AS appmsg_type,
  COUNT(*) AS cnt
FROM message_0
WHERE local_type = 49
GROUP BY appmsg_type
ORDER BY cnt DESC;
```

## 7. 误判高发点

1. 只按 `local_type=49` 统一当链接，忽略子类型。
2. 错误使用全局正则提取 `<type>`，导致误判嵌套结构体内的同名节点。
3. 位置消息未解析 `location.label/poiname`，导致内容不可读。
4. 系统消息未做标签清洗，输出残留 XML。

## 8. 验收标准

1. 统计总量与数据库行数一致。
2. 常见子类型（链接/文件/聊天记录/小程序/引用）均可区分。
3. 大整数 `local_type` 不再落入 `unknown`。
4. 导出文案不出现大段 XML 原文。
