# 基准测试：base 与 head

- base：`v4.0.0`（`4e8844c139`），weflow 4.3.0
- head：`v4.1.0`（`1161df68e4`），weflow 4.3.0
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 5 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 24.3 MiB | 24.6 MiB | 1.01× |

## 命令

**有运行失败**（输出见 `logs/`，不计入中位数）：`export media images`（base 5 次），`export messages json with images`（base 5 次），`export messages json`（base 5 次），`export messages chatlab`（base 5 次），`export messages excel`（base 5 次），`export messages html`（base 5 次），`export messages txt`（base 5 次），`report annual`（base 5 次），`analytics overall`（base 5 次），`group members`（base 5 次），`group ranking`（base 5 次），`chat export-stats`（base 5 次），`chat date-counts`（base 5 次），`chat search`（base 5 次），`report dual`（base 5 次），`chat messages first page`（base 5 次），`chat messages offset`（base 5 次）

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 36,570（36,143–37,969） | - |
| `export messages json with images` | - | 41,674（41,123–49,240） | - |
| `export messages json` | - | 5,197（5,114–5,269） | - |
| `export messages chatlab` | - | 4,591（4,572–4,923） | - |
| `export messages excel` | - | 6,384（6,310–6,468） | - |
| `export messages html` | - | 3,941（3,920–3,992） | - |
| `export messages txt` | - | 3,878（3,730–3,972） | - |
| `report annual` | - | 3,655（3,586–3,655） | - |
| `analytics overall` | - | 1,127（1,113–1,409） | - |
| `group members` | - | 1,342（1,330–1,358） | - |
| `group ranking` | - | 1,312（1,310–1,337） | - |
| `chat export-stats` | - | 1,289（1,273–1,320） | - |
| `chat date-counts` | - | 973（953–987） | - |
| `chat search` | - | 2,780（2,767–2,819） | - |
| `report dual` | - | 699（690–725） | - |
| `chat messages first page` | - | 330（328–359） | - |
| `chat messages offset` | - | 2,833（2,801–3,296） | - |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 2,349 | - |
| `export messages json with images` | - | 807 | - |
| `export messages json` | - | 714 | - |
| `export messages chatlab` | - | 864 | - |
| `export messages excel` | - | 721 | - |
| `export messages html` | - | 723 | - |
| `export messages txt` | - | 717 | - |
| `report annual` | - | 18 | - |
| `analytics overall` | - | 161 | - |
| `report dual` | - | 18 | - |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 18,219 | - |
| `export messages json with images` | - | 26,797 | - |
| `export messages json` | - | 11,703 | - |
| `export messages chatlab` | - | 10,375 | - |
| `export messages excel` | - | 12,797 | - |
| `export messages html` | - | 11,125 | - |
| `export messages txt` | - | 10,391 | - |
| `report annual` | - | 3,469 | - |
| `analytics overall` | - | 1,172 | - |
| `group members` | - | 1,234 | - |
| `group ranking` | - | 1,250 | - |
| `chat export-stats` | - | 1,156 | - |
| `chat date-counts` | - | 969 | - |
| `chat search` | - | 2,531 | - |
| `report dual` | - | 719 | - |
| `chat messages first page` | - | 438 | - |
| `chat messages offset` | - | 2,281 | - |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 47,047 | - |
| `export messages json with images` | - | 48,047 | - |
| `export messages json` | - | 922 | - |
| `export messages chatlab` | - | 766 | - |
| `export messages excel` | - | 1,031 | - |
| `export messages html` | - | 953 | - |
| `export messages txt` | - | 859 | - |
| `report annual` | - | 297 | - |
| `analytics overall` | - | 203 | - |
| `group members` | - | 266 | - |
| `group ranking` | - | 188 | - |
| `chat export-stats` | - | 250 | - |
| `chat date-counts` | - | 172 | - |
| `chat search` | - | 391 | - |
| `report dual` | - | 109 | - |
| `chat messages first page` | - | 78 | - |
| `chat messages offset` | - | 656 | - |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 300 | - |
| `export messages json with images` | - | 605 | - |
| `export messages json` | - | 250 | - |
| `export messages chatlab` | - | 256 | - |
| `export messages excel` | - | 284 | - |
| `export messages html` | - | 266 | - |
| `export messages txt` | - | 267 | - |
| `report annual` | - | 134 | - |
| `analytics overall` | - | 120 | - |
| `group members` | - | 204 | - |
| `group ranking` | - | 209 | - |
| `chat export-stats` | - | 203 | - |
| `chat date-counts` | - | 203 | - |
| `chat search` | - | 325 | - |
| `report dual` | - | 56.1 | - |
| `chat messages first page` | - | 39.7 | - |
| `chat messages offset` | - | 959 | - |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 443 | - |
| `export messages json with images` | - | 813 | - |
| `export messages json` | - | 455 | - |
| `export messages chatlab` | - | 446 | - |
| `export messages excel` | - | 477 | - |
| `export messages html` | - | 464 | - |
| `export messages txt` | - | 463 | - |
| `report annual` | - | 284 | - |
| `analytics overall` | - | 246 | - |
| `group members` | - | 337 | - |
| `group ranking` | - | 339 | - |
| `chat export-stats` | - | 333 | - |
| `chat date-counts` | - | 330 | - |
| `chat search` | - | 471 | - |
| `report dual` | - | 191 | - |
| `chat messages first page` | - | 162 | - |
| `chat messages offset` | - | 1,112 | - |

