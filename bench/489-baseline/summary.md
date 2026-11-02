# 基准测试：base 与 head

- base：`HEAD`（`e095929773`），weflow 5.0.1-185-ge0959297
- head：`HEAD`（`e095929773`），weflow 5.0.1-185-ge0959297
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 10 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 30.7 MiB | 30.7 MiB | 1.00× |

## 命令

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 25,054（23,942–27,511） | 24,990（23,875–35,832） | 1.00× |
| `export messages json with images` | 28,612（28,129–29,367） | 28,726（27,951–29,259） | 1.00× |
| `export messages json` | 5,207（5,137–5,299） | 5,224（5,135–5,272） | 1.00× |
| `export messages chatlab` | 4,529（4,482–4,587） | 4,544（4,496–4,610） | 1.00× |
| `export messages excel` | 5,494（5,435–5,572） | 5,514（5,445–5,539） | 1.00× |
| `export messages html` | 3,976（3,902–4,189） | 3,941（3,914–4,027） | 0.99× |
| `export messages txt` | 3,817（3,772–3,844） | 3,839（3,777–3,871） | 1.01× |
| `report annual` | 2,141（2,120–2,167） | 2,143（2,121–2,161） | 1.00× |
| `analytics overall` | 969（955–979） | 965（955–985） | 1.00× |
| `group members` | 1,068（1,054–1,085） | 1,063（1,053–1,083） | 1.00× |
| `group ranking` | 1,048（1,035–1,056） | 1,045（1,034–1,059） | 1.00× |
| `chat export-stats` | 950（936–959） | 941（932–958） | 0.99× |
| `chat date-counts` | 781（776–788） | 788（774–794） | 1.01× |
| `chat search` | 450（447–458） | 452（446–460） | 1.00× |
| `report dual` | 414（405–423） | 409（405–423） | 0.99× |
| `chat messages first page` | 188（186–215） | 188（187–199） | 1.00× |
| `chat messages offset` | 653（647–668） | 653（645–659） | 1.00× |
| `chat messages cursor` | 188（186–198） | 187（186–190） | 1.00× |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 2,351 | 2,354 | 1.00× |
| `export messages json with images` | 510 | 522 | 1.02× |
| `export messages json` | 488 | 494 | 1.01× |
| `export messages chatlab` | 521 | 526 | 1.01× |
| `export messages excel` | 490 | 484 | 0.99× |
| `export messages html` | 484 | 481 | 0.99× |
| `export messages txt` | 479 | 481 | 1.00× |
| `report annual` | 19 | 19 | 1.00× |
| `analytics overall` | 164 | 165 | 1.00× |
| `report dual` | 19 | 19 | 1.01× |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 17,016 | 16,617 | 0.98× |
| `export messages json with images` | 25,625 | 25,414 | 0.99× |
| `export messages json` | 11,680 | 11,844 | 1.01× |
| `export messages chatlab` | 10,719 | 10,703 | 1.00× |
| `export messages excel` | 12,023 | 12,148 | 1.01× |
| `export messages html` | 11,258 | 11,258 | 1.00× |
| `export messages txt` | 10,633 | 10,656 | 1.00× |
| `report annual` | 1,805 | 1,875 | 1.04× |
| `analytics overall` | 797 | 797 | 1.00× |
| `group members` | 914 | 875 | 0.96× |
| `group ranking` | 898 | 883 | 0.98× |
| `chat export-stats` | 781 | 781 | 1.00× |
| `chat date-counts` | 594 | 625 | 1.05× |
| `chat search` | 344 | 359 | 1.05× |
| `report dual` | 344 | 344 | 1.00× |
| `chat messages first page` | 180 | 156 | 0.87× |
| `chat messages offset` | 531 | 531 | 1.00× |
| `chat messages cursor` | 156 | 164 | 1.05× |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 26,078 | 25,836 | 0.99× |
| `export messages json with images` | 26,672 | 26,867 | 1.01× |
| `export messages json` | 1,148 | 1,086 | 0.95× |
| `export messages chatlab` | 812 | 844 | 1.04× |
| `export messages excel` | 1,125 | 1,023 | 0.91× |
| `export messages html` | 875 | 891 | 1.02× |
| `export messages txt` | 719 | 789 | 1.10× |
| `report annual` | 297 | 281 | 0.95× |
| `analytics overall` | 172 | 172 | 1.00× |
| `group members` | 133 | 172 | 1.29× |
| `group ranking` | 164 | 164 | 1.00× |
| `chat export-stats` | 164 | 148 | 0.90× |
| `chat date-counts` | 188 | 148 | 0.79× |
| `chat search` | 109 | 102 | 0.93× |
| `report dual` | 70 | 86 | 1.22× |
| `chat messages first page` | 47 | 31 | 0.67× |
| `chat messages offset` | 109 | 117 | 1.07× |
| `chat messages cursor` | 31 | 23 | 0.75× |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 294 | 295 | 1.00× |
| `export messages json with images` | 617 | 617 | 1.00× |
| `export messages json` | 262 | 266 | 1.02× |
| `export messages chatlab` | 267 | 269 | 1.01× |
| `export messages excel` | 290 | 295 | 1.02× |
| `export messages html` | 271 | 269 | 0.99× |
| `export messages txt` | 272 | 275 | 1.01× |
| `report annual` | 134 | 134 | 1.00× |
| `analytics overall` | 120 | 120 | 1.00× |
| `group members` | 206 | 206 | 1.00× |
| `group ranking` | 198 | 198 | 1.00× |
| `chat export-stats` | 198 | 198 | 1.00× |
| `chat date-counts` | 195 | 195 | 1.00× |
| `chat search` | 58.6 | 58.5 | 1.00× |
| `report dual` | 51.0 | 50.8 | 1.00× |
| `chat messages first page` | 31.6 | 31.6 | 1.00× |
| `chat messages offset` | 145 | 145 | 1.00× |
| `chat messages cursor` | 31.7 | 31.7 | 1.00× |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 438 | 438 | 1.00× |
| `export messages json with images` | 829 | 824 | 0.99× |
| `export messages json` | 454 | 453 | 1.00× |
| `export messages chatlab` | 445 | 442 | 0.99× |
| `export messages excel` | 473 | 473 | 1.00× |
| `export messages html` | 457 | 456 | 1.00× |
| `export messages txt` | 459 | 461 | 1.00× |
| `report annual` | 276 | 276 | 1.00× |
| `analytics overall` | 247 | 247 | 1.00× |
| `group members` | 339 | 339 | 1.00× |
| `group ranking` | 328 | 328 | 1.00× |
| `chat export-stats` | 328 | 328 | 1.00× |
| `chat date-counts` | 324 | 324 | 1.00× |
| `chat search` | 186 | 186 | 1.00× |
| `report dual` | 189 | 189 | 1.00× |
| `chat messages first page` | 157 | 157 | 1.00× |
| `chat messages offset` | 292 | 292 | 1.00× |
| `chat messages cursor` | 157 | 157 | 1.00× |

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
| `export messages json with images` | 1,467 | 1,468 | 1.00× |
| `export messages json` | 811 | 811 | 1.00× |
| `export messages chatlab` | 651 | 651 | 1.00× |
| `export messages excel` | 686 | 685 | 1.00× |
| `export messages html` | 664 | 663 | 1.00× |
| `export messages txt` | 603 | 603 | 1.00× |
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
| `export media images` | 138,985 | 140,779 | 1.01× |
| `export messages json with images` | 255,221 | 252,042 | 0.99× |
| `export messages json` | 107,168 | 109,844 | 1.02× |
| `export messages chatlab` | 111,153 | 114,052 | 1.03× |
| `export messages excel` | 115,746 | 120,687 | 1.04× |
| `export messages html` | 105,158 | 105,707 | 1.01× |
| `export messages txt` | 104,618 | 106,532 | 1.02× |
| `report annual` | 36,104 | 36,100 | 1.00× |
| `analytics overall` | 30,857 | 30,871 | 1.00× |
| `group members` | 58,682 | 58,688 | 1.00× |
| `group ranking` | 56,583 | 56,552 | 1.00× |
| `chat export-stats` | 54,564 | 54,570 | 1.00× |
| `chat date-counts` | 52,426 | 52,504 | 1.00× |
| `chat search` | 15,228 | 15,216 | 1.00× |
| `report dual` | 21,688 | 21,543 | 0.99× |
| `chat messages first page` | 8,368 | 8,374 | 1.00× |
| `chat messages offset` | 37,362 | 37,328 | 1.00× |
| `chat messages cursor` | 8,399 | 8,408 | 1.00× |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | 6,990 | 6,962 | 1.00× |
| `export messages json` | 38,407 | 38,285 | 1.00× |
| `export messages chatlab` | 44,156 | 44,019 | 1.00× |
| `export messages excel` | 36,404 | 36,273 | 1.00× |
| `export messages html` | 50,300 | 50,755 | 1.01× |
| `export messages txt` | 52,394 | 52,095 | 0.99× |
| `report annual` | 152,158 | 152,015 | 1.00× |
| `group members` | 187,339 | 188,205 | 1.00× |
| `group ranking` | 190,891 | 191,328 | 1.00× |
| `chat export-stats` | 210,509 | 212,624 | 1.01× |
| `chat date-counts` | 255,992 | 253,909 | 0.99× |
| `report dual` | 24,158 | 24,427 | 1.01× |
| `chat messages first page` | 266 | 266 | 1.00× |
| `chat messages offset` | 77 | 77 | 1.00× |
| `chat messages cursor` | 267 | 267 | 1.00× |

