# 基准测试：base 与 head

- base：`e095929773dee6f6915333312c8e72a5af386894`（`e095929773`），weflow 5.0.1-185-ge0959297
- head：`HEAD`（`0b0b598503`），weflow 5.0.1-188-g0b0b5985
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 10 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 30.7 MiB | 30.8 MiB | 1.00× |

## 命令

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 26,640（26,367–30,417） | 26,628（25,884–32,462） | 1.00× |
| `export messages json with images` | 31,096（30,421–31,679） | 30,872（30,398–31,315） | 0.99× |
| `export messages json` | 5,306（5,257–5,440） | 4,998（4,892–5,066） | 0.94× |
| `export messages chatlab` | 4,673（4,636–4,798） | 4,057（4,028–4,100） | 0.87× ▼ |
| `export messages excel` | 5,717（5,617–5,745） | 5,326（5,258–5,457） | 0.93× |
| `export messages html` | 4,094（4,063–4,125） | 3,717（3,700–3,763） | 0.91× |
| `export messages txt` | 3,935（3,887–4,001） | 3,384（3,373–3,432） | 0.86× ▼ |
| `report annual` | 2,551（2,536–2,567） | 2,546（2,527–2,581） | 1.00× |
| `analytics overall` | 1,095（1,083–1,107） | 1,100（1,088–1,112） | 1.00× |
| `group members` | 1,148（1,134–1,160） | 1,152（1,124–1,193） | 1.00× |
| `group ranking` | 1,137（1,127–1,151） | 1,143（1,125–1,178） | 1.01× |
| `chat export-stats` | 1,030（1,017–1,046） | 1,030（1,025–1,043） | 1.00× |
| `chat date-counts` | 839（826–857） | 837（831–856） | 1.00× |
| `chat search` | 554（549–565） | 556（549–563） | 1.00× |
| `report dual` | 442（431–453） | 450（443–456） | 1.02× |
| `chat messages first page` | 194（191–207） | 193（190–202） | 1.00× |
| `chat messages offset` | 712（704–736） | 717（703–733） | 1.01× |
| `chat messages cursor` | 193（191–209） | 194（192–195） | 1.01× |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 2,362 | 2,358 | 1.00× |
| `export messages json with images` | 534 | 563 | 1.06× |
| `export messages json` | 516 | 497 | 0.96× |
| `export messages chatlab` | 557 | 532 | 0.95× |
| `export messages excel` | 518 | 492 | 0.95× |
| `export messages html` | 517 | 500 | 0.97× |
| `export messages txt` | 505 | 494 | 0.98× |
| `report annual` | 19 | 19 | 0.99× |
| `analytics overall` | 166 | 166 | 1.00× |
| `report dual` | 19 | 19 | 1.03× |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 18,242 | 18,820 | 1.03× |
| `export messages json with images` | 26,867 | 25,555 | 0.95× |
| `export messages json` | 12,211 | 10,156 | 0.83× ▼ |
| `export messages chatlab` | 11,023 | 9,258 | 0.84× ▼ |
| `export messages excel` | 12,430 | 10,383 | 0.84× ▼ |
| `export messages html` | 11,727 | 9,570 | 0.82× ▼ |
| `export messages txt` | 10,883 | 9,477 | 0.87× ▼ |
| `report annual` | 2,203 | 2,156 | 0.98× |
| `analytics overall` | 922 | 906 | 0.98× |
| `group members` | 992 | 984 | 0.99× |
| `group ranking` | 984 | 992 | 1.01× |
| `chat export-stats` | 859 | 844 | 0.98× |
| `chat date-counts` | 672 | 680 | 1.01× |
| `chat search` | 414 | 414 | 1.00× |
| `report dual` | 359 | 375 | 1.04× |
| `chat messages first page` | 164 | 164 | 1.00× |
| `chat messages offset` | 578 | 570 | 0.99× |
| `chat messages cursor` | 156 | 172 | 1.10× |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 35,039 | 35,570 | 1.02× |
| `export messages json with images` | 36,438 | 36,391 | 1.00× |
| `export messages json` | 1,000 | 922 | 0.92× |
| `export messages chatlab` | 805 | 750 | 0.93× |
| `export messages excel` | 1,086 | 1,000 | 0.92× |
| `export messages html` | 938 | 750 | 0.80× ▼ |
| `export messages txt` | 836 | 773 | 0.93× |
| `report annual` | 367 | 367 | 1.00× |
| `analytics overall` | 172 | 188 | 1.09× |
| `group members` | 141 | 188 | 1.33× |
| `group ranking` | 172 | 164 | 0.95× |
| `chat export-stats` | 188 | 172 | 0.92× |
| `chat date-counts` | 156 | 164 | 1.05× |
| `chat search` | 141 | 148 | 1.06× |
| `report dual` | 62 | 78 | 1.25× |
| `chat messages first page` | 39 | 39 | 1.00× |
| `chat messages offset` | 133 | 156 | 1.18× |
| `chat messages cursor` | 23 | 39 | 1.67× |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 294 | 282 | 0.96× |
| `export messages json with images` | 617 | 585 | 0.95× |
| `export messages json` | 263 | 231 | 0.88× ▼ |
| `export messages chatlab` | 272 | 226 | 0.83× ▼ |
| `export messages excel` | 289 | 253 | 0.87× ▼ |
| `export messages html` | 269 | 229 | 0.85× ▼ |
| `export messages txt` | 273 | 241 | 0.88× ▼ |
| `report annual` | 132 | 132 | 1.00× |
| `analytics overall` | 117 | 117 | 1.00× |
| `group members` | 206 | 206 | 1.00× |
| `group ranking` | 198 | 198 | 1.00× |
| `chat export-stats` | 198 | 198 | 1.00× |
| `chat date-counts` | 195 | 195 | 1.00× |
| `chat search` | 58.6 | 58.6 | 1.00× |
| `report dual` | 50.9 | 48.7 | 0.96× |
| `chat messages first page` | 31.6 | 31.5 | 1.00× |
| `chat messages offset` | 145 | 145 | 1.00× |
| `chat messages cursor` | 31.7 | 31.6 | 1.00× |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 438 | 426 | 0.97× |
| `export messages json with images` | 830 | 818 | 0.99× |
| `export messages json` | 456 | 422 | 0.93× |
| `export messages chatlab` | 443 | 429 | 0.97× |
| `export messages excel` | 468 | 426 | 0.91× |
| `export messages html` | 457 | 417 | 0.91× |
| `export messages txt` | 461 | 443 | 0.96× |
| `report annual` | 276 | 275 | 0.99× |
| `analytics overall` | 242 | 242 | 1.00× |
| `group members` | 339 | 339 | 1.00× |
| `group ranking` | 328 | 328 | 1.00× |
| `chat export-stats` | 328 | 328 | 1.00× |
| `chat date-counts` | 324 | 324 | 1.00× |
| `chat search` | 186 | 186 | 1.00× |
| `report dual` | 190 | 189 | 1.00× |
| `chat messages first page` | 157 | 157 | 1.00× |
| `chat messages offset` | 292 | 291 | 1.00× |
| `chat messages cursor` | 157 | 158 | 1.00× |

