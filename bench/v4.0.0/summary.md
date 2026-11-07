# 基准测试：base 与 head

- base：`v3.0.1`（`afd428718d`），weflow 4.3.0
- head：`v4.0.0`（`4e8844c139`），weflow 4.3.0
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 5 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 24.4 MiB | 24.3 MiB | 1.00× |

## 命令

**有运行失败**（输出见 `logs/`，不计入中位数）：`export media images`（head 5 次），`export messages json with images`（head 5 次），`export messages json`（head 5 次），`export messages chatlab`（head 5 次），`export messages excel`（head 5 次），`export messages html`（head 5 次），`export messages txt`（head 5 次），`report annual`（head 5 次），`analytics overall`（head 5 次），`group members`（head 5 次），`group ranking`（head 5 次），`chat export-stats`（head 5 次），`chat date-counts`（head 5 次），`chat search`（head 5 次），`report dual`（head 5 次），`chat messages first page`（head 5 次），`chat messages offset`（head 5 次）

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 36,727（35,834–39,354） | - | - |
| `export messages json with images` | 40,252（39,043–42,564） | - | - |
| `export messages json` | 4,325（3,807–4,553） | - | - |
| `export messages chatlab` | 3,944（3,407–3,997） | - | - |
| `export messages excel` | 4,786（4,646–4,917） | - | - |
| `export messages html` | 3,221（3,167–3,291） | - | - |
| `export messages txt` | 3,104（3,038–3,187） | - | - |
| `report annual` | 2,575（2,560–2,594） | - | - |
| `analytics overall` | 701（680–710） | - | - |
| `group members` | 801（773–837） | - | - |
| `group ranking` | 782（768–785） | - | - |
| `chat export-stats` | 790（788–814） | - | - |
| `chat date-counts` | 565（559–577） | - | - |
| `chat search` | 1,758（1,745–1,800） | - | - |
| `report dual` | 419（404–420） | - | - |
| `chat messages first page` | 197（182–207） | - | - |
| `chat messages offset` | 2,234（2,201–2,288） | - | - |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 1,605 | - | - |
| `export messages json with images` | 644 | - | - |
| `export messages json` | 732 | - | - |
| `export messages chatlab` | 734 | - | - |
| `export messages excel` | 637 | - | - |
| `export messages html` | 627 | - | - |
| `export messages txt` | 631 | - | - |
| `report annual` | 12 | - | - |
| `analytics overall` | 91 | - | - |
| `report dual` | 13 | - | - |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 64,828 | - | - |
| `export messages json with images` | 69,094 | - | - |
| `export messages json` | 9,203 | - | - |
| `export messages chatlab` | 7,922 | - | - |
| `export messages excel` | 8,562 | - | - |
| `export messages html` | 6,984 | - | - |
| `export messages txt` | 6,359 | - | - |
| `report annual` | 2,406 | - | - |
| `analytics overall` | 594 | - | - |
| `group members` | 719 | - | - |
| `group ranking` | 672 | - | - |
| `chat export-stats` | 719 | - | - |
| `chat date-counts` | 516 | - | - |
| `chat search` | 1,578 | - | - |
| `report dual` | 438 | - | - |
| `chat messages first page` | 203 | - | - |
| `chat messages offset` | 1,844 | - | - |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 61,203 | - | - |
| `export messages json with images` | 61,641 | - | - |
| `export messages json` | 594 | - | - |
| `export messages chatlab` | 453 | - | - |
| `export messages excel` | 609 | - | - |
| `export messages html` | 469 | - | - |
| `export messages txt` | 391 | - | - |
| `report annual` | 250 | - | - |
| `analytics overall` | 172 | - | - |
| `group members` | 156 | - | - |
| `group ranking` | 188 | - | - |
| `chat export-stats` | 156 | - | - |
| `chat date-counts` | 109 | - | - |
| `chat search` | 266 | - | - |
| `report dual` | 31 | - | - |
| `chat messages first page` | 47 | - | - |
| `chat messages offset` | 484 | - | - |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 278 | - | - |
| `export messages json with images` | 522 | - | - |
| `export messages json` | 160 | - | - |
| `export messages chatlab` | 154 | - | - |
| `export messages excel` | 176 | - | - |
| `export messages html` | 151 | - | - |
| `export messages txt` | 148 | - | - |
| `report annual` | 129 | - | - |
| `analytics overall` | 116 | - | - |
| `group members` | 201 | - | - |
| `group ranking` | 198 | - | - |
| `chat export-stats` | 201 | - | - |
| `chat date-counts` | 196 | - | - |
| `chat search` | 321 | - | - |
| `report dual` | 45.6 | - | - |
| `chat messages first page` | 31.8 | - | - |
| `chat messages offset` | 968 | - | - |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 269 | - | - |
| `export messages json with images` | 546 | - | - |
| `export messages json` | 152 | - | - |
| `export messages chatlab` | 146 | - | - |
| `export messages excel` | 172 | - | - |
| `export messages html` | 142 | - | - |
| `export messages txt` | 139 | - | - |
| `report annual` | 115 | - | - |
| `analytics overall` | 104 | - | - |
| `group members` | 196 | - | - |
| `group ranking` | 194 | - | - |
| `chat export-stats` | 190 | - | - |
| `chat date-counts` | 189 | - | - |
| `chat search` | 329 | - | - |
| `report dual` | 32.7 | - | - |
| `chat messages first page` | 19.5 | - | - |
| `chat messages offset` | 978 | - | - |

