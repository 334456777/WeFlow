# 基准测试：base 与 head

- base：`e095929773`，weflow 5.0.1-185-ge0959297
- head：`HEAD`（`a8aac15235`），weflow 5.0.1-206-ga8aac152
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 10 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 30.7 MiB | 30.3 MiB | 0.99× |

## 命令

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 26,110（25,342–28,748） | 26,194（25,094–30,733） | 1.00× |
| `export messages json with images` | 30,944（30,011–31,288） | 29,583（29,127–30,345） | 0.96× |
| `export messages json` | 5,310（5,260–5,468） | 4,557（4,492–4,612） | 0.86× ▼ |
| `export messages chatlab` | 4,708（4,661–4,779） | 4,018（3,944–4,079） | 0.85× ▼ |
| `export messages excel` | 5,687（5,664–5,817） | 5,335（5,309–5,401） | 0.94× |
| `export messages html` | 4,082（4,029–4,180） | 3,683（3,615–3,755） | 0.90× |
| `export messages txt` | 3,951（3,900–4,003） | 3,387（3,369–3,485） | 0.86× ▼ |
| `report annual` | 2,564（2,537–2,595） | 2,557（2,534–2,581） | 1.00× |
| `analytics overall` | 1,101（1,083–1,104） | 1,094（1,080–1,120） | 0.99× |
| `group members` | 1,155（1,145–1,167） | 1,155（1,137–1,184） | 1.00× |
| `group ranking` | 1,133（1,113–1,155） | 1,131（1,123–1,155） | 1.00× |
| `chat export-stats` | 1,020（1,005–1,032） | 1,028（1,006–1,054） | 1.01× |
| `chat date-counts` | 861（843–873） | 862（851–872） | 1.00× |
| `chat search` | 558（551–568） | 560（554–569） | 1.00× |
| `report dual` | 439（427–450） | 360（349–368） | 0.82× ▼ |
| `chat messages first page` | 191（191–200） | 192（191–204） | 1.01× |
| `chat messages offset` | 720（708–737） | 724（714–733） | 1.01× |
| `chat messages cursor` | 192（191–200） | 191（190–197） | 0.99× |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 2,268 | 2,147 | 0.95× |
| `export messages json with images` | 564 | 514 | 0.91× |
| `export messages json` | 524 | 500 | 0.95× |
| `export messages chatlab` | 566 | 548 | 0.97× |
| `export messages excel` | 521 | 495 | 0.95× |
| `export messages html` | 511 | 495 | 0.97× |
| `export messages txt` | 523 | 490 | 0.94× |
| `report annual` | 18 | 18 | 1.00× |
| `analytics overall` | 172 | 168 | 0.97× |
| `report dual` | 19 | 18 | 0.99× |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 18,094 | 17,938 | 0.99× |
| `export messages json with images` | 26,633 | 24,867 | 0.93× |
| `export messages json` | 12,328 | 9,969 | 0.81× ▼ |
| `export messages chatlab` | 11,102 | 9,109 | 0.82× ▼ |
| `export messages excel` | 12,422 | 10,367 | 0.83× ▼ |
| `export messages html` | 11,711 | 9,562 | 0.82× ▼ |
| `export messages txt` | 11,016 | 9,375 | 0.85× ▼ |
| `report annual` | 2,156 | 2,203 | 1.02× |
| `analytics overall` | 938 | 914 | 0.97× |
| `group members` | 977 | 938 | 0.96× |
| `group ranking` | 961 | 969 | 1.01× |
| `chat export-stats` | 844 | 828 | 0.98× |
| `chat date-counts` | 695 | 672 | 0.97× |
| `chat search` | 438 | 438 | 1.00× |
| `report dual` | 375 | 281 | 0.75× ▼ |
| `chat messages first page` | 164 | 164 | 1.00× |
| `chat messages offset` | 578 | 586 | 1.01× |
| `chat messages cursor` | 156 | 156 | 1.00× |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 35,109 | 35,320 | 1.01× |
| `export messages json with images` | 36,172 | 35,992 | 1.00× |
| `export messages json` | 969 | 961 | 0.99× |
| `export messages chatlab` | 805 | 773 | 0.96× |
| `export messages excel` | 1,125 | 1,031 | 0.92× |
| `export messages html` | 852 | 828 | 0.97× |
| `export messages txt` | 805 | 781 | 0.97× |
| `report annual` | 406 | 344 | 0.85× ▼ |
| `analytics overall` | 172 | 156 | 0.91× |
| `group members` | 195 | 211 | 1.08× |
| `group ranking` | 188 | 164 | 0.88× |
| `chat export-stats` | 188 | 219 | 1.17× |
| `chat date-counts` | 164 | 172 | 1.05× |
| `chat search` | 102 | 133 | 1.31× |
| `report dual` | 78 | 62 | 0.80× |
| `chat messages first page` | 23 | 31 | 1.33× |
| `chat messages offset` | 156 | 125 | 0.80× |
| `chat messages cursor` | 31 | 31 | 1.00× |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 295 | 198 | 0.67× ▼ |
| `export messages json with images` | 616 | 588 | 0.95× |
| `export messages json` | 266 | 229 | 0.86× ▼ |
| `export messages chatlab` | 269 | 232 | 0.86× ▼ |
| `export messages excel` | 291 | 250 | 0.86× ▼ |
| `export messages html` | 268 | 237 | 0.89× ▼ |
| `export messages txt` | 271 | 240 | 0.89× ▼ |
| `report annual` | 132 | 132 | 1.00× |
| `analytics overall` | 117 | 117 | 1.00× |
| `group members` | 206 | 206 | 1.00× |
| `group ranking` | 198 | 198 | 1.00× |
| `chat export-stats` | 198 | 198 | 1.00× |
| `chat date-counts` | 195 | 195 | 1.00× |
| `chat search` | 58.6 | 58.6 | 1.00× |
| `report dual` | 50.6 | 48.1 | 0.95× |
| `chat messages first page` | 31.6 | 31.7 | 1.00× |
| `chat messages offset` | 145 | 145 | 1.00× |
| `chat messages cursor` | 31.9 | 31.6 | 0.99× |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 438 | 396 | 0.91× |
| `export messages json with images` | 827 | 817 | 0.99× |
| `export messages json` | 458 | 417 | 0.91× |
| `export messages chatlab` | 445 | 425 | 0.95× |
| `export messages excel` | 474 | 433 | 0.91× |
| `export messages html` | 458 | 426 | 0.93× |
| `export messages txt` | 461 | 436 | 0.95× |
| `report annual` | 276 | 276 | 1.00× |
| `analytics overall` | 242 | 242 | 1.00× |
| `group members` | 339 | 339 | 1.00× |
| `group ranking` | 328 | 328 | 1.00× |
| `chat export-stats` | 328 | 328 | 1.00× |
| `chat date-counts` | 324 | 324 | 1.00× |
| `chat search` | 187 | 186 | 1.00× |
| `report dual` | 189 | 197 | 1.04× |
| `chat messages first page` | 157 | 157 | 1.00× |
| `chat messages offset` | 292 | 292 | 1.00× |
| `chat messages cursor` | 157 | 158 | 1.01× |

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
| `export messages json with images` | 1,469 | 1,470 | 1.00× |
| `export messages json` | 812 | 813 | 1.00× |
| `export messages chatlab` | 651 | 654 | 1.00× |
| `export messages excel` | 688 | 687 | 1.00× |
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
| `chat search` | 0.1 | 0.1 | 0.92× |
| `report dual` | 0.0 | 0.0 | 1.00× |
| `chat messages first page` | 0.1 | 0.1 | 0.82× |
| `chat messages offset` | 0.1 | 0.1 | 0.86× |
| `chat messages cursor` | 0.1 | 0.1 | 0.86× |

