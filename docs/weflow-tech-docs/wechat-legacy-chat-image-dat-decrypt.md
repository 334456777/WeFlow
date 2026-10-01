---
id: wechat-legacy-chat-image-dat-decrypt
title: 聊天图片 DAT 解密机制
subtitle: 从 .dat 到可查看图片的版本识别、分段解密与格式还原
articleRole: sub
parentId: 02-核心解析机制
link: '#'
---

# 聊天图片 DAT 解密机制

微信聊天图片在本地通常不是直接的 `.jpg/.png`，而是被保存为 `.dat`。  
想要正确还原图片，你需要这样：

1. 判断这份 `.dat` 属于哪种版本。
2. 按该版本规则做 AES/XOR 解密。
3. 对结果做图片格式识别（部分图片会做 `wxgf` 封装，需要特殊处理）。

## 1. 解密前需要准备什么

1. 一份图片 `.dat` 文件
2. `xorKey`（1 字节数值，0~255）
3. `aesKey`（16 字节 ASCII；仅 V2 必需）

说明：

1. 旧版（V3）只用 XOR。
2. V1/V2 是“头部描述 + AES 段 + 明文段 + XOR 尾段”的复合结构。
3. V1 的 AES 密钥是固定值 `cfcd208495d565ef`；V2 使用账号相关 AES 密钥。

## 2. 先判断 DAT 版本（必须）

读取文件前 6 字节：

1. `07 08 56 31 08 07` -> V1
2. `07 08 56 32 08 07` -> V2
3. 其他 -> V3（旧版）

## 3. V3：全文件 XOR 解密

V3 最直接：

`plain[i] = cipher[i] XOR xorKey`

做完整文件逐字节 XOR 后，通常就能直接得到 JPG/PNG/GIF/WebP 头。

## 4. V1 / V2：分段解密（重点）

### 4.1 数据布局

V1/V2 前 15 字节是头：

1. `header[0:6]`：版本魔数
2. `header[6:10]`：`aesSize`（小端 int32）
3. `header[10:14]`：`xorSize`（小端 int32）
4. `header[15...]`：负载数据 `payload`

`payload` 再拆为三段：

1. `aesData`：前段 AES 密文（长度需要按 16 字节块对齐）
2. `rawData`：中段原样数据
3. `xorTail`：末段 XOR 数据（长度为 `xorSize`）

### 4.2 AES 段的逻辑

先算对齐后长度：

```text
remainder = aesSize % 16
alignedAesSize = aesSize + (16 - remainder)
如果 remainder == 0，仍需 +16（PKCS#7 满块填充）
```

然后从 `payload` 开头截取 `alignedAesSize` 字节作为 `aesData`。

### 4.3 AES 段如何解密

1. 算法：`AES-128-ECB`
2. 密钥：16 字节 ASCII
3. V1 固定使用 `cfcd208495d565ef`；V2 使用你准备好的 `aesKey`
4. 关闭自动填充
5. 解密后执行严格 PKCS#7 去填充

### 4.4 XOR 尾段怎么解密

`payload` 去掉 `aesData` 后剩下 `remaining`。  
其中最后 `xorSize` 字节是 `xorTail`：

`xorPlain[i] = xorTail[i] XOR xorKey`

`remaining` 前面那部分是 `rawData`，不做处理。

### 4.5 大功告成

最终原图二进制：

`plain = unpaddedAes + rawData + xorPlain`

### 4.6 伪代码

```text
if V3:
  plain = xor_all(cipher, xorKey)
else:
  header = cipher[0:0x0F]
  payload = cipher[0x0F:]
  aesSize = le_i32(header[6:10])
  xorSize = le_i32(header[10:14])
  alignedAesSize = align_pkcs7(aesSize, 16)

  aesData = payload[0:alignedAesSize]
  decAes = AES-128-ECB-Decrypt(aesData, aesKey)
  unpaddedAes = strict_pkcs7_unpad(decAes)

  remaining = payload[alignedAesSize:]
  rawData = remaining[0:len(remaining)-xorSize]
  xorTail = remaining[len(remaining)-xorSize:]
  xorPlain = xor_each(xorTail, xorKey)

  plain = unpaddedAes + rawData + xorPlain
```

## 5. 输出格式判断

解密后先看文件头：

1. `FF D8 FF` -> JPG
2. `89 50 4E 47` -> PNG
3. `47 49 46` -> GIF
4. `52 49 46 46 .... 57 45 42 50` -> WebP

按命中的头写对应后缀即可。

## 6. wxgf：解密后仍打不开时怎么处理

如果解密后前 4 字节是 `77 78 67 66`（`wxgf`），说明它不是普通图片。  
常见处理顺序：

1. 在前几 KB 内先尝试搜内嵌 JPG/PNG 头，命中则从该偏移截出。
2. 若没有内嵌头，提取内部 HEVC NALU 数据。
3. 用 ffmpeg 等工具把 HEVC 转成 JPG/PNG。

所以解密成功但无法直接预览在 `wxgf` 下是正常现象，不代表密钥错误。