### 峰值线程数

每 20 ms 采样一次主进程得到的最大值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 12 | - |
| `export messages json with images` | - | 12 | - |
| `export messages json` | - | 12 | - |
| `export messages chatlab` | - | 12 | - |
| `export messages excel` | - | 12 | - |
| `export messages html` | - | 12 | - |
| `export messages txt` | - | 12 | - |
| `report annual` | - | 10 | - |
| `analytics overall` | - | 10 | - |
| `group members` | - | 10 | - |
| `group ranking` | - | 10 | - |
| `chat export-stats` | - | 10 | - |
| `chat date-counts` | - | 10 | - |
| `chat search` | - | 10 | - |
| `report dual` | - | 10 | - |
| `chat messages first page` | - | 10 | - |
| `chat messages offset` | - | 10 | - |

### 读取（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 1,157 | - |
| `export messages json with images` | - | 1,635 | - |
| `export messages json` | - | 679 | - |
| `export messages chatlab` | - | 523 | - |
| `export messages excel` | - | 557 | - |
| `export messages html` | - | 531 | - |
| `export messages txt` | - | 474 | - |
| `report annual` | - | 146 | - |
| `analytics overall` | - | 142 | - |
| `group members` | - | 207 | - |
| `group ranking` | - | 207 | - |
| `chat export-stats` | - | 232 | - |
| `chat date-counts` | - | 206 | - |
| `chat search` | - | 524 | - |
| `report dual` | - | 42.7 | - |
| `chat messages first page` | - | 28.4 | - |
| `chat messages offset` | - | 429 | - |

### 写入（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 952 | - |
| `export messages json with images` | - | 1,381 | - |
| `export messages json` | - | 431 | - |
| `export messages chatlab` | - | 99.1 | - |
| `export messages excel` | - | 98.2 | - |
| `export messages html` | - | 122 | - |
| `export messages txt` | - | 14.6 | - |
| `report annual` | - | 0.0 | - |
| `analytics overall` | - | 0.0 | - |
| `group members` | - | 0.1 | - |
| `group ranking` | - | 0.0 | - |
| `chat export-stats` | - | 0.0 | - |
| `chat date-counts` | - | 0.0 | - |
| `chat search` | - | 0.1 | - |
| `report dual` | - | 0.0 | - |
| `chat messages first page` | - | 0.1 | - |
| `chat messages offset` | - | 36.5 | - |