### 页错误

进程及其子进程，包括软页错误。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 142,294 | 123,478 | 0.87× ▼ |
| `export messages json with images` | 253,503 | 256,938 | 1.01× |
| `export messages json` | 110,828 | 88,555 | 0.80× ▼ |
| `export messages chatlab` | 108,128 | 85,730 | 0.79× ▼ |
| `export messages excel` | 117,597 | 96,617 | 0.82× ▼ |
| `export messages html` | 107,153 | 89,355 | 0.83× ▼ |
| `export messages txt` | 107,392 | 88,494 | 0.82× ▼ |
| `report annual` | 36,299 | 36,274 | 1.00× |
| `analytics overall` | 31,064 | 31,068 | 1.00× |
| `group members` | 58,684 | 58,671 | 1.00× |
| `group ranking` | 56,546 | 56,511 | 1.00× |
| `chat export-stats` | 54,570 | 54,700 | 1.00× |
| `chat date-counts` | 52,394 | 52,373 | 1.00× |
| `chat search` | 15,214 | 15,218 | 1.00× |
| `report dual` | 21,648 | 16,856 | 0.78× ▼ |
| `chat messages first page` | 8,348 | 8,421 | 1.01× |
| `chat messages offset` | 37,370 | 37,328 | 1.00× |
| `chat messages cursor` | 8,474 | 8,373 | 0.99× |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | 6,463 | 6,761 | 1.05× |
| `export messages json` | 37,667 | 43,887 | 1.17× ▲ |
| `export messages chatlab` | 42,479 | 49,774 | 1.17× ▲ |
| `export messages excel` | 35,169 | 37,490 | 1.07× |
| `export messages html` | 48,990 | 54,299 | 1.11× ▲ |
| `export messages txt` | 50,625 | 59,049 | 1.17× ▲ |
| `report annual` | 127,090 | 127,434 | 1.00× |
| `group members` | 173,189 | 173,158 | 1.00× |
| `group ranking` | 176,567 | 176,776 | 1.00× |
| `chat export-stats` | 195,991 | 194,619 | 0.99× |
| `chat date-counts` | 232,413 | 232,092 | 1.00× |
| `report dual` | 22,755 | 27,742 | 1.22× ▲ |
| `chat messages first page` | 261 | 260 | 0.99× |
| `chat messages offset` | 69 | 69 | 0.99× |
| `chat messages cursor` | 260 | 262 | 1.01× |

