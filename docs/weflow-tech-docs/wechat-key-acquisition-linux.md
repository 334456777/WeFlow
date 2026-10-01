---
id: wechat-key-acquisition-linux
title: 3.3 Linux：数据库密钥挂点定位机制
subtitle: ELF 静态扫描定位 targetVA，再映射到进程绝对地址
articleRole: sub
parentId: 03-密钥获取机制总览
link: '#'
---

# Linux：数据库密钥挂点定位机制

## 1. `db_scan` 的核心入口

1. 先取 `微信的pid`。
2. 用 `LinuxScanner::SearchForHookAddress("/proc/<pid>/exe", scanRes)` 找 `targetVA`。
3. 再把 `targetVA` 转成运行时绝对地址 `targetAddr`。

## 2. 静态定位算法

### 2.1 解析 ELF 节区

扫描器先解析目标 ELF 的：

1. `.rodata`
2. `.text`

### 2.2 字符串锚点

在 `.rodata` 查找：

`com.Tencent.WCDB.Config.Cipher`

### 2.3 第一跳引用

在 `.text` 里找：

`lea rsi, [rip+disp32]`（字节 `48 8D 35`）

并要求它指向上面的字符串地址。

### 2.4 反推出中间目标（unkVA）

以第一跳命中点为基准，检查其前 7 字节是否是：

`48 8D 3D <disp32>`（`lea rdi, [rip+disp32]`）

若成立，计算得到 `unkVA`。

### 2.5 第二跳引用

继续在 `.text` 查找 `lea rsi` 指向 `unkVA` 的位置。

### 2.6 回溯函数头

从第二跳位置向后回扫最多 `0x500` 字节，寻找函数序言：

`55 41 57`

命中后该位置即 `headVA`，并作为 `result.targetVA` 返回。

## 3. 从 `targetVA` 到 `targetAddr`

`db_scan` 不直接返回可 Hook 的绝对地址，而是：

1. 读取 `/proc/<pid>/maps` 找主可执行映射基址 `baseAddr`。
2. 计算：`targetAddr = baseAddr + targetVA`。

最终输出给 `db_hook` 的就是这个 `targetAddr`。

## 4. `db_hook` 如何使用该挂点

`db_hook` 阶段调用 `LinuxHooker::CaptureKey(pid, targetAddr, ...)`：

1. attach 全线程。
2. 在 DR0 设置硬件断点到 `targetAddr`。
3. 命中后检查 `keyLen == 32` 并提取 key buffer。