### 峰值线程数

每 20 ms 采样一次主进程得到的最大值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 12 | - | - |
| `export messages json with images` | 13 | - | - |
| `export messages json` | 13 | - | - |
| `export messages chatlab` | 13 | - | - |
| `export messages excel` | 13 | - | - |
| `export messages html` | 13 | - | - |
| `export messages txt` | 13 | - | - |
| `report annual` | 10 | - | - |
| `analytics overall` | 10 | - | - |
| `group members` | 10 | - | - |
| `group ranking` | 10 | - | - |
| `chat export-stats` | 10 | - | - |
| `chat date-counts` | 10 | - | - |
| `chat search` | 10 | - | - |
| `report dual` | 10 | - | - |
| `chat messages first page` | 10 | - | - |
| `chat messages offset` | 10 | - | - |

### 读取（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 900 | - | - |
| `export messages json with images` | 1,325 | - | - |
| `export messages json` | 633 | - | - |
| `export messages chatlab` | 467 | - | - |
| `export messages excel` | 504 | - | - |
| `export messages html` | 479 | - | - |
| `export messages txt` | 417 | - | - |
| `report annual` | 146 | - | - |
| `analytics overall` | 142 | - | - |
| `group members` | 207 | - | - |
| `group ranking` | 207 | - | - |
| `chat export-stats` | 232 | - | - |
| `chat date-counts` | 206 | - | - |
| `chat search` | 524 | - | - |
| `report dual` | 42.7 | - | - |
| `chat messages first page` | 28.4 | - | - |
| `chat messages offset` | 429 | - | - |

### 写入（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 952 | - | - |
| `export messages json with images` | 1,381 | - | - |
| `export messages json` | 431 | - | - |
| `export messages chatlab` | 99.1 | - | - |
| `export messages excel` | 98.2 | - | - |
| `export messages html` | 122 | - | - |
| `export messages txt` | 14.6 | - | - |
| `report annual` | 0.0 | - | - |
| `analytics overall` | 0.0 | - | - |
| `group members` | 0.1 | - | - |
| `group ranking` | 0.0 | - | - |
| `chat export-stats` | 0.0 | - | - |
| `chat date-counts` | 0.0 | - | - |
| `chat search` | 0.1 | - | - |
| `report dual` | 0.0 | - | - |
| `chat messages first page` | 0.1 | - | - |
| `chat messages offset` | 36.5 | - | - |

### 页错误