### 分配次数

分配和重新分配的次数。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 33,890,993 | 33,047,710 | 0.98× |
| `export messages json with images` | 61,251,114 | 45,645,601 | 0.75× ▼ |
| `export messages json` | 30,093,545 | 14,481,531 | 0.48× ▼ |
| `export messages chatlab` | 26,119,772 | 14,471,196 | 0.55× ▼ |
| `export messages excel` | 24,283,688 | 16,631,218 | 0.68× ▼ |
| `export messages html` | 26,067,292 | 14,765,752 | 0.57× ▼ |
| `export messages txt` | 20,528,340 | 12,875,893 | 0.63× ▼ |
| `report annual` | 10,160,502 | 10,061,534 | 0.99× |
| `analytics overall` | 2,425,673 | 2,425,672 | 1.00× |
| `group members` | 123,390 | 123,390 | 1.00× |
| `group ranking` | 23,634 | 23,634 | 1.00× |
| `chat export-stats` | 105,522 | 105,522 | 1.00× |
| `chat date-counts` | 15,956 | 15,956 | 1.00× |
| `chat search` | 45,991 | 44,969 | 0.98× |
| `report dual` | 626,817 | 282,904 | 0.45× ▼ |
| `chat messages first page` | 11,668 | 8,193 | 0.70× ▼ |
| `chat messages offset` | 17,874 | 14,704 | 0.82× ▼ |
| `chat messages cursor` | 11,217 | 8,047 | 0.72× ▼ |

### 累计分配（MiB）

每次分配的大小之和，重新分配只计增长的部分。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 8,408 | 8,267 | 0.98× |
| `export messages json with images` | 47,882 | 45,495 | 0.95× |
| `export messages json` | 43,287 | 40,898 | 0.94× |
| `export messages chatlab` | 41,382 | 39,937 | 0.97× |
| `export messages excel` | 41,182 | 39,972 | 0.97× |
| `export messages html` | 40,163 | 38,525 | 0.96× |
| `export messages txt` | 40,189 | 38,980 | 0.97× |
| `report annual` | 1,171 | 1,166 | 1.00× |
| `analytics overall` | 400 | 400 | 1.00× |
| `group members` | 384 | 384 | 1.00× |
| `group ranking` | 375 | 375 | 1.00× |
| `chat export-stats` | 384 | 384 | 1.00× |
| `chat date-counts` | 373 | 373 | 1.00× |
| `chat search` | 204 | 204 | 1.00× |
| `report dual` | 1,876 | 622 | 0.33× ▼ |
| `chat messages first page` | 59.4 | 59.1 | 0.99× |
| `chat messages offset` | 342 | 341 | 1.00× |
| `chat messages cursor` | 57.7 | 57.5 | 1.00× |

