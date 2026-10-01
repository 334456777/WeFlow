---
id: wechat-image-aes-xor-key-acquisition
title: 3.4 图片 AES/XOR 密钥获取
subtitle: 先取 code，再用 wxid 推导并验真；失败时走内存兜底
articleRole: sub
parentId: 03-密钥获取机制总览
link: '#'
---

# 图片 AES/XOR 密钥获取

`code` 来自微信本地缓存目录 `kvcomm` 的文件名。

## 1. 你要准备的两个值

1. `xorKey`：用于 V3 全文件 XOR，或 V2 的尾段 XOR。
2. `aesKey`：用于 V2 的 AES-128-ECB 主段解密。

## 2. `code` 到底怎么准备

### 2.1 找到 kvcomm 目录

按系统找：

1. Windows：`%USERPROFILE%\AppData\Roaming\Tencent\xwechat\net\kvcomm`
2. macOS（常见）：
`~/Library/Containers/com.tencent.xinWeChat/Data/Documents/app_data/net/kvcomm`
3. Linux（常见）：
`~/.xwechat/net/kvcomm`

如果你的微信数据目录是自定义的，原则一样：找到 `net/kvcomm`。

### 2.2 从文件名提取 `code`

在 `kvcomm` 下找类似文件：

1. `key_123456789_xxx.statistic`（Windows/macOS 常见）
2. `key_xxx_123456789_xxx`（Linux 上也常见带额外前缀）

提取规则：

1. 取文件名中的那段十进制数字（`1 ~ 4294967295`）。
2. 去重后作为 `code` 候选集合。

## 3. 准备 `wxid`

准备当前账号 `wxid`，然后做归一化：

1. 如果是数据库目录提取的 `wxid_xxx_yyyy`，只保留到第二个下划线前：`wxid_xxx`。
2. 其他格式保持原值。

## 4. 用 code + wxid 计算 AES/XOR

对每组 `(code, cleanedWxid)` 计算：

```text
xorKey = code & 0xFF
aesKey = MD5(String(code) + cleanedWxid).substring(0, 16)
```

## 5. 验真

推荐用一份 V2 图片样本验真：

1. 从 V2 `.dat` 中取 16 字节密文：偏移 `0x0F..0x1F`。
2. 用候选 `aesKey` 做 AES-128-ECB 解密（无自动填充）。
3. 解密后命中图片头（JPG/PNG/WEBP/WXGF/GIF）才算正确。

命中后，这组 `aesKey + xorKey` 才能用于批量解密。

## 6. 如果拿不到 `code`，怎么兜底

无 `code` 时，依然可以分两步兜底：

1. 从 V2 样本末尾两字节估算 `xorKey`：
候选 `k = x XOR 0xFF`，并要求 `k == (y XOR 0xD9)`。
2. 对微信进程做内存扫描，找可用 `aesKey`，再用“样本头验真”确认。

## 7. 三端差异

1. Windows：`code` 来源稳定，优先走 `kvcomm -> 公式 -> 验真`。
2. macOS：同样先走 `kvcomm`；若进程扫描，常需要管理员授权，且受系统安全策略影响。
3. Linux：先本地 `kvcomm` 提取；失败再走进程扫描，通常需要更高权限。

## 8. 最短执行清单

1. 找到 `kvcomm` 目录并提取全部 `code`。
2. 准备并清洗 `wxid`。
3. 批量计算 `xorKey/aesKey`。
4. 用 V2 样本做 AES 验真。
5. 验真失败再走内存扫描兜底。