### 峰值线程数

每 20 ms 采样一次主进程得到的最大值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 12 | 12 | 1.00× |
| `export messages json with images` | 12 | 12 | 1.00× |
| `export messages json` | 11 | 11 | 1.00× |
| `export messages chatlab` | 11 | 11 | 1.00× |
| `export messages excel` | 11 | 11 | 1.00× |
| `export messages html` | 11 | 11 | 1.00× |
| `export messages txt` | 11 | 11 | 1.00× |
| `report annual` | 8.0 | 8.0 | 1.00× |
| `analytics overall` | 8.0 | 8.0 | 1.00× |
| `group members` | 8.0 | 8.0 | 1.00× |
| `group ranking` | 8.0 | 8.0 | 1.00× |
| `chat export-stats` | 8.0 | 8.0 | 1.00× |
| `chat date-counts` | 8.0 | 8.0 | 1.00× |
| `chat search` | 8.0 | 8.0 | 1.00× |
| `report dual` | 8.0 | 8.0 | 1.00× |
| `chat messages first page` | 8.0 | 8.0 | 1.00× |
| `chat messages offset` | 8.0 | 8.0 | 1.00× |
| `chat messages cursor` | 8.0 | 8.0 | 1.00× |

### 读取（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 847 | 847 | 1.00× |
| `export messages json with images` | 1,469 | 1,469 | 1.00× |
| `export messages json` | 812 | 811 | 1.00× |
| `export messages chatlab` | 653 | 654 | 1.00× |
| `export messages excel` | 687 | 687 | 1.00× |
| `export messages html` | 665 | 662 | 1.00× |
| `export messages txt` | 604 | 605 | 1.00× |
| `report annual` | 132 | 132 | 1.00× |
| `analytics overall` | 128 | 128 | 1.00× |
| `group members` | 190 | 190 | 1.00× |
| `group ranking` | 190 | 190 | 1.00× |
| `chat export-stats` | 191 | 191 | 1.00× |
| `chat date-counts` | 190 | 190 | 1.00× |
| `chat search` | 93.4 | 93.4 | 1.00× |
| `report dual` | 31.8 | 31.8 | 1.00× |
| `chat messages first page` | 21.1 | 21.1 | 1.00× |
| `chat messages offset` | 158 | 158 | 1.00× |
| `chat messages cursor` | 21.0 | 21.0 | 1.00× |