### 分配次数

分配和重新分配的次数。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 33,893,103 | 33,903,983 | 1.00× |
| `export messages json with images` | 61,269,668 | 61,264,850 | 1.00× |
| `export messages json` | 30,093,661 | 30,093,667 | 1.00× |
| `export messages chatlab` | 26,120,183 | 26,120,296 | 1.00× |
| `export messages excel` | 24,283,420 | 24,283,642 | 1.00× |
| `export messages html` | 26,067,310 | 26,067,081 | 1.00× |
| `export messages txt` | 20,534,752 | 20,528,065 | 1.00× |
| `report annual` | 10,160,501 | 10,160,502 | 1.00× |
| `analytics overall` | 2,425,673 | 2,425,673 | 1.00× |
| `group members` | 123,389 | 123,389 | 1.00× |
| `group ranking` | 23,633 | 23,634 | 1.00× |
| `chat export-stats` | 105,523 | 105,522 | 1.00× |
| `chat date-counts` | 15,956 | 15,955 | 1.00× |
| `chat search` | 45,989 | 45,990 | 1.00× |
| `report dual` | 626,816 | 626,816 | 1.00× |
| `chat messages first page` | 11,668 | 11,668 | 1.00× |
| `chat messages offset` | 17,874 | 17,873 | 1.00× |
| `chat messages cursor` | 11,217 | 11,217 | 1.00× |