### 页错误

进程及其子进程，包括软页错误。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | - | 180,181 | - |
| `export messages json with images` | - | 295,385 | - |
| `export messages json` | - | 102,684 | - |
| `export messages chatlab` | - | 112,727 | - |
| `export messages excel` | - | 112,595 | - |
| `export messages html` | - | 106,352 | - |
| `export messages txt` | - | 123,008 | - |
| `report annual` | - | 37,436 | - |
| `analytics overall` | - | 32,276 | - |
| `group members` | - | 61,488 | - |
| `group ranking` | - | 59,217 | - |
| `chat export-stats` | - | 53,676 | - |
| `chat date-counts` | - | 54,449 | - |
| `chat search` | - | 91,551 | - |
| `report dual` | - | 23,253 | - |
| `chat messages first page` | - | 10,304 | - |
| `chat messages offset` | - | 248,859 | - |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | - | 4,799 | - |
| `export messages json` | - | 38,484 | - |
| `export messages chatlab` | - | 43,564 | - |
| `export messages excel` | - | 31,329 | - |
| `export messages html` | - | 50,752 | - |
| `export messages txt` | - | 51,572 | - |
| `report annual` | - | 89,145 | - |
| `group members` | - | 149,076 | - |
| `group ranking` | - | 152,396 | - |
| `chat export-stats` | - | 155,156 | - |
| `chat date-counts` | - | 205,607 | - |
| `report dual` | - | 14,302 | - |
| `chat messages first page` | - | 152 | - |
| `chat messages offset` | - | 18 | - |

## `weflow serve` 持续负载

没有完成：base：weflow serve exited (2) before it listened: error: unexpected argument '--api-only' found   tip: a similar argument exists: '--api-token' Usage: weflow-base.exe serve --port <PORT> --api-token <API_TOKEN> For more information, try '--help'.；head：weflow serve exited (2) before it listened: error: unexpected argument '--api-only' found   tip: a similar argument exists: '--api-token' Usage: weflow-head.exe serve --port <PORT> --api-token <API_TOKEN> For more information, try '--help'.。完整信息见 results.json 的 `load_failed`。

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 1，head 1。非 0 表示它在自己的合成账号上有检查没通过（旧版本缺少某个路由时会这样），计时照常保留，原因见 `logs/serve-bench-*.txt`。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。base 没有计时结果：它的 `weflow serve` 没有启动（v5.0.0 之前的版本没有现在的命令 API），见 `logs/serve-bench-base.txt`。head 没有计时结果：它的 `weflow serve` 没有启动（v5.0.0 之前的版本没有现在的命令 API），见 `logs/serve-bench-head.txt`。
## 内存曲线

主进程的工作集，每 20 ms 采样一次（持续负载每 0.5 s 一次）。命令取耗时居中的那次运行。蓝线 base，红线 head；先结束的版本在退出后记为 0。

### `export media images`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 36.569
    y-axis "MiB" 0 --> 300
    line [24.4, 158.3, 249.3, 269.4, 299.4, 299.6, 185.1, 194.3, 194.9, 182.2, 186.9, 189.3, 186.9, 188.5, 183.8, 186.2, 186.2, 197.9, 198.1, 197.8, 197.9, 197.6, 217.2, 218.1, 196.3, 193.1, 197.4, 204.9, 206.3, 212.1, 231.1, 232.4, 226.2, 226.4, 218.5, 217.4, 217.4, 181.2, 179.8, 180.4, 180.4, 180.4, 182.4, 180.7, 183.7, 183.8, 189.4, 189.4, 185.3, 207.1, 207.1, 188.4, 188.4, 184.0, 184.0, 198.8, 198.9, 198.9, 187.4, 194.4]