### 写入（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 417 | 417 | 1.00× |
| `export messages json with images` | 847 | 847 | 1.00× |
| `export messages json` | 432 | 432 | 1.00× |
| `export messages chatlab` | 99.1 | 99.1 | 1.00× |
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
| `chat messages first page` | 0.1 | 0.1 | 1.00× |
| `chat messages offset` | 0.1 | 0.1 | 1.00× |
| `chat messages cursor` | 0.1 | 0.1 | 1.00× |

### 页错误

进程及其子进程，包括软页错误。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 142,722 | 141,059 | 0.99× |
| `export messages json with images` | 255,393 | 256,960 | 1.01× |
| `export messages json` | 109,516 | 89,762 | 0.82× ▼ |
| `export messages chatlab` | 110,879 | 84,952 | 0.77× ▼ |
| `export messages excel` | 115,597 | 95,470 | 0.83× ▼ |
| `export messages html` | 104,932 | 85,943 | 0.82× ▼ |
| `export messages txt` | 106,498 | 90,637 | 0.85× ▼ |
| `report annual` | 36,269 | 36,260 | 1.00× |
| `analytics overall` | 31,038 | 31,070 | 1.00× |
| `group members` | 58,668 | 58,679 | 1.00× |
| `group ranking` | 56,565 | 56,608 | 1.00× |
| `chat export-stats` | 54,596 | 54,574 | 1.00× |
| `chat date-counts` | 52,484 | 52,514 | 1.00× |
| `chat search` | 15,220 | 15,213 | 1.00× |
| `report dual` | 21,455 | 21,256 | 0.99× |
| `chat messages first page` | 8,356 | 8,350 | 1.00× |
| `chat messages offset` | 37,278 | 37,351 | 1.00× |
| `chat messages cursor` | 8,400 | 8,390 | 1.00× |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | 6,432 | 6,478 | 1.01× |
| `export messages json` | 37,692 | 40,018 | 1.06× |
| `export messages chatlab` | 42,798 | 49,296 | 1.15× ▲ |
| `export messages excel` | 34,982 | 37,554 | 1.07× |
| `export messages html` | 48,854 | 53,803 | 1.10× ▲ |
| `export messages txt` | 50,823 | 59,093 | 1.16× ▲ |
| `report annual` | 127,693 | 127,965 | 1.00× |
| `group members` | 174,151 | 173,605 | 1.00× |
| `group ranking` | 175,978 | 175,048 | 0.99× |
| `chat export-stats` | 194,260 | 194,190 | 1.00× |
| `chat date-counts` | 238,443 | 238,989 | 1.00× |
| `report dual` | 22,645 | 22,204 | 0.98× |
| `chat messages first page` | 258 | 259 | 1.00× |
| `chat messages offset` | 70 | 70 | 0.99× |
| `chat messages cursor` | 259 | 258 | 0.99× |