进程及其子进程，包括软页错误。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 10,553,166 | - | - |
| `export messages json with images` | 10,665,434 | - | - |
| `export messages json` | 128,748 | - | - |
| `export messages chatlab` | 132,487 | - | - |
| `export messages excel` | 182,551 | - | - |
| `export messages html` | 140,614 | - | - |
| `export messages txt` | 141,857 | - | - |
| `report annual` | 43,239 | - | - |
| `analytics overall` | 38,067 | - | - |
| `group members` | 63,250 | - | - |
| `group ranking` | 61,341 | - | - |
| `chat export-stats` | 56,360 | - | - |
| `chat date-counts` | 57,379 | - | - |
| `chat search` | 94,800 | - | - |
| `report dual` | 27,127 | - | - |
| `chat messages first page` | 13,770 | - | - |
| `chat messages offset` | 259,868 | - | - |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | 4,969 | - | - |
| `export messages json` | 46,244 | - | - |
| `export messages chatlab` | 50,712 | - | - |
| `export messages excel` | 41,785 | - | - |
| `export messages html` | 62,094 | - | - |
| `export messages txt` | 64,426 | - | - |
| `report annual` | 126,532 | - | - |
| `group members` | 249,585 | - | - |
| `group ranking` | 255,645 | - | - |
| `chat export-stats` | 253,052 | - | - |
| `chat date-counts` | 353,995 | - | - |
| `report dual` | 23,865 | - | - |
| `chat messages first page` | 254 | - | - |
| `chat messages offset` | 22 | - | - |

## `weflow serve` 持续负载

没有完成：base：weflow serve exited (2) before it listened: error: unexpected argument '--api-only' found   tip: a similar argument exists: '--api-token' Usage: weflow-base.exe serve --port <PORT> --api-token <API_TOKEN> For more information, try '--help'.；head：weflow serve exited (2) before it listened: error: unexpected argument '--api-only' found   tip: a similar argument exists: '--api-token' Usage: weflow-head.exe serve --port <PORT> --api-token <API_TOKEN> For more information, try '--help'.。完整信息见 results.json 的 `load_failed`。

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 1，head 1。非 0 表示它在自己的合成账号上有检查没通过（旧版本缺少某个路由时会这样），计时照常保留，原因见 `logs/serve-bench-*.txt`。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。base 没有计时结果：它的 `weflow serve` 没有启动（v5.0.0 之前的版本没有现在的命令 API），见 `logs/serve-bench-base.txt`。head 没有计时结果：它的 `weflow serve` 没有启动（v5.0.0 之前的版本没有现在的命令 API），见 `logs/serve-bench-head.txt`。
## 内存曲线

主进程的工作集，每 20 ms 采样一次（持续负载每 0.5 s 一次）。命令取耗时居中的那次运行。蓝线 base，红线 head；先结束的版本在退出后记为 0。

### `export media images`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 36.706
    y-axis "MiB" 0 --> 300
    line [76.9, 248.4, 276.5, 211.1, 161.4, 162.3, 165.6, 166.7, 166.7, 167.3, 169.6, 172.1, 171.3, 170.9, 172.5, 175.7, 180.1, 179.4, 180.6, 183.2, 183.3, 184.1, 184.6, 184.3, 184.6, 182.1, 182.7, 183.5, 184.6, 185.9, 186.7, 186.7, 186.5, 186.0, 187.2, 189.0, 188.6, 189.6, 191.8, 195.2, 193.4, 192.8, 193.2, 195.3, 194.5, 196.9, 197.2, 197.1, 198.8, 198.2, 199.1, 200.1, 199.4, 199.1, 200.2, 199.5, 199.6, 199.6, 201.7, 199.4]