```

### `export messages json with images`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 41.651
    y-axis "MiB" 0 --> 800
    line [39.1, 201.2, 293.4, 372.0, 500.3, 545.1, 599.9, 601.3, 550.2, 550.2, 558.4, 559.6, 553.4, 554.0, 553.9, 553.1, 571.1, 572.6, 554.0, 561.6, 564.1, 569.3, 566.4, 566.6, 564.7, 564.1, 565.0, 563.1, 563.9, 578.1, 578.1, 576.0, 568.5, 568.6, 562.6, 579.9, 579.9, 564.8, 565.8, 566.2, 580.8, 565.5, 568.7, 568.7, 568.7, 561.7, 560.6, 560.8, 559.8, 560.5, 560.2, 558.3, 558.3, 562.9, 565.9, 557.5, 557.8, 557.8, 558.0, 558.8]
```

### `export messages json`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 5.18
    y-axis "MiB" 0 --> 300
    line [19.9, 19.9, 24.3, 24.3, 43.1, 60.3, 64.8, 113.8, 144.0, 168.0, 181.3, 189.0, 185.8, 172.3, 177.5, 178.5, 186.3, 211.4, 224.2, 225.2, 226.8, 229.8, 230.1, 230.1, 209.5, 212.0, 213.4, 214.2, 217.5, 220.3, 222.1, 223.4, 225.5, 225.7, 225.8, 216.4, 224.4, 225.9, 227.7, 227.9, 228.7, 234.3, 235.2, 235.8, 237.5, 237.7, 237.9, 231.3, 232.1, 234.8, 235.8, 237.2, 238.0, 240.1, 242.5, 242.4, 242.4, 242.4, 242.4, 242.4]
```

### `export messages chatlab`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 4.584
    y-axis "MiB" 0 --> 300
    line [19.9, 19.9, 20.9, 20.9, 26.5, 26.5, 38.7, 54.9, 69.3, 80.5, 114.7, 141.8, 153.7, 154.1, 164.0, 169.4, 175.2, 176.1, 180.9, 181.9, 184.9, 205.4, 213.5, 216.3, 218.4, 223.4, 224.0, 209.1, 213.5, 217.0, 224.2, 227.7, 229.8, 232.3, 232.5, 232.8, 232.9, 233.3, 233.4, 233.5, 232.3, 233.2, 234.1, 234.3, 234.4, 234.7, 235.7, 236.2, 238.3, 240.7, 242.7, 245.3, 246.4, 248.5, 251.8, 253.2, 254.5, 259.2, 259.3, 259.3]
```

### `export messages excel`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 6.37
    y-axis "MiB" 0 --> 300
    line [19.9, 20.5, 24.3, 47.3, 61.9, 77.3, 134.4, 152.9, 188.8, 195.0, 174.4, 179.5, 189.5, 242.2, 263.6, 265.2, 267.5, 270.9, 274.7, 277.5, 280.3, 248.5, 251.0, 257.1, 258.1, 259.0, 260.5, 261.1, 261.4, 261.6, 243.8, 246.5, 247.0, 250.1, 251.5, 251.5, 251.6, 255.5, 255.8, 255.8, 252.2, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 252.6, 168.1, 168.7, 169.2, 170.1, 170.2]
```

### `export messages html`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 3.922
    y-axis "MiB" 0 --> 300
    line [19.9, 19.9, 19.9, 24.3, 24.3, 44.9, 51.9, 62.0, 69.2, 93.3, 116.0, 147.3, 165.6, 180.7, 188.1, 191.4, 165.3, 166.0, 166.6, 171.5, 172.1, 191.4, 201.3, 210.1, 216.1, 219.4, 221.7, 225.5, 228.2, 231.4, 235.8, 237.3, 236.4, 236.8, 237.1, 240.7, 241.2, 242.2, 244.9, 247.1, 247.3, 248.0, 248.0, 248.7, 249.9, 250.6, 252.6, 249.5, 252.5, 256.0, 259.2, 261.8, 264.6, 266.4, 268.2, 268.5, 269.7, 271.1, 271.1, 271.0]
```