### 累计分配（MiB）

每次分配的大小之和，重新分配只计增长的部分。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 8,408 | 8,409 | 1.00× |
| `export messages json with images` | 47,876 | 47,884 | 1.00× |
| `export messages json` | 43,292 | 43,292 | 1.00× |
| `export messages chatlab` | 41,375 | 41,388 | 1.00× |
| `export messages excel` | 41,171 | 41,180 | 1.00× |
| `export messages html` | 40,163 | 40,154 | 1.00× |
| `export messages txt` | 40,183 | 40,177 | 1.00× |
| `report annual` | 1,171 | 1,171 | 1.00× |
| `analytics overall` | 400 | 400 | 1.00× |
| `group members` | 384 | 384 | 1.00× |
| `group ranking` | 375 | 375 | 1.00× |
| `chat export-stats` | 384 | 384 | 1.00× |
| `chat date-counts` | 373 | 373 | 1.00× |
| `chat search` | 204 | 204 | 1.00× |
| `report dual` | 1,876 | 1,876 | 1.00× |
| `chat messages first page` | 59.4 | 59.4 | 1.00× |
| `chat messages offset` | 342 | 342 | 1.00× |
| `chat messages cursor` | 57.7 | 57.7 | 1.00× |

### 峰值存活堆（MiB）

