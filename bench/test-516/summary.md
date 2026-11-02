# 基准测试：base 与 head

- base：`v5.0.1`（`6115746c5f`），weflow 5.0.1
- head：`HEAD`（`4ddda8ce4f`），weflow 5.0.1-183-g4ddda8ce
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 2 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 28.7 MiB | 30.7 MiB | 1.07× |

## 命令

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 27,286（26,388–28,184） | 26,493（26,255–26,732） | 0.97× |
| `export messages json` | 5,281（5,279–5,283） | 5,252（5,232–5,272） | 0.99× |
| `export messages excel` | 5,628（5,624–5,633） | 5,652（5,600–5,703） | 1.00× |
| `export messages html` | 4,100（4,079–4,120） | 4,068（4,052–4,085） | 0.99× |
| `export messages txt` | 3,962（3,959–3,965） | 4,033（3,910–4,155） | 1.02× |
| `report annual` | 2,678（2,566–2,791） | 2,727（2,626–2,829） | 1.02× |
| `analytics overall` | 1,259（1,207–1,310） | 1,217（1,170–1,264） | 0.97× |
| `group members` | 1,257（1,205–1,308） | 1,460（1,223–1,698） | 1.16× ▲，范围重叠 |
| `group ranking` | 1,248（1,204–1,292） | 1,250（1,225–1,275） | 1.00× |
| `chat export-stats` | 1,142（1,139–1,144） | 1,172（1,151–1,193） | 1.03× |
| `chat date-counts` | 930（929–932） | 925（923–927） | 0.99× |
| `chat search` | 627（627–628） | 608（598–618） | 0.97× |
| `report dual` | 496（471–521） | 497（495–499） | 1.00× |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 2,589 | 2,337 | 0.90× |
| `export messages json` | 530 | 504 | 0.95× |
| `export messages excel` | 516 | 508 | 0.98× |
| `export messages html` | 514 | 509 | 0.99× |
| `export messages txt` | 503 | 498 | 0.99× |
| `report annual` | 18 | 19 | 1.00× |
| `analytics overall` | 203 | 169 | 0.83× ▼ |
| `report dual` | 20 | 23 | 1.16× |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 17,969 | 17,969 | 1.00× |
| `export messages json` | 12,664 | 12,195 | 0.96× |
| `export messages excel` | 12,516 | 12,344 | 0.99× |
| `export messages html` | 11,820 | 11,539 | 0.98× |
| `export messages txt` | 11,094 | 11,062 | 1.00× |
| `report annual` | 2,141 | 2,172 | 1.01× |
| `analytics overall` | 969 | 1,062 | 1.10× |
| `group members` | 1,055 | 1,133 | 1.07× |
| `group ranking` | 1,078 | 992 | 0.92× |
| `chat export-stats` | 977 | 922 | 0.94× |
| `chat date-counts` | 750 | 734 | 0.98× |
| `chat search` | 469 | 453 | 0.97× |
| `report dual` | 398 | 453 | 1.14× ▲ |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 35,898 | 36,312 | 1.01× |
| `export messages json` | 688 | 945 | 1.38× ▲ |
| `export messages excel` | 930 | 1,133 | 1.22× ▲ |
| `export messages html` | 883 | 953 | 1.08× |
| `export messages txt` | 766 | 727 | 0.95× |
| `report annual` | 508 | 508 | 1.00× |
| `analytics overall` | 258 | 125 | 0.48× ▼ |
| `group members` | 195 | 305 | 1.56× ▲ |
| `group ranking` | 141 | 242 | 1.72× ▲ |
| `chat export-stats` | 164 | 234 | 1.43× ▲ |
| `chat date-counts` | 180 | 172 | 0.96× |
| `chat search` | 156 | 141 | 0.90× |
| `report dual` | 94 | 39 | 0.42× ▼ |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 295 | 294 | 1.00× |
| `export messages json` | 267 | 265 | 0.99× |
| `export messages excel` | 288 | 295 | 1.03× |
| `export messages html` | 275 | 268 | 0.97× |
| `export messages txt` | 268 | 282 | 1.05× |
| `report annual` | 136 | 134 | 0.98× |
| `analytics overall` | 117 | 117 | 1.00× |
| `group members` | 205 | 203 | 0.99× |
| `group ranking` | 198 | 199 | 1.00× |
| `chat export-stats` | 198 | 198 | 1.00× |
| `chat date-counts` | 195 | 195 | 1.00× |
| `chat search` | 58.2 | 58.9 | 1.01× |
| `report dual` | 50.7 | 51.2 | 1.01× |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 438 | 438 | 1.00× |
| `export messages json` | 464 | 455 | 0.98× |
| `export messages excel` | 469 | 469 | 1.00× |
| `export messages html` | 454 | 458 | 1.01× |
| `export messages txt` | 455 | 467 | 1.03× |
| `report annual` | 282 | 276 | 0.98× |
| `analytics overall` | 244 | 245 | 1.01× |
| `group members` | 338 | 336 | 0.99× |
| `group ranking` | 328 | 328 | 1.00× |
| `chat export-stats` | 328 | 328 | 1.00× |
| `chat date-counts` | 324 | 324 | 1.00× |
| `chat search` | 186 | 187 | 1.00× |
| `report dual` | 189 | 190 | 1.00× |