### 分配次数

分配和重新分配的次数。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 33,890,910 | 33,892,407 | 1.00× |
| `export messages json with images` | 61,248,293 | 53,801,681 | 0.88× ▼ |
| `export messages json` | 30,093,798 | 22,637,562 | 0.75× ▼ |
| `export messages chatlab` | 26,126,617 | 18,664,172 | 0.71× ▼ |
| `export messages excel` | 24,283,752 | 16,827,647 | 0.69× ▼ |
| `export messages html` | 26,067,397 | 18,611,266 | 0.71× ▼ |
| `export messages txt` | 20,528,302 | 13,072,432 | 0.64× ▼ |
| `report annual` | 10,160,502 | 10,160,502 | 1.00× |
| `analytics overall` | 2,425,672 | 2,425,673 | 1.00× |
| `group members` | 123,389 | 123,390 | 1.00× |
| `group ranking` | 23,634 | 23,634 | 1.00× |
| `chat export-stats` | 105,522 | 105,522 | 1.00× |
| `chat date-counts` | 15,955 | 15,956 | 1.00× |
| `chat search` | 45,991 | 45,991 | 1.00× |
| `report dual` | 626,816 | 616,331 | 0.98× |
| `chat messages first page` | 11,668 | 11,496 | 0.99× |
| `chat messages offset` | 17,874 | 17,715 | 0.99× |
| `chat messages cursor` | 11,217 | 11,059 | 0.99× |

### 累计分配（MiB）

每次分配的大小之和，重新分配只计增长的部分。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 8,408 | 8,403 | 1.00× |
| `export messages json with images` | 47,885 | 46,682 | 0.97× |
| `export messages json` | 43,298 | 42,086 | 0.97× |
| `export messages chatlab` | 41,389 | 40,176 | 0.97× |
| `export messages excel` | 41,185 | 39,979 | 0.97× |
| `export messages html` | 40,167 | 38,960 | 0.97× |
| `export messages txt` | 40,188 | 38,993 | 0.97× |
| `report annual` | 1,171 | 1,171 | 1.00× |
| `analytics overall` | 400 | 400 | 1.00× |
| `group members` | 384 | 384 | 1.00× |
| `group ranking` | 375 | 375 | 1.00× |
| `chat export-stats` | 384 | 384 | 1.00× |
| `chat date-counts` | 373 | 373 | 1.00× |
| `chat search` | 204 | 204 | 1.00× |
| `report dual` | 1,876 | 1,871 | 1.00× |
| `chat messages first page` | 59.4 | 59.3 | 1.00× |
| `chat messages offset` | 342 | 341 | 1.00× |
| `chat messages cursor` | 57.7 | 57.6 | 1.00× |

### 峰值存活堆（MiB）