同一时刻仍未释放的分配之和的最大值。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 182 | 182 | 1.00× |
| `export messages json with images` | 424 | 424 | 1.00× |
| `export messages json` | 77.4 | 77.4 | 1.00× |
| `export messages chatlab` | 72.0 | 72.0 | 1.00× |
| `export messages excel` | 105 | 106 | 1.01× |
| `export messages html` | 77.2 | 77.2 | 1.00× |
| `export messages txt` | 77.3 | 77.3 | 1.00× |
| `report annual` | 15.8 | 15.8 | 1.00× |
| `analytics overall` | 11.8 | 11.8 | 1.00× |
| `group members` | 11.8 | 11.8 | 1.00× |
| `group ranking` | 10.6 | 10.6 | 1.00× |
| `chat export-stats` | 14.6 | 14.6 | 1.00× |
| `chat date-counts` | 10.6 | 7.7 | 0.73× |
| `chat search` | 10.1 | 10.6 | 1.05× |
| `report dual` | 12.4 | 12.4 | 1.00× |
| `chat messages first page` | 10.6 | 10.6 | 1.00× |
| `chat messages offset` | 15.3 | 15.3 | 1.00× |
| `chat messages cursor` | 10.6 | 10.6 | 1.00× |

## `weflow serve` 持续负载

每个版本启动一个 `weflow serve`，用 4 个并发连续请求 90 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。

|  | base | head | head / base |
|---|---|---|---|
| 每秒请求数 | 21 | 22 | 1.03× |
| 错误数 | 0 | 0 | - |
| 延迟 p50 (ms) | 76 | 78 | 1.03× |
| 延迟 p95 (ms) | 901 | 863 | 0.96× |
| 延迟 p99 (ms) | 1,473 | 1,471 | 1.00× |
| 启动到开始监听 (ms) | 19 | 20 | 1.07× |
| 工作集，第 1 段中位数 (MiB) | 347 | 341 | 0.98× |
| 工作集，第 2 段中位数 (MiB) | 352 | 337 | 0.96× |
| 工作集，第 3 段中位数 (MiB) | 344 | 347 | 1.01× |
| 工作集，第 4 段中位数 (MiB) | 346 | 349 | 1.01× |
| 第 2 段到第 4 段的增长 (MiB) | -6.6 | 12 | - |
| 峰值工作集 (MiB) | 384 | 397 | 1.03× |
| 峰值线程数 | 12 | 12 | 1.00× |
| CPU 时间 (ms) | 137,406 | 141,391 | 1.03× |

各路由：请求次数，延迟 p50 / p95（ms）。

| 路由 | base | head | p50 head / base |
|---|---|---|---|
| `/api/chat/messages`，大群任意一页 | 673 次，165 / 368 | 693 次，163 / 350 | 0.99× |
| `/api/chat/messages`，其他会话第一页 | 423 次，15 / 145 | 433 次，15 / 155 | 1.00× |
| `/api/chat/sessions` | 263 次，45 / 252 | 275 次，44 / 266 | 0.96× |
| `/api/chat/search`，全部会话 | 189 次，901 / 1,718 | 191 次，877 / 1,701 | 0.97× |
| `/api/chat/date-counts` | 185 次，13 / 137 | 190 次，13 / 144 | 0.99× |
| `/api/chat/contacts` | 199 次，27 / 45 | 207 次，26 / 44 | 0.98× |

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 0，head 0。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。