### 峰值线程数

每 20 ms 采样一次主进程得到的最大值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 12 | 12 | 1.00× |
| `export messages json` | 11 | 11 | 1.00× |
| `export messages excel` | 11 | 12 | 1.05× |
| `export messages html` | 11 | 11 | 1.00× |
| `export messages txt` | 11 | 11 | 1.00× |
| `report annual` | 8.0 | 8.0 | 1.00× |
| `analytics overall` | 8.5 | 8.0 | 0.94× |
| `group members` | 8.0 | 8.0 | 1.00× |
| `group ranking` | 8.0 | 8.0 | 1.00× |
| `chat export-stats` | 8.0 | 8.0 | 1.00× |
| `chat date-counts` | 8.0 | 8.0 | 1.00× |
| `chat search` | 9.0 | 8.0 | 0.89× |
| `report dual` | 8.0 | 8.0 | 1.00× |

### 读取（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 847 | 847 | 1.00× |
| `export messages json` | 814 | 809 | 0.99× |
| `export messages excel` | 689 | 686 | 1.00× |
| `export messages html` | 663 | 665 | 1.00× |
| `export messages txt` | 605 | 604 | 1.00× |
| `report annual` | 132 | 132 | 1.00× |
| `analytics overall` | 128 | 128 | 1.00× |
| `group members` | 190 | 190 | 1.00× |
| `group ranking` | 190 | 190 | 1.00× |
| `chat export-stats` | 191 | 191 | 1.00× |
| `chat date-counts` | 190 | 190 | 1.00× |
| `chat search` | 93.4 | 93.4 | 1.00× |
| `report dual` | 31.8 | 31.8 | 1.00× |

### 写入（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 417 | 417 | 1.00× |
| `export messages json` | 432 | 432 | 1.00× |
| `export messages excel` | 98.3 | 98.3 | 1.00× |
| `export messages html` | 122 | 122 | 1.00× |
| `export messages txt` | 14.6 | 14.6 | 1.00× |
| `report annual` | 0.0 | 0.0 | 1.00× |
| `analytics overall` | 0.0 | 0.0 | 1.00× |
| `group members` | 0.1 | 0.1 | 1.00× |
| `group ranking` | 0.0 | 0.0 | 1.00× |
| `chat export-stats` | 0.0 | 0.0 | 1.00× |
| `chat date-counts` | 0.0 | 0.0 | 1.00× |
| `chat search` | 0.1 | 0.1 | 1.00× |
| `report dual` | 0.0 | 0.0 | 1.00× |

### 页错误