```

### `export messages json with images`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 40.245
    y-axis "MiB" 0 --> 600
    line [66.8, 183.3, 309.2, 438.2, 522.3, 479.2, 480.7, 481.4, 481.3, 481.1, 489.3, 480.7, 481.1, 481.3, 481.8, 483.5, 483.4, 484.7, 483.9, 483.9, 483.9, 495.4, 484.9, 484.6, 485.6, 484.6, 483.4, 483.5, 485.7, 485.7, 489.1, 485.6, 485.5, 484.7, 485.4, 486.6, 488.2, 488.7, 489.0, 488.8, 489.4, 489.6, 489.0, 489.1, 484.2, 484.2, 489.6, 489.2, 489.7, 489.7, 489.7, 488.3, 488.7, 488.8, 488.3, 484.4, 483.2, 483.2, 483.2, 483.2]
```

### `export messages json`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 4.32
    y-axis "MiB" 0 --> 200
    line [14.6, 14.6, 22.8, 22.8, 37.1, 66.0, 68.4, 69.7, 70.7, 71.8, 98.0, 120.1, 134.1, 137.9, 146.9, 149.8, 151.7, 153.5, 148.5, 151.1, 147.8, 149.4, 154.6, 158.1, 151.1, 150.9, 150.7, 152.2, 155.8, 151.3, 152.6, 154.3, 155.2, 156.1, 157.3, 149.5, 152.8, 153.5, 156.9, 157.0, 153.6, 158.2, 157.5, 154.1, 159.3, 158.2, 151.5, 157.0, 158.8, 159.9, 153.9, 148.8, 141.4, 143.2, 147.9, 151.3, 153.6, 116.5, 102.3, 102.3]
```

### `export messages chatlab`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 3.937
    y-axis "MiB" 0 --> 150
    line [14.6, 14.6, 22.9, 22.9, 29.6, 58.3, 68.7, 69.5, 70.8, 71.7, 72.9, 103.0, 122.0, 122.2, 122.9, 127.6, 131.2, 134.7, 135.4, 134.7, 136.2, 131.3, 127.5, 127.4, 133.5, 132.8, 137.1, 136.4, 132.3, 132.8, 138.4, 137.7, 129.6, 132.9, 136.9, 131.9, 132.3, 137.1, 130.6, 131.6, 138.9, 131.1, 132.2, 132.5, 133.6, 133.1, 137.7, 139.1, 139.8, 130.6, 132.9, 137.4, 131.7, 133.2, 134.2, 136.2, 140.6, 133.9, 122.4, 103.5]
```

### `export messages excel`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 4.76
    y-axis "MiB" 0 --> 200
    line [14.6, 22.9, 22.9, 40.3, 68.0, 69.4, 70.7, 72.2, 113.8, 137.1, 153.3, 166.7, 176.2, 176.3, 156.4, 139.3, 136.0, 126.3, 130.9, 130.2, 130.9, 132.0, 132.7, 130.7, 133.8, 134.9, 133.2, 132.2, 134.3, 132.4, 134.2, 134.7, 131.2, 130.7, 132.3, 133.0, 136.3, 133.0, 134.1, 136.0, 137.1, 136.8, 133.6, 140.2, 143.0, 124.7, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0, 104.0]
```

### `export messages html`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 3.218
    y-axis "MiB" 0 --> 200
    line [14.7, 14.7, 23.0, 23.0, 30.7, 49.1, 68.3, 69.2, 70.0, 71.3, 72.2, 78.4, 111.0, 126.8, 131.3, 135.3, 137.6, 140.8, 139.6, 141.4, 128.2, 127.3, 127.7, 126.2, 129.7, 130.3, 130.5, 131.3, 131.5, 133.3, 131.3, 132.3, 133.2, 133.6, 133.6, 132.8, 134.5, 131.7, 133.8, 139.1, 141.9, 142.6, 142.5, 143.0, 148.4, 146.7, 148.3, 146.6, 149.7, 152.6, 152.4, 153.4, 152.7, 148.5, 137.3, 139.1, 141.5, 144.1, 137.0, 107.1]
```