### 峰值存活堆（MiB）

同一时刻仍未释放的分配之和的最大值。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 182 | 94.7 | 0.52× ▼ |
| `export messages json with images` | 424 | 424 | 1.00× |
| `export messages json` | 77.4 | 63.4 | 0.82× ▼ |
| `export messages chatlab` | 73.2 | 62.1 | 0.85× ▼ |
| `export messages excel` | 104 | 81.5 | 0.79× ▼ |
| `export messages html` | 77.2 | 63.2 | 0.82× ▼ |
| `export messages txt` | 77.2 | 63.2 | 0.82× ▼ |
| `report annual` | 15.8 | 15.6 | 0.99× |
| `analytics overall` | 11.8 | 11.8 | 1.00× |
| `group members` | 11.8 | 10.0 | 0.85× |
| `group ranking` | 10.6 | 10.6 | 1.00× |
| `chat export-stats` | 14.6 | 14.6 | 1.00× |
| `chat date-counts` | 10.6 | 10.6 | 1.00× |
| `chat search` | 10.6 | 10.6 | 1.00× |
| `report dual` | 12.4 | 11.7 | 0.95× |
| `chat messages first page` | 10.6 | 10.6 | 1.00× |
| `chat messages offset` | 15.3 | 15.3 | 1.00× |
| `chat messages cursor` | 10.6 | 10.6 | 1.00× |

## `weflow serve` 持续负载

每个版本启动一个 `weflow serve`，用 4 个并发连续请求 90 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。

|  | base | head | head / base |
|---|---|---|---|
| 每秒请求数 | 19 | 19 | 1.02× |
| 错误数 | 0 | 0 | - |
| 延迟 p50 (ms) | 56 | 55 | 0.98× |
| 延迟 p95 (ms) | 1,222 | 1,205 | 0.99× |
| 延迟 p99 (ms) | 2,138 | 1,983 | 0.93× |
| 启动到开始监听 (ms) | 20 | 21 | 1.06× |
| 工作集，第 1 段中位数 (MiB) | 323 | 335 | 1.04× |
| 工作集，第 2 段中位数 (MiB) | 335 | 337 | 1.01× |
| 工作集，第 3 段中位数 (MiB) | 337 | 373 | 1.11× ▲ |
| 工作集，第 4 段中位数 (MiB) | 342 | 371 | 1.08× |
| 第 2 段到第 4 段的增长 (MiB) | 7.7 | 34 | - |
| 峰值工作集 (MiB) | 374 | 407 | 1.09× |
| 峰值线程数 | 12 | 13 | 1.08× |
| CPU 时间 (ms) | 144,422 | 144,062 | 1.00× |

各路由：请求次数，延迟 p50 / p95（ms）。

| 路由 | base | head | p50 head / base |
|---|---|---|---|
| `/api/chat/messages`，大群任意一页 | 577 次，165 / 366 | 594 次，162 / 381 | 0.98× |
| `/api/chat/messages`，其他会话第一页 | 369 次，5.0 / 139 | 375 次，4.6 / 148 | 0.92× |
| `/api/chat/sessions` | 232 次，43 / 278 | 236 次，37 / 259 | 0.86× ▼ |
| `/api/chat/search`，全部会话 | 162 次，1,233 / 2,336 | 168 次，1,217 / 2,284 | 0.99× |
| `/api/chat/date-counts` | 163 次，3.8 / 155 | 165 次，3.9 / 152 | 1.03× |
| `/api/chat/contacts` | 178 次，23 / 44 | 181 次，23 / 41 | 1.00× |

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 0，head 0。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。