同一时刻仍未释放的分配之和的最大值。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 182 | 176 | 0.96× |
| `export messages json with images` | 424 | 424 | 1.00× |
| `export messages json` | 77.4 | 63.3 | 0.82× ▼ |
| `export messages chatlab` | 73.2 | 62.0 | 0.85× ▼ |
| `export messages excel` | 102 | 82.3 | 0.80× ▼ |
| `export messages html` | 77.2 | 63.2 | 0.82× ▼ |
| `export messages txt` | 77.2 | 63.2 | 0.82× ▼ |
| `report annual` | 15.8 | 15.8 | 1.00× |
| `analytics overall` | 11.8 | 11.8 | 1.00× |
| `group members` | 11.8 | 11.8 | 1.00× |
| `group ranking` | 10.6 | 10.6 | 1.00× |
| `chat export-stats` | 14.6 | 14.6 | 1.00× |
| `chat date-counts` | 10.6 | 10.6 | 1.00× |
| `chat search` | 10.6 | 10.6 | 1.00× |
| `report dual` | 12.4 | 12.2 | 0.98× |
| `chat messages first page` | 10.6 | 10.6 | 1.00× |
| `chat messages offset` | 15.3 | 15.3 | 1.00× |
| `chat messages cursor` | 10.6 | 10.6 | 1.00× |

## `weflow serve` 持续负载

每个版本启动一个 `weflow serve`，用 4 个并发连续请求 90 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。

|  | base | head | head / base |
|---|---|---|---|
| 每秒请求数 | 18 | 19 | 1.02× |
| 错误数 | 0 | 0 | - |
| 延迟 p50 (ms) | 68 | 63 | 0.94× |
| 延迟 p95 (ms) | 1,154 | 1,159 | 1.00× |
| 延迟 p99 (ms) | 2,057 | 2,181 | 1.06× |
| 启动到开始监听 (ms) | 21 | 22 | 1.07× |
| 工作集，第 1 段中位数 (MiB) | 334 | 357 | 1.07× |
| 工作集，第 2 段中位数 (MiB) | 342 | 361 | 1.05× |
| 工作集，第 3 段中位数 (MiB) | 348 | 348 | 1.00× |
| 工作集，第 4 段中位数 (MiB) | 353 | 345 | 0.98× |
| 第 2 段到第 4 段的增长 (MiB) | 11 | -16 | - |
| 峰值工作集 (MiB) | 391 | 391 | 1.00× |
| 峰值线程数 | 13 | 13 | 1.00× |
| CPU 时间 (ms) | 143,062 | 146,016 | 1.02× |

各路由：请求次数，延迟 p50 / p95（ms）。

| 路由 | base | head | p50 head / base |
|---|---|---|---|
| `/api/chat/messages`，大群任意一页 | 570 次，177 / 384 | 581 次，172 / 363 | 0.97× |
| `/api/chat/messages`，其他会话第一页 | 360 次，5.1 / 155 | 367 次，5.1 / 143 | 1.01× |
| `/api/chat/sessions` | 229 次，40 / 316 | 232 次，39 / 227 | 0.97× |
| `/api/chat/search`，全部会话 | 163 次，1,154 / 2,386 | 167 次，1,168 / 2,526 | 1.01× |
| `/api/chat/date-counts` | 161 次，4.0 / 145 | 164 次，5.1 / 153 | 1.29× |
| `/api/chat/contacts` | 178 次，24 / 41 | 180 次，23 / 45 | 0.97× |

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 0，head 0。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。