### `export messages txt`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 3.861
    y-axis "MiB" 0 --> 300
    line [19.9, 19.9, 19.9, 24.1, 24.1, 42.7, 60.8, 68.3, 73.2, 91.3, 117.4, 142.5, 158.5, 170.2, 192.5, 196.2, 161.4, 162.0, 162.8, 167.5, 168.3, 177.7, 198.0, 205.4, 212.0, 219.9, 224.0, 227.6, 230.1, 233.2, 237.2, 238.6, 236.5, 241.7, 244.5, 248.5, 250.9, 251.4, 258.7, 259.5, 259.6, 259.7, 260.0, 260.0, 260.2, 260.5, 260.6, 245.5, 247.0, 247.2, 249.0, 248.9, 254.5, 254.9, 256.3, 256.4, 256.4, 257.1, 258.8, 258.7]
```

### `report annual`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 3.649
    y-axis "MiB" 0 --> 150
    line [19.3, 19.3, 19.3, 26.2, 26.2, 38.4, 47.3, 54.9, 63.8, 69.1, 78.1, 83.9, 89.0, 96.3, 103.0, 110.8, 117.4, 117.1, 121.4, 123.7, 123.7, 124.2, 124.8, 124.8, 125.4, 126.5, 126.5, 126.5, 131.4, 131.4, 131.4, 131.4, 131.4, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 128.8, 130.2, 128.0, 128.1, 130.0, 131.6, 132.7, 132.7, 132.7, 132.7, 132.7, 132.7]
```

### `analytics overall`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 1.112
    y-axis "MiB" 0 --> 150
    line [1.8, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 26.2, 26.2, 26.2, 26.2, 26.2, 26.2, 26.2, 26.2, 26.2, 26.2, 40.3, 43.6, 48.2, 48.2, 51.7, 51.7, 54.8, 59.1, 63.5, 63.5, 65.8, 68.6, 68.6, 71.2, 75.6, 77.5, 80.0, 82.1, 84.0, 85.8, 85.8, 87.9, 90.2, 94.4, 95.6, 97.7, 97.7, 100.5, 103.8, 103.8, 106.9, 106.9, 111.1, 113.9, 113.9, 108.3, 108.3, 111.8, 115.2, 115.2, 118.3]
```

### `group members`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 1.331
    y-axis "MiB" 0 --> 300
    line [1.8, 19.9, 19.9, 19.9, 19.9, 19.9, 19.9, 19.9, 20.9, 20.9, 20.9, 20.9, 20.9, 20.9, 20.9, 31.7, 31.7, 31.7, 31.7, 31.7, 31.7, 45.8, 50.4, 56.8, 63.3, 67.0, 67.0, 72.9, 82.1, 88.3, 95.8, 102.2, 109.3, 115.4, 121.8, 121.8, 132.2, 138.6, 144.7, 150.6, 157.0, 162.5, 168.6, 181.1, 187.8, 182.2, 188.6, 195.0, 201.4, 201.9, 201.9, 201.9, 198.0, 199.2, 199.2, 200.9, 201.9, 201.9, 201.9, 110.3]
```

### `group ranking`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 1.301
    y-axis "MiB" 0 --> 300
    line [1.8, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 38.6, 43.7, 50.3, 56.9, 59.5, 61.4, 68.7, 75.5, 82.5, 88.6, 96.1, 96.1, 103.3, 109.2, 115.5, 121.7, 130.2, 130.2, 139.2, 148.5, 157.0, 163.6, 169.8, 175.9, 182.2, 188.6, 195.0, 201.1, 206.7, 206.7, 206.7, 202.0, 203.1, 204.4, 205.5, 206.8, 206.8, 206.8, 206.8, 193.1, 193.1, 193.1, 193.1, 193.1, 193.1, 117.9]
