---
id: wechat-legacy-sns-decrypt-download
title: 朋友圈媒体下载与解密
subtitle: 图片、视频、LivePhoto 与评论表情的下载与还原流程
articleRole: sub
parentId: 02-核心解析机制
link: '#'
---

# 朋友圈媒体下载与解密

把朋友圈里的媒体 URL 变成本地可查看文件要怎么做呢？

## 1. 你需要的收集的数据

1. `url`
2. `token`（可选）
3. `key`（可选，常用于媒体流 XOR）
4. `enc_idx`（可选）

评论中的表情还需要：

1. `encrypt_url`
2. `aes_key`

## 2. 下载前先做 URL 归一化

1. `http://` 统一改为 `https://`
2. 非视频图片 URL 把 `/150` 升级为 `/0`
3. 缺 `token` 时补 `token=<token>&idx=1`
4. 视频 URL 拼 token 时保留原 query

## 3. 先判断媒体类型

1. 帖子类型是视频（如 type=15）优先当视频处理。
2. URL 命中 `snsvideodownload` / `video` / `.mp4` 当视频。
3. URL 包含 `vweixinthumb` 仅视为缩略图，不当主视频。
4. 命中 `LivePhoto` 时，额外生成一条实况视频任务。

## 4. 图片下载与解密

### 4.1 触发解密条件

当满足以下任一条件，且 `key` 可用时进入解密：

1. 响应头 `x-enc = 1`
2. XML中明确给出了 `key`

### 4.2 解密方式

图片链路使用基于 `key` 的 ISAAC64 keystream，然后做逐字节 XOR：

`plain[i] = cipher[i] XOR keystream[i]`

图片会对全长度做 XOR。

### 4.3 成功判定

解密后命中常见图片头（JPG/PNG/GIF/WEBP）即通过。

## 5. 视频与 LivePhoto 解密

视频和实况视频走同一条链路：

1. 先完整下载到临时文件。
2. 生成 128KB keystream。
3. 仅对前 `min(fileLen, 128KB)` 字节做 XOR。
4. 其余字节保持原样。

快速校验：偏移 `4..7` 应为 `ftyp`。

## 6. 评论表情

评论里的表情是 `encrypt_url + aes_key` 的组合。  
处理顺序：

1. 先尝试把响应当明文图片直接识别。
2. 若不是明文，再尝试 AES 解密。

AES 建议按多布局/多模式依次尝试：

1. AES-GCM（多 nonce/tag 布局）
2. AES-CBC
3. AES-ECB

解密成功后按图片头决定后缀并落盘。

## 7. 缓存策略

媒体下载建议做 URL 级缓存：

1. 同一 URL 命中缓存直接复用。
2. 仅在未命中时重新请求网络。

## 8. 常见的一些失败

1. 403：token 拼接错误或缺失。
2. 图片打不开：把非加密内容误解密，或 key 不是有效数字键。
3. 视频不可播：未做前 128KB 解密，或 key 错误。
4. LivePhoto 丢失：只处理了静态图，漏了 live video 链路。