| 路由 | base | head | head / base |
|---|---|---|---|
| `16 pages at once (all of them) [api]` | 50 | 45 | 0.89× ▼ |
| `16 pages at once (all of them) [sub]` | 84 | 84 | 1.00× |
| `20 images at once (all of them) [api]` | 17 | 17 | 0.98× |
| `20 images at once (all of them) [sub]` | 50 | 50 | 0.99× |
| `analytics overall (cached) [api]` | 2.6 | 3.0 | 1.12× |
| `analytics overall (cached) [sub]` | 24 | 25 | 1.05× |
| `analytics overall --force [api]` | 545 | 557 | 1.02× |
| `analytics overall --force [sub]` | 934 | 947 | 1.01× |
| `analytics rankings (cached) [api]` | 8.8 | 9.8 | 1.11× |
| `analytics rankings (cached) [sub]` | 32 | 34 | 1.05× |
| `analytics time (cached) [api]` | 2.8 | 3.0 | 1.06× |
| `analytics time (cached) [sub]` | 24 | 26 | 1.08× |
| `biz pay-records [api]` | 1.0 | 1.0 | 1.00× |
| `biz pay-records [sub]` | 37 | 37 | 1.00× |
| `cache list [api]` | 0.8 | 0.8 | 1.00× |
| `cache list [sub]` | 21 | 22 | 1.05× |
| `chat date-counts [api]` | 79 | 80 | 1.02× |
| `chat date-counts [sub]` | 706 | 716 | 1.01× |
| `chat dates [api]` | 78 | 80 | 1.03× |
| `chat dates [sub]` | 704 | 718 | 1.02× |
| `chat detail --fast [api]` | 1.5 | 1.5 | 1.02× |
| `chat detail --fast [sub]` | 92 | 92 | 1.01× |
| `chat resources, all conversations [api]` | 99 | 85 | 0.86× ▼ |
| `chat resources, all conversations [sub]` | 271 | 262 | 0.97× |
| `chat resources, all conversations, far page [api]` | 169 | 153 | 0.91× |
| `chat resources, all conversations, far page [sub]` | 361 | 346 | 0.96× |
| `chat resources, all conversations, file [api]` | 125 | 117 | 0.93× |
| `chat resources, all conversations, file [sub]` | 260 | 249 | 0.96× |
| `chat resources, all conversations, image [api]` | 60 | 56 | 0.93× |
| `chat resources, all conversations, image [sub]` | 135 | 131 | 0.97× |
| `chat resources, all conversations, video [api]` | 72 | 64 | 0.90× ▼ |
| `chat resources, all conversations, video [sub]` | 172 | 167 | 0.97× |
| `chat resources, all conversations, voice [api]` | 38 | 34 | 0.90× |
| `chat resources, all conversations, voice [sub]` | 122 | 120 | 0.98× |
| `chat resources, one conversation [api]` | 62 | 49 | 0.80× ▼ |
| `chat resources, one conversation [sub]` | 226 | 213 | 0.94× |
| `chat statuses of 60 conversations [api]` | 1.7 | 1.7 | 0.97× |
| `chat statuses of 60 conversations [sub]` | 24 | 25 | 1.05× |
| `chat tab-counts [api]` | 2.1 | 2.3 | 1.14× |
| `chat tab-counts [sub]` | 24 | 25 | 1.07× |
| `config profiles [api]` | 0.6 | 0.6 | 0.95× |
| `config profiles [sub]` | 20 | 19 | 0.93× |
| `contacts, all [api]` | 15 | 14 | 0.93× |
| `contacts, all [sub]` | 62 | 56 | 0.90× ▼ |
| `contacts, all, br [api]` | 26 | 26 | 1.00× |
| `contacts, all, gzip [api]` | 20 | 19 | 0.98× |
| `contacts, all, the fields of a list [api]` | 15 | 14 | 0.97× |
| `contacts, all, the fields of a list, br [api]` | 17 | 17 | 1.00× |
| `counts of 60 conversations [api]` | 17 | 17 | 1.00× |
| `counts of 60 conversations [sub]` | 118 | 119 | 1.01× |
| `db test [api]` | 0.7 | 0.7 | 0.96× |
| `db test [sub]` | 21 | 21 | 1.02× |
| `first conversation opened after start (first page) [api]` | 12 | 11 | 0.91× |
| `first page, 1 at once (all of them) [api]` | 4.3 | 3.7 | 0.86× |
| `first page, 1 at once (all of them) [sub]` | 48 | 47 | 0.98× |
| `first page, 16 at once (all of them) [api]` | 38 | 31 | 0.83× ▼ |
| `first page, 16 at once (all of them) [sub]` | 85 | 84 | 0.99× |
| `first page, 32 at once (all of them) [api]` | 73 | 62 | 0.86× ▼ |
| `first page, 32 at once (all of them) [sub]` | 86 | 81 | 0.94× |
| `first page, 6 at once (all of them) [api]` | 16 | 13 | 0.80× |
| `first page, 6 at once (all of them) [sub]` | 83 | 81 | 0.97× |
| `first page, 64 at once (all of them) [api]` | 145 | 122 | 0.84× ▼ |
| `first read after start (sessions, 50) [api]` | 179 | 172 | 0.96× |
| `group list [api]` | 6.2 | 6.3 | 1.01× |
| `group list [sub]` | 29 | 30 | 1.03× |
| `group members [api]` | 1.8 | 1.8 | 0.98× |
| `group members [sub]` | 41 | 40 | 1.00× |
| `image, again (each) [api]` | 1.7 | 1.6 | 0.96× |
| `image, again (each) [sub]` | 47 | 46 | 0.97× |
| `image, first time (each) [api]` | 3.4 | 2.9 | 0.87× |
| `image, first time (each) [sub]` | 48 | 47 | 0.97× |
| `latest 20 (polling) [api]` | 3.1 | 2.5 | 0.82× |
| `latest 20 (polling) [sub]` | 45 | 44 | 0.98× |
| `messages, first page [api]` | 4.1 | 3.4 | 0.83× |
| `messages, first page [sub]` | 47 | 45 | 0.95× |
| `messages, first page, br [api]` | 6.1 | 5.2 | 0.84× |
| `messages, first page, gzip [api]` | 4.9 | 4.3 | 0.87× |
| `messages, first page, the fields shown [api]` | 4.0 | 3.3 | 0.83× |
| `messages, first page, the fields shown, br [api]` | 5.9 | 5.3 | 0.90× |
| `messages, page in the middle [api]` | 118 | 112 | 0.95× |
| `messages, page in the middle [sub]` | 569 | 563 | 0.99× |
| `messages, page in the middle, from a cursor [api]` | 4.4 | 3.3 | 0.74× |
| `messages, page in the middle, from a cursor [sub]` | 48 | 45 | 0.95× |
| `one contact [api]` | 1.0 | 0.9 | 0.89× |
| `one contact [sub]` | 24 | 22 | 0.95× |
| `one message [api]` | 1.3 | 1.1 | 0.90× |
| `one message [sub]` | 40 | 40 | 0.99× |
| `push engine, CPU ms per second while listened to [api]` | 0.0 | 0.0 | - |
| `report annual years [api]` | 153 | 151 | 0.99× |
| `report annual years [sub]` | 533 | 540 | 1.01× |
| `search in the conversation [api]` | 3.5 | 3.4 | 0.97× |
| `search in the conversation [sub]` | 46 | 47 | 1.02× |
| `search in the conversation, a word found nowhere [api]` | 70 | 70 | 0.99× |
| `search in the conversation, a word found nowhere [sub]` | 209 | 212 | 1.01× |
| `server start (until it listens) [-]` | 19 | 20 | 1.01× |
| `sessions, 50 [api]` | 17 | 17 | 0.97× |
| `sessions, 50 [sub]` | 58 | 58 | 1.00× |
| `sessions, all [api]` | 18 | 17 | 0.97× |
| `sessions, all [sub]` | 61 | 60 | 0.99× |
| `sns post-counts --prefer-cache [api]` | 0.8 | 0.6 | 0.82× |
| `sns post-counts --prefer-cache [sub]` | 39 | 38 | 0.97× |
| `sns stats --fast [api]` | 0.5 | 0.5 | 0.97× |
| `sns stats --fast [sub]` | 39 | 39 | 1.01× |
| `sns timeline, 20 [api]` | 9.2 | 9.0 | 0.97× |
| `sns timeline, 20 [sub]` | 93 | 92 | 0.99× |
| `sns timeline, 20, far page [api]` | 8.3 | 8.0 | 0.96× |
| `sns timeline, 20, far page [sub]` | 92 | 92 | 1.00× |
| `sns users [api]` | 4.8 | 5.1 | 1.05× |
| `sns users [sub]` | 39 | 40 | 1.01× |
| `thumbnail of the image (each) [api]` | 1.5 | 1.4 | 0.93× |
| `thumbnail of the image (each) [sub]` | 46 | 45 | 0.99× |
