---
id: wechat-key-acquisition-windows
title: 3.1 Windows：数据库密钥挂点定位机制
subtitle: 从字符串引用链到版本特征回退的双通道定位
articleRole: sub
parentId: 03-密钥获取机制总览
link: '#'
---

# Windows：数据库密钥定位

## 1. 路径：`RemoteScannerCommon::SearchForHookAddress`

### 1.1 锁定模块与节区

1. 先定位 `Weixin.dll`。
2. 读取远程 PE 节区，拿到 `.rdata` 和 `.text`。

### 1.2 以字符串为锚点

1. 在 `.rdata` 中查找字符串：`com.Tencent.WCDB.Config.Cipher`。
2. 在 `.text` 里找对该字符串的 `lea rdx, [rip+rel32]` 引用（opcode `48 8D 15`）。

### 1.3 计算中间对象地址（unk）

对第一个命中的 `lea rdx`，向前 7 字节检查是否存在：

`48 8D 0D <rel32>`（`lea rcx, [rip+rel32]`）

若成立，则按 RIP 相对寻址规则计算 `unk` 的实际地址。

### 1.4 二次引用追踪到目标函数

1. 再次在 `.text` 中查找对 `unk` 的 `lea rdx` 引用。
2. 从该引用点向后回溯 0x100 字节，查找 `CC CC CC CC` 边界。
3. 命中后将 `head + 4` 作为最终挂点地址。


### 2. 整体镜像分块扫描

1. 对 `Weixin.dll` 做 1MB 分块扫描。
2. 按 `pattern + mask` 匹配全部候选。
3. 要求候选数必须等于 1。
4. 最终挂点 = `match + offset`。