```

### `chat export-stats`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 1.282
    y-axis "MiB" 0 --> 300
    line [1.8, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 24.2, 24.2, 24.2, 24.2, 24.2, 24.2, 24.2, 39.8, 48.1, 48.1, 56.5, 61.4, 67.0, 73.6, 73.6, 82.1, 87.3, 91.7, 97.7, 103.0, 103.0, 112.7, 120.8, 126.6, 133.5, 133.5, 142.1, 150.5, 156.4, 163.0, 168.8, 175.4, 175.4, 183.8, 192.2, 192.2, 199.8, 200.3, 200.6, 200.8, 192.4, 192.6, 192.9, 193.2, 193.2, 193.5, 193.5, 193.5, 193.5, 193.5, 193.5, 76.1]
```

### `chat date-counts`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.962
    y-axis "MiB" 0 --> 300
    line [1.8, 1.8, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 32.1, 32.1, 40.6, 40.6, 50.4, 55.0, 55.0, 61.5, 67.8, 74.0, 74.0, 80.6, 86.7, 93.1, 93.1, 99.0, 107.7, 107.7, 114.2, 120.8, 126.9, 126.9, 133.1, 139.4, 145.4, 145.4, 151.9, 155.9, 162.3, 162.3, 169.8, 169.8, 178.8, 178.8, 188.2, 188.2, 197.4, 197.4, 202.3, 107.1]
```

### `chat search`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 2.758
    y-axis "MiB" 0 --> 400
    line [1.8, 19.4, 19.4, 19.4, 24.1, 24.1, 24.1, 37.6, 43.8, 54.5, 58.2, 60.0, 70.9, 81.4, 97.3, 103.9, 119.9, 130.8, 141.4, 152.0, 162.5, 172.7, 185.7, 190.8, 192.3, 194.9, 196.2, 199.3, 201.4, 202.4, 204.4, 207.0, 209.0, 210.8, 212.9, 215.0, 217.3, 218.8, 220.6, 224.1, 226.9, 233.1, 240.6, 243.4, 250.8, 253.6, 258.0, 265.4, 268.8, 273.5, 278.3, 283.3, 287.5, 292.6, 297.6, 304.4, 309.9, 314.3, 318.9, 320.9]
```

### `report dual`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.699
    y-axis "MiB" 0 --> 60
    line [1.8, 1.8, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 19.4, 21.0, 21.0, 21.0, 21.0, 21.0, 21.0, 21.0, 21.0, 21.0, 21.0, 21.8, 21.8, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 25.5, 41.9, 41.9, 41.9, 51.3, 51.3, 53.1, 53.1, 55.2, 56.2, 56.2, 56.7, 56.7, 57.1, 57.1, 57.5, 57.5, 57.5, 57.5, 57.5, 8.6]
```

### `chat messages first page`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.31
    y-axis "MiB" 0 --> 40
    line [1.7, 1.7, 1.7, 1.7, 1.7, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 19.3, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 24.1, 33.8]
```

### `chat messages offset`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#dc2626"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 2.828
    y-axis "MiB" 0 --> 1000
    line [1.8, 19.4, 19.4, 20.3, 24.1, 24.1, 24.1, 44.7, 52.3, 57.6, 64.3, 69.0, 72.6, 76.2, 80.3, 83.8, 88.5, 110.6, 134.4, 153.8, 166.7, 173.4, 221.8, 227.8, 235.3, 245.0, 252.0, 263.4, 276.7, 290.1, 305.5, 330.4, 345.1, 355.3, 382.3, 394.7, 417.1, 429.0, 451.8, 463.1, 482.3, 501.2, 520.8, 535.7, 551.1, 567.3, 585.1, 600.6, 622.2, 633.6, 656.4, 667.9, 756.8, 829.4, 939.9, 957.6, 957.6, 957.6, 684.1, 267.0]
```