### `export messages txt`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 3.09
    y-axis "MiB" 0 --> 200
    line [14.7, 14.7, 22.9, 22.9, 23.1, 41.4, 67.9, 68.6, 69.6, 70.6, 71.5, 72.3, 94.9, 109.3, 117.6, 120.4, 124.3, 127.2, 125.6, 128.9, 137.3, 135.2, 134.2, 136.9, 125.3, 127.7, 128.0, 130.4, 132.1, 130.0, 133.0, 129.9, 131.6, 131.6, 129.6, 131.6, 132.4, 131.8, 134.4, 129.6, 130.1, 130.5, 130.9, 131.5, 132.0, 136.0, 139.5, 145.6, 148.2, 148.2, 148.6, 149.6, 152.5, 145.5, 138.6, 136.7, 137.8, 141.1, 141.2, 109.1]
```

### `report annual`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 2.568
    y-axis "MiB" 0 --> 150
    line [1.8, 14.5, 15.7, 23.4, 23.4, 37.6, 47.4, 51.9, 62.4, 66.4, 75.2, 84.8, 89.0, 96.1, 104.5, 108.6, 116.2, 116.3, 117.8, 118.0, 117.2, 118.0, 117.4, 118.1, 118.2, 118.3, 118.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.5, 123.6, 123.7, 123.7, 123.7, 123.8, 125.4, 126.8, 128.0, 128.0, 128.0, 128.0, 128.0, 128.0]
```

### `analytics overall`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.682
    y-axis "MiB" 0 --> 150
    line [1.8, 1.8, 14.4, 14.4, 14.4, 14.4, 14.4, 14.4, 23.5, 23.5, 23.5, 23.5, 23.5, 23.5, 23.5, 23.5, 23.6, 23.6, 31.3, 31.3, 36.8, 36.8, 40.8, 40.8, 44.0, 44.0, 50.1, 50.1, 52.5, 55.4, 55.4, 60.4, 60.4, 64.1, 64.1, 68.3, 68.3, 70.7, 70.7, 74.1, 74.1, 78.0, 78.0, 83.5, 83.5, 86.7, 86.7, 90.0, 90.0, 94.1, 94.1, 94.1, 100.3, 100.3, 105.0, 105.0, 105.0, 111.0, 111.0, 114.3]
```

### `group members`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.787
    y-axis "MiB" 0 --> 200
    line [1.8, 1.8, 15.0, 15.0, 15.0, 15.0, 15.0, 15.9, 15.9, 15.9, 15.9, 15.9, 15.9, 15.9, 25.7, 25.7, 25.7, 25.7, 25.7, 25.7, 25.7, 32.0, 45.0, 45.0, 45.0, 52.8, 63.7, 63.7, 74.2, 74.2, 74.2, 91.1, 101.5, 101.5, 111.9, 111.9, 111.9, 128.3, 138.7, 138.7, 150.7, 150.7, 160.8, 170.9, 170.9, 180.7, 180.7, 190.5, 198.2, 198.2, 198.3, 198.3, 193.6, 195.3, 195.3, 197.0, 197.0, 198.2, 198.2, 194.7]
```

### `group ranking`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.764
    y-axis "MiB" 0 --> 200
    line [1.8, 1.8, 14.5, 14.5, 14.5, 14.5, 14.5, 22.6, 22.6, 22.6, 22.6, 22.6, 22.6, 22.6, 31.4, 31.4, 42.8, 48.3, 48.3, 53.4, 53.4, 62.0, 62.0, 62.0, 78.9, 88.7, 88.7, 100.6, 100.6, 100.6, 116.0, 116.0, 130.6, 130.6, 141.0, 151.4, 151.4, 161.8, 161.8, 172.1, 182.2, 182.2, 192.5, 192.5, 195.5, 195.5, 191.2, 191.2, 191.2, 193.6, 193.6, 195.4, 195.4, 195.4, 195.4, 189.9, 189.9, 189.9, 189.9, 189.9]