| 路由 | base | head | head / base |
|---|---|---|---|
| `16 pages at once (all of them) [api]` | 52 | 52 | 0.99× |
| `16 pages at once (all of them) [sub]` | 80 | 80 | 1.00× |
| `20 images at once (all of them) [api]` | 14 | 14 | 1.01× |
| `20 images at once (all of them) [sub]` | 48 | 49 | 1.02× |
| `analytics overall (cached) [api]` | 2.3 | 2.3 | 1.01× |
| `analytics overall (cached) [sub]` | 26 | 26 | 1.00× |
| `analytics overall --force [api]` | 467 | 479 | 1.03× |
| `analytics overall --force [sub]` | 831 | 818 | 0.98× |
| `analytics rankings (cached) [api]` | 8.0 | 8.4 | 1.04× |
| `analytics rankings (cached) [sub]` | 34 | 34 | 1.01× |
| `analytics time (cached) [api]` | 2.4 | 2.4 | 1.00× |
| `analytics time (cached) [sub]` | 26 | 26 | 1.01× |
| `biz pay-records [api]` | 0.9 | 0.9 | 1.02× |
| `biz pay-records [sub]` | 35 | 36 | 1.02× |
| `cache list [api]` | 0.7 | 0.7 | 0.97× |
| `cache list [sub]` | 21 | 22 | 1.05× |
| `chat date-counts [api]` | 71 | 72 | 1.00× |
| `chat date-counts [sub]` | 639 | 636 | 1.00× |
| `chat dates [api]` | 72 | 72 | 1.00× |
| `chat dates [sub]` | 638 | 639 | 1.00× |
| `chat detail --fast [api]` | 1.4 | 1.5 | 1.07× |
| `chat detail --fast [sub]` | 72 | 73 | 1.01× |
| `chat resources, all conversations [api]` | 93 | 95 | 1.02× |
| `chat resources, all conversations [sub]` | 233 | 233 | 1.00× |
| `chat resources, all conversations, far page [api]` | 168 | 169 | 1.01× |
| `chat resources, all conversations, far page [sub]` | 317 | 320 | 1.01× |
| `chat resources, all conversations, file [api]` | 118 | 121 | 1.02× |
| `chat resources, all conversations, file [sub]` | 230 | 231 | 1.00× |
| `chat resources, all conversations, image [api]` | 61 | 61 | 1.01× |
| `chat resources, all conversations, image [sub]` | 133 | 132 | 0.99× |
| `chat resources, all conversations, video [api]` | 63 | 66 | 1.05× |
| `chat resources, all conversations, video [sub]` | 152 | 153 | 1.01× |
| `chat resources, all conversations, voice [api]` | 34 | 35 | 1.02× |
| `chat resources, all conversations, voice [sub]` | 107 | 110 | 1.03× |
| `chat resources, one conversation [api]` | 56 | 56 | 1.00× |
| `chat resources, one conversation [sub]` | 184 | 185 | 1.01× |
| `chat statuses of 60 conversations [api]` | 1.7 | 1.7 | 1.02× |
| `chat statuses of 60 conversations [sub]` | 25 | 25 | 1.03× |
| `chat tab-counts [api]` | 2.0 | 1.9 | 0.98× |
| `chat tab-counts [sub]` | 25 | 26 | 1.03× |
| `config profiles [api]` | 0.6 | 0.6 | 0.96× |
| `config profiles [sub]` | 21 | 21 | 1.02× |
| `contacts, all [api]` | 16 | 15 | 0.93× |
| `contacts, all [sub]` | 57 | 57 | 1.00× |
| `contacts, all, br [api]` | 28 | 24 | 0.88× |
| `contacts, all, gzip [api]` | 20 | 19 | 0.93× |
| `contacts, all, the fields of a list [api]` | 14 | 13 | 0.91× |
| `contacts, all, the fields of a list, br [api]` | 17 | 16 | 0.93× |
| `counts of 60 conversations [api]` | 10 | 10 | 0.97× |
| `counts of 60 conversations [sub]` | 88 | 88 | 1.00× |
| `db test [api]` | 0.6 | 0.6 | 0.99× |
| `db test [sub]` | 22 | 22 | 1.02× |
| `first conversation opened after start (first page) [api]` | 8.8 | 9.5 | 1.08× |
| `first page, 1 at once (all of them) [api]` | 4.2 | 4.2 | 0.99× |
| `first page, 1 at once (all of them) [sub]` | 45 | 46 | 1.02× |
| `first page, 16 at once (all of them) [api]` | 37 | 39 | 1.04× |
| `first page, 16 at once (all of them) [sub]` | 76 | 77 | 1.02× |
| `first page, 32 at once (all of them) [api]` | 70 | 72 | 1.04× |
| `first page, 32 at once (all of them) [sub]` | 78 | 80 | 1.02× |
| `first page, 6 at once (all of them) [api]` | 15 | 16 | 1.03× |
| `first page, 6 at once (all of them) [sub]` | 75 | 76 | 1.01× |
| `first page, 64 at once (all of them) [api]` | 140 | 144 | 1.03× |
| `first read after start (sessions, 50) [api]` | 172 | 167 | 0.97× |
| `group list [api]` | 4.2 | 4.0 | 0.97× |
| `group list [sub]` | 29 | 28 | 0.99× |
| `group members [api]` | 1.8 | 1.7 | 0.94× |
| `group members [sub]` | 40 | 41 | 1.00× |
| `image, again (each) [api]` | 1.5 | 1.5 | 1.05× |
| `image, again (each) [sub]` | 45 | 46 | 1.02× |
| `image, first time (each) [api]` | 2.5 | 3.6 | 1.44× |
| `image, first time (each) [sub]` | 46 | 47 | 1.03× |
| `latest 20 (polling) [api]` | 2.9 | 2.9 | 1.01× |
| `latest 20 (polling) [sub]` | 43 | 43 | 1.00× |
| `messages, first page [api]` | 4.1 | 4.0 | 0.96× |
| `messages, first page [sub]` | 44 | 46 | 1.03× |
| `messages, first page, br [api]` | 6.7 | 5.8 | 0.87× |
| `messages, first page, gzip [api]` | 4.7 | 4.8 | 1.03× |
| `messages, first page, the fields shown [api]` | 3.9 | 3.9 | 1.00× |
| `messages, first page, the fields shown, br [api]` | 5.6 | 5.7 | 1.02× |
| `messages, page in the middle [api]` | 103 | 103 | 1.00× |
| `messages, page in the middle [sub]` | 508 | 507 | 1.00× |
| `messages, page in the middle, from a cursor [api]` | 4.1 | 4.1 | 1.01× |
| `messages, page in the middle, from a cursor [sub]` | 45 | 45 | 1.00× |
| `one contact [api]` | 0.9 | 0.9 | 1.03× |
| `one contact [sub]` | 23 | 23 | 0.99× |
| `one message [api]` | 1.2 | 1.2 | 0.99× |
| `one message [sub]` | 39 | 40 | 1.01× |
| `push engine, CPU ms per second while listened to [api]` | 0.0 | 0.0 | - |
| `report annual years [api]` | 107 | 110 | 1.02× |
| `report annual years [sub]` | 455 | 450 | 0.99× |
| `search in the conversation [api]` | 3.3 | 3.2 | 0.98× |
| `search in the conversation [sub]` | 45 | 44 | 0.99× |
| `search in the conversation, a word found nowhere [api]` | 64 | 65 | 1.01× |
| `search in the conversation, a word found nowhere [sub]` | 186 | 185 | 0.99× |
| `server start (until it listens) [-]` | 21 | 21 | 1.01× |
| `sessions, 50 [api]` | 14 | 15 | 1.05× |
| `sessions, 50 [sub]` | 57 | 56 | 0.99× |
| `sessions, all [api]` | 14 | 15 | 1.04× |
| `sessions, all [sub]` | 59 | 58 | 0.99× |
| `sns post-counts --prefer-cache [api]` | 0.6 | 0.6 | 1.00× |
| `sns post-counts --prefer-cache [sub]` | 39 | 39 | 1.00× |
| `sns stats --fast [api]` | 0.5 | 0.5 | 1.00× |
| `sns stats --fast [sub]` | 38 | 39 | 1.02× |
| `sns timeline, 20 [api]` | 8.5 | 8.6 | 1.01× |
| `sns timeline, 20 [sub]` | 94 | 95 | 1.01× |
| `sns timeline, 20, far page [api]` | 7.6 | 7.8 | 1.03× |
| `sns timeline, 20, far page [sub]` | 93 | 94 | 1.01× |
| `sns users [api]` | 4.0 | 3.9 | 0.98× |
| `sns users [sub]` | 40 | 40 | 1.01× |
| `thumbnail of the image (each) [api]` | 1.3 | 1.4 | 1.04× |
| `thumbnail of the image (each) [sub]` | 45 | 46 | 1.03× |