进程及其子进程，包括软页错误。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 141,474 | 139,262 | 0.98× |
| `export messages json` | 108,451 | 109,778 | 1.01× |
| `export messages excel` | 114,226 | 125,509 | 1.10× |
| `export messages html` | 105,559 | 105,306 | 1.00× |
| `export messages txt` | 104,998 | 114,932 | 1.09× |
| `report annual` | 37,438 | 36,880 | 0.99× |
| `analytics overall` | 31,082 | 30,993 | 1.00× |
| `group members` | 58,471 | 58,688 | 1.00× |
| `group ranking` | 56,610 | 56,662 | 1.00× |
| `chat export-stats` | 54,759 | 54,967 | 1.00× |
| `chat date-counts` | 52,423 | 52,406 | 1.00× |
| `chat search` | 15,108 | 15,288 | 1.01× |
| `report dual` | 21,680 | 22,005 | 1.01× |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json` | 37,873 | 38,079 | 1.01× |
| `export messages excel` | 35,534 | 35,387 | 1.00× |
| `export messages html` | 48,784 | 49,161 | 1.01× |
| `export messages txt` | 50,479 | 49,594 | 0.98× |
| `report annual` | 121,645 | 119,458 | 0.98× |
| `group members` | 159,165 | 136,955 | 0.86× ▼ |
| `group ranking` | 160,267 | 159,980 | 1.00× |
| `chat export-stats` | 175,177 | 170,650 | 0.97× |
| `chat date-counts` | 214,966 | 216,233 | 1.01× |
| `report dual` | 20,152 | 20,133 | 1.00× |

## `weflow serve` 持续负载

每个版本启动一个 `weflow serve`，用 4 个并发连续请求 30 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。

|  | base | head | head / base |
|---|---|---|---|
| 每秒请求数 | 1.1 | 20 | 18.00× ▲ |
| 错误数 | 0 | 0 | - |
| 延迟 p50 (ms) | 770 | 47 | 0.06× ▼ |
| 延迟 p95 (ms) | 20,307 | 1,079 | 0.05× ▼ |
| 延迟 p99 (ms) | 22,052 | 1,979 | 0.09× ▼ |
| 启动到开始监听 (ms) | 21 | 21 | 1.02× |
| 工作集，第 1 段中位数 (MiB) | 670 | 325 | 0.49× ▼ |
| 工作集，第 2 段中位数 (MiB) | 1,211 | 338 | 0.28× ▼ |
| 工作集，第 3 段中位数 (MiB) | 646 | 345 | 0.53× ▼ |
| 工作集，第 4 段中位数 (MiB) | 1,111 | 330 | 0.30× ▼ |
| 第 2 段到第 4 段的增长 (MiB) | -99 | -7.7 | - |
| 峰值工作集 (MiB) | 1,706 | 383 | 0.22× ▼ |
| 峰值线程数 | 13 | 13 | 1.00× |
| CPU 时间 (ms) | 45,562 | 47,594 | 1.04× |

各路由：请求次数，延迟 p50 / p95（ms）。

| 路由 | base | head | p50 head / base |
|---|---|---|---|
| `/api/chat/messages`，大群任意一页 | 13 次，6,171 / 10,766 | 198 次，164 / 407 | 0.03× ▼ |
| `/api/chat/messages`，其他会话第一页 | 9 次，7.8 / 7,243 | 129 次，5.0 / 139 | 0.63× |
| `/api/chat/sessions` | 3 次，39 / 43 | 95 次，37 / 266 | 0.95× |
| `/api/chat/search`，全部会话 | 2 次，22,052 / 22,052 | 50 次，1,332 / 2,488 | 0.06× ▼ |
| `/api/chat/date-counts` | 5 次，108 / 770 | 54 次，5.2 / 189 | 0.05× ▼ |
| `/api/chat/contacts` | 6 次，33 / 786 | 70 次，23 / 51 | 0.69× ▼ |

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 0，head 0。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。

| 路由 | base | head | head / base |
|---|---|---|---|
| `16 pages at once (all of them) [api]` | 154 | 51 | 0.33× ▼ |
| `16 pages at once (all of them) [sub]` | 84 | 81 | 0.97× |
| `20 images at once (all of them) [api]` | 18 | 19 | 1.02× |
| `20 images at once (all of them) [sub]` | 46 | 48 | 1.05× |
| `analytics overall (cached) [api]` | 0.5（状态 {404: 2}） | 2.8 | - |
| `analytics overall (cached) [sub]` | 23 | 24 | 1.04× |
| `analytics overall --force [api]` | 0.5（状态 {404: 1}） | 545 | - |
| `analytics overall --force [sub]` | 913 | 910 | 1.00× |
| `analytics rankings (cached) [api]` | 0.5（状态 {404: 2}） | 9.6 | - |
| `analytics rankings (cached) [sub]` | 32 | 33 | 1.02× |
| `analytics time (cached) [api]` | 0.5（状态 {404: 2}） | 3.0 | - |
| `analytics time (cached) [sub]` | 23 | 24 | 1.04× |
| `biz pay-records [api]` | 0.4（状态 {404: 2}） | 1.2 | - |
| `biz pay-records [sub]` | 36 | 36 | 1.01× |
| `cache list [api]` | 0.5（状态 {404: 2}） | 0.9 | - |
| `cache list [sub]` | 20 | 20 | 0.99× |
| `chat date-counts [api]` | 77 | 76 | 0.99× |
| `chat date-counts [sub]` | 672 | 689 | 1.02× |
| `chat dates [api]` | 196 | 184 | 0.94× |
| `chat dates [sub]` | 674 | 690 | 1.02× |
| `chat detail --fast [api]` | 2.0 | 1.8 | 0.87× |
| `chat detail --fast [sub]` | 90 | 93 | 1.04× |
| `chat resources, all conversations [api]` | 110 | 105 | 0.95× |
| `chat resources, all conversations [sub]` | 264 | 267 | 1.01× |
| `chat resources, all conversations, far page [api]` | 177 | 172 | 0.97× |
| `chat resources, all conversations, far page [sub]` | 365 | 361 | 0.99× |
| `chat resources, all conversations, file [api]` | 131 | 136 | 1.04× |
| `chat resources, all conversations, file [sub]` | 266 | 266 | 1.00× |
| `chat resources, all conversations, image [api]` | 63 | 62 | 0.98× |
| `chat resources, all conversations, image [sub]` | 130 | 133 | 1.02× |
| `chat resources, all conversations, video [api]` | 82 | 76 | 0.93× |
| `chat resources, all conversations, video [sub]` | 165 | 171 | 1.03× |
| `chat resources, all conversations, voice [api]` | 43 | 38 | 0.89× |
| `chat resources, all conversations, voice [sub]` | 121 | 122 | 1.00× |
| `chat resources, one conversation [api]` | 63 | 57 | 0.90× |
| `chat resources, one conversation [sub]` | 220 | 226 | 1.03× |
| `chat statuses of 60 conversations [api]` | 0.5（状态 {404: 2}） | 1.9 | - |
| `chat statuses of 60 conversations [sub]` | 23 | 25 | 1.07× |
| `chat tab-counts [api]` | 0.5（状态 {404: 2}） | 2.4 | - |
| `chat tab-counts [sub]` | 23 | 23 | 0.97× |
| `config profiles [api]` | 0.5（状态 {404: 2}） | 0.7 | - |
| `config profiles [sub]` | 20 | 20 | 1.01× |
| `contacts, all [api]` | 19 | 14 | 0.73× ▼ |
| `contacts, all [sub]` | 47 | 55 | 1.15× ▲ |
| `contacts, all, br [api]` | 22 | 29 | 1.33× ▲ |
| `contacts, all, gzip [api]` | 20 | 20 | 0.96× |
| `contacts, all, the fields of a list [api]` | 15 | 14 | 0.97× |
| `contacts, all, the fields of a list, br [api]` | 17 | 17 | 1.01× |
| `counts of 60 conversations [api]` | 38 | 18 | 0.47× ▼ |
| `counts of 60 conversations [sub]` | 117 | 116 | 0.99× |
| `db test [api]` | 0.5（状态 {404: 2}） | 0.8 | - |
| `db test [sub]` | 22 | 20 | 0.91× |
| `first conversation opened after start (first page) [api]` | 12 | 11 | 0.92× |
| `first page, 1 at once (all of them) [api]` | 5.3 | 4.5 | 0.84× |
| `first page, 1 at once (all of them) [sub]` | 46 | 46 | 0.99× |
| `first page, 16 at once (all of them) [api]` | 41 | 37 | 0.90× |
| `first page, 16 at once (all of them) [sub]` | 84 | 80 | 0.96× |
| `first page, 32 at once (all of them) [api]` | 79 | 78 | 0.98× |
| `first page, 32 at once (all of them) [sub]` | 84 | 84 | 1.00× |
| `first page, 6 at once (all of them) [api]` | 19 | 15 | 0.82× |
| `first page, 6 at once (all of them) [sub]` | 84 | 86 | 1.02× |
| `first page, 64 at once (all of them) [api]` | 164 | 144 | 0.88× ▼ |
| `first read after start (sessions, 50) [api]` | 184 | 170 | 0.92× |
| `group list [api]` | 0.5（状态 {404: 2}） | 6.2 | - |
| `group list [sub]` | 27 | 29 | 1.05× |
| `group members [api]` | 8.7 | 8.1 | 0.93× |
| `group members [sub]` | 38 | 39 | 1.01× |
| `image, again (each) [api]` | 1.8 | 1.6 | 0.91× |
| `image, again (each) [sub]` | 42 | 44 | 1.04× |
| `image, first time (each) [api]` | 2.8 | 2.6 | 0.95× |
| `image, first time (each) [sub]` | 43 | 45 | 1.04× |
| `latest 20 (polling) [api]` | 3.5 | 3.8 | 1.09× |
| `latest 20 (polling) [sub]` | 43 | 47 | 1.10× |
| `messages, first page [api]` | 4.8 | 4.5 | 0.94× |
| `messages, first page [sub]` | 46 | 47 | 1.01× |
| `messages, first page, br [api]` | 6.9 | 6.9 | 1.00× |
| `messages, first page, gzip [api]` | 5.5 | 5.1 | 0.94× |
| `messages, first page, the fields shown [api]` | 4.2 | 4.1 | 0.98× |
| `messages, first page, the fields shown, br [api]` | 6.7 | 5.8 | 0.86× |
| `messages, page in the middle [api]` | 2,368 | 118 | 0.05× ▼ |
| `messages, page in the middle [sub]` | 2,691 | 555 | 0.21× ▼ |
| `messages, page in the middle, from a cursor [api]` | 4.5 | 4.3 | 0.96× |
| `messages, page in the middle, from a cursor [sub]` | 46 | 47 | 1.01× |
| `one contact [api]` | 1.8 | 1.0 | 0.58× |
| `one contact [sub]` | 24 | 22 | 0.91× |
| `one message [api]` | 29 | 3.6 | 0.12× ▼ |
| `one message [sub]` | 39 | 39 | 1.00× |
| `push engine, CPU ms per second while listened to [api]` | 0.0 | 0.0 | - |
| `report annual years [api]` | 0.5（状态 {404: 2}） | 318 | - |
| `report annual years [sub]` | 530 | 525 | 0.99× |
| `search in the conversation [api]` | 4.1 | 3.7 | 0.90× |
| `search in the conversation [sub]` | 45 | 46 | 1.01× |
| `search in the conversation, a word found nowhere [api]` | 119 | 119 | 1.00× |
| `search in the conversation, a word found nowhere [sub]` | 203 | 212 | 1.04× |
| `server start (until it listens) [-]` | 19 | 19 | 1.01× |
| `sessions, 50 [api]` | 24 | 18 | 0.77× ▼ |
| `sessions, 50 [sub]` | 73 | 58 | 0.79× ▼ |
| `sessions, all [api]` | 19 | 25 | 1.31× ▲ |
| `sessions, all [sub]` | 57 | 59 | 1.03× |
| `sns post-counts --prefer-cache [api]` | 2.8 | 2.6 | 0.92× |
| `sns post-counts --prefer-cache [sub]` | 37 | 38 | 1.01× |
| `sns stats --fast [api]` | 2.8 | 2.6 | 0.94× |
| `sns stats --fast [sub]` | 37 | 38 | 1.02× |
| `sns timeline, 20 [api]` | 60 | 61 | 1.02× |
| `sns timeline, 20 [sub]` | 87 | 94 | 1.08× |
| `sns timeline, 20, far page [api]` | 31 | 30 | 0.98× |
| `sns timeline, 20, far page [sub]` | 89 | 92 | 1.03× |
| `sns users [api]` | 5.0 | 4.5 | 0.90× |
| `sns users [sub]` | 39 | 38 | 0.98× |
| `thumbnail of the image (each) [api]` | 1.6 | 1.5 | 0.93× |
| `thumbnail of the image (each) [sub]` | 42 | 43 | 1.04× |