| 路由 | base | head | head / base |
|---|---|---|---|
| `16 pages at once (all of them) [api]` | 49 | 53 | 1.07× |
| `16 pages at once (all of them) [sub]` | 82 | 91 | 1.11× ▲ |
| `20 images at once (all of them) [api]` | 17 | 17 | 1.05× |
| `20 images at once (all of them) [sub]` | 48 | 51 | 1.05× |
| `analytics overall (cached) [api]` | 2.3 | 2.4 | 1.03× |
| `analytics overall (cached) [sub]` | 24 | 24 | 1.01× |
| `analytics overall --force [api]` | 546 | 551 | 1.01× |
| `analytics overall --force [sub]` | 908 | 921 | 1.01× |
| `analytics rankings (cached) [api]` | 7.9 | 8.8 | 1.11× |
| `analytics rankings (cached) [sub]` | 31 | 32 | 1.04× |
| `analytics time (cached) [api]` | 2.4 | 2.9 | 1.20× |
| `analytics time (cached) [sub]` | 23 | 24 | 1.04× |
| `biz pay-records [api]` | 1.0 | 1.0 | 1.00× |
| `biz pay-records [sub]` | 35 | 36 | 1.02× |
| `cache list [api]` | 0.7 | 0.7 | 1.00× |
| `cache list [sub]` | 20 | 20 | 0.98× |
| `chat date-counts [api]` | 79 | 81 | 1.02× |
| `chat date-counts [sub]` | 677 | 694 | 1.02× |
| `chat dates [api]` | 80 | 80 | 1.00× |
| `chat dates [sub]` | 679 | 690 | 1.02× |
| `chat detail --fast [api]` | 1.5 | 1.5 | 0.99× |
| `chat detail --fast [sub]` | 92 | 93 | 1.01× |
| `chat resources, all conversations [api]` | 99 | 98 | 0.98× |
| `chat resources, all conversations [sub]` | 269 | 280 | 1.04× |
| `chat resources, all conversations, far page [api]` | 170 | 174 | 1.02× |
| `chat resources, all conversations, far page [sub]` | 360 | 364 | 1.01× |
| `chat resources, all conversations, file [api]` | 125 | 133 | 1.06× |
| `chat resources, all conversations, file [sub]` | 253 | 257 | 1.02× |
| `chat resources, all conversations, image [api]` | 59 | 61 | 1.04× |
| `chat resources, all conversations, image [sub]` | 134 | 137 | 1.02× |
| `chat resources, all conversations, video [api]` | 72 | 71 | 0.98× |
| `chat resources, all conversations, video [sub]` | 171 | 174 | 1.02× |
| `chat resources, all conversations, voice [api]` | 39 | 39 | 1.00× |
| `chat resources, all conversations, voice [sub]` | 122 | 125 | 1.02× |
| `chat resources, one conversation [api]` | 58 | 60 | 1.03× |
| `chat resources, one conversation [sub]` | 222 | 229 | 1.03× |
| `chat statuses of 60 conversations [api]` | 1.6 | 1.7 | 1.08× |
| `chat statuses of 60 conversations [sub]` | 24 | 24 | 1.03× |
| `chat tab-counts [api]` | 2.1 | 2.1 | 1.00× |
| `chat tab-counts [sub]` | 24 | 25 | 1.01× |
| `config profiles [api]` | 0.6 | 0.6 | 0.94× |
| `config profiles [sub]` | 20 | 20 | 1.01× |
| `contacts, all [api]` | 16 | 15 | 0.93× |
| `contacts, all [sub]` | 56 | 57 | 1.01× |
| `contacts, all, br [api]` | 23 | 24 | 1.05× |
| `contacts, all, gzip [api]` | 20 | 20 | 0.99× |
| `contacts, all, the fields of a list [api]` | 15 | 15 | 0.98× |
| `contacts, all, the fields of a list, br [api]` | 17 | 17 | 1.01× |
| `counts of 60 conversations [api]` | 17 | 17 | 1.02× |
| `counts of 60 conversations [sub]` | 116 | 116 | 1.00× |
| `db test [api]` | 0.7 | 0.7 | 1.03× |
| `db test [sub]` | 21 | 20 | 0.97× |
| `first conversation opened after start (first page) [api]` | 11 | 11 | 1.06× |
| `first page, 1 at once (all of them) [api]` | 4.4 | 4.7 | 1.07× |
| `first page, 1 at once (all of them) [sub]` | 46 | 48 | 1.04× |
| `first page, 16 at once (all of them) [api]` | 37 | 39 | 1.06× |
| `first page, 16 at once (all of them) [sub]` | 83 | 85 | 1.03× |
| `first page, 32 at once (all of them) [api]` | 72 | 76 | 1.05× |
| `first page, 32 at once (all of them) [sub]` | 83 | 85 | 1.03× |
| `first page, 6 at once (all of them) [api]` | 16 | 16 | 1.00× |
| `first page, 6 at once (all of them) [sub]` | 81 | 85 | 1.04× |
| `first page, 64 at once (all of them) [api]` | 143 | 154 | 1.08× |
| `first read after start (sessions, 50) [api]` | 170 | 171 | 1.00× |
| `group list [api]` | 5.3 | 5.9 | 1.10× |
| `group list [sub]` | 27 | 28 | 1.06× |
| `group members [api]` | 1.5 | 1.5 | 0.98× |
| `group members [sub]` | 37 | 39 | 1.04× |
| `image, again (each) [api]` | 1.6 | 1.7 | 1.03× |
| `image, again (each) [sub]` | 44 | 46 | 1.05× |
| `image, first time (each) [api]` | 3.5 | 3.7 | 1.06× |
| `image, first time (each) [sub]` | 46 | 48 | 1.04× |
| `latest 20 (polling) [api]` | 2.9 | 3.0 | 1.05× |
| `latest 20 (polling) [sub]` | 44 | 44 | 1.00× |
| `messages, first page [api]` | 4.2 | 4.3 | 1.03× |
| `messages, first page [sub]` | 46 | 46 | 1.02× |
| `messages, first page, br [api]` | 6.3 | 6.4 | 1.02× |
| `messages, first page, gzip [api]` | 5.1 | 5.3 | 1.05× |
| `messages, first page, the fields shown [api]` | 4.3 | 4.3 | 1.00× |
| `messages, first page, the fields shown, br [api]` | 6.2 | 6.3 | 1.02× |
| `messages, page in the middle [api]` | 114 | 116 | 1.01× |
| `messages, page in the middle [sub]` | 559 | 556 | 0.99× |
| `messages, page in the middle, from a cursor [api]` | 4.1 | 3.9 | 0.95× |
| `messages, page in the middle, from a cursor [sub]` | 46 | 46 | 1.01× |
| `one contact [api]` | 0.9 | 0.9 | 1.01× |
| `one contact [sub]` | 22 | 22 | 1.02× |
| `one message [api]` | 1.2 | 1.2 | 0.98× |
| `one message [sub]` | 38 | 40 | 1.03× |
| `push engine, CPU ms per second while listened to [api]` | 0.0 | 1.6 | - |
| `report annual years [api]` | 149 | 154 | 1.03× |
| `report annual years [sub]` | 521 | 525 | 1.01× |
| `search in the conversation [api]` | 3.3 | 3.3 | 0.99× |
| `search in the conversation [sub]` | 45 | 46 | 1.01× |
| `search in the conversation, a word found nowhere [api]` | 71 | 71 | 1.01× |
| `search in the conversation, a word found nowhere [sub]` | 204 | 204 | 1.00× |
| `server start (until it listens) [-]` | 20 | 20 | 0.97× |
| `sessions, 50 [api]` | 17 | 17 | 1.01× |
| `sessions, 50 [sub]` | 58 | 57 | 1.00× |
| `sessions, all [api]` | 18 | 18 | 1.02× |
| `sessions, all [sub]` | 59 | 60 | 1.03× |
| `sns post-counts --prefer-cache [api]` | 0.6 | 0.6 | 0.98× |
| `sns post-counts --prefer-cache [sub]` | 37 | 38 | 1.03× |
| `sns stats --fast [api]` | 0.5 | 0.5 | 0.95× |
| `sns stats --fast [sub]` | 37 | 38 | 1.01× |
| `sns timeline, 20 [api]` | 8.5 | 8.7 | 1.03× |
| `sns timeline, 20 [sub]` | 90 | 92 | 1.02× |
| `sns timeline, 20, far page [api]` | 7.9 | 8.8 | 1.11× |
| `sns timeline, 20, far page [sub]` | 90 | 91 | 1.01× |
| `sns users [api]` | 4.5 | 4.7 | 1.04× |
| `sns users [sub]` | 38 | 39 | 1.01× |
| `thumbnail of the image (each) [api]` | 1.4 | 1.5 | 1.06× |
| `thumbnail of the image (each) [sub]` | 44 | 46 | 1.05× |