```

### `chat export-stats`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.763
    y-axis "MiB" 0 --> 200
    line [1.8, 1.8, 14.3, 14.3, 14.3, 14.3, 14.3, 14.3, 22.5, 22.5, 22.5, 22.5, 22.5, 24.0, 24.0, 35.4, 45.1, 45.1, 45.1, 56.3, 56.3, 65.8, 75.5, 75.5, 85.8, 85.8, 98.1, 98.1, 98.1, 111.7, 120.9, 120.9, 130.0, 130.0, 139.7, 139.7, 148.1, 157.3, 157.3, 157.3, 170.6, 180.0, 180.0, 189.0, 189.0, 189.0, 189.5, 189.5, 190.0, 190.0, 190.0, 190.5, 190.8, 190.8, 191.1, 191.1, 191.1, 191.1, 191.1, 191.1]
```

### `chat date-counts`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.554
    y-axis "MiB" 0 --> 200
    line [1.8, 1.8, 1.8, 14.3, 14.3, 14.3, 14.3, 14.3, 14.3, 14.3, 22.4, 22.4, 22.4, 22.4, 22.4, 22.4, 22.4, 22.4, 22.4, 28.8, 28.8, 37.3, 37.3, 37.3, 48.1, 48.1, 56.8, 56.8, 56.8, 56.8, 73.4, 73.4, 84.2, 84.2, 84.2, 96.3, 96.3, 106.8, 106.8, 117.4, 117.4, 117.4, 127.3, 127.3, 137.4, 137.4, 148.5, 148.5, 148.5, 148.5, 163.2, 163.2, 172.8, 172.8, 182.6, 182.6, 182.6, 192.8, 192.8, 177.6]
```

### `chat search`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 1.735
    y-axis "MiB" 0 --> 400
    line [1.8, 15.2, 15.2, 15.2, 22.6, 22.6, 26.7, 33.6, 40.8, 49.4, 52.8, 60.3, 69.0, 77.4, 87.2, 99.4, 120.7, 129.5, 138.4, 150.6, 162.0, 174.3, 186.6, 188.1, 191.1, 192.7, 194.1, 196.8, 198.5, 200.0, 202.6, 204.0, 205.6, 208.6, 209.9, 211.6, 214.5, 215.8, 216.7, 218.4, 223.0, 229.8, 233.2, 238.5, 240.8, 248.3, 251.7, 257.3, 263.9, 270.7, 274.7, 278.8, 287.6, 290.6, 294.7, 302.4, 306.2, 309.0, 317.2, 320.8]
```

### `report dual`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.407
    y-axis "MiB" 0 --> 60
    line [1.8, 1.8, 1.8, 1.8, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 14.5, 15.7, 15.7, 15.7, 15.7, 15.7, 15.7, 15.7, 15.7, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 24.0, 34.3, 34.3, 34.3, 38.9, 38.9, 38.9, 42.5, 42.5, 42.5, 42.1, 42.1, 42.1, 43.0, 43.0, 43.0, 43.0, 43.5, 43.5, 43.5, 45.3]
```

### `chat messages first page`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 0.167
    y-axis "MiB" 0 --> 30
    line [1.8, 1.8, 1.8, 1.8, 1.8, 1.8, 1.8, 1.8, 1.8, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 15.2, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3, 23.3]
```

### `chat messages offset`

```mermaid
%%{init: {"xyChart": {"width": 720, "height": 260}, "themeVariables": {"xyChart": {"plotColorPalette": "#2563eb"}}}}%%
xychart-beta
    x-axis "秒" 0 --> 2.212
    y-axis "MiB" 0 --> 1000
    line [1.8, 14.3, 14.3, 22.4, 22.4, 27.3, 38.2, 45.5, 53.4, 56.7, 63.2, 65.6, 72.2, 80.6, 99.2, 124.9, 150.4, 163.9, 211.5, 217.1, 231.8, 238.2, 252.9, 273.1, 283.4, 304.3, 316.8, 350.6, 368.6, 387.2, 405.1, 435.9, 448.1, 480.0, 492.9, 517.7, 542.1, 554.2, 579.0, 604.0, 629.2, 646.5, 664.4, 688.3, 737.6, 833.4, 881.6, 930.4, 964.8, 960.9, 960.9, 962.5, 962.5, 890.6, 811.1, 658.9, 484.0, 404.9, 297.4, 175.6]
```
