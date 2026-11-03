# 基准测试：base 与 head

- base：`a19a4391840e8fe5fe01eea8ad7586d9a6c1945f`（`a19a439184`），weflow 5.0.1-222-ga19a4391
- head：`bba77916edd6314b1fb25124e843caf70ce8fe3b`（`bba77916ed`），weflow 5.0.1-229-gbba77916
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 5 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 29.5 MiB | 29.5 MiB | 1.00× |

## 命令

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 16,119（15,937–16,549） | 15,846（15,507–16,627） | 0.98× |
| `export messages json with images` | 18,584（17,984–23,175） | 17,799（17,017–19,097） | 0.96× |
| `export messages json` | 2,897（2,843–2,935） | 2,475（2,370–2,613） | 0.85× ▼ |
| `export messages chatlab` | 2,343（2,295–2,444） | 2,090（1,955–2,281） | 0.89× ▼ |
| `export messages excel` | 3,277（3,210–3,295） | 2,927（2,856–2,980） | 0.89× ▼ |
| `export messages html` | 2,207（2,129–2,258） | 1,987（1,922–2,268） | 0.90× |
| `export messages txt` | 2,008（1,952–2,015） | 1,767（1,740–2,458） | 0.88× ▼，范围重叠 |
| `report annual` | 1,415（1,367–1,638） | 1,408（1,350–1,612） | 0.99× |
| `analytics overall` | 596（591–605） | 605（579–696） | 1.02× |
| `group members` | 659（636–703） | 652（640–697） | 0.99× |
| `group ranking` | 632（627–645） | 654（650–688） | 1.04× |
| `chat export-stats` | 556（552–607） | 559（557–574） | 1.01× |
| `chat date-counts` | 450（449–453） | 451（447–458） | 1.00× |
| `chat search` | 266（264–278） | 271（267–279） | 1.02× |
| `report dual` | 208（208–213） | 209（200–213） | 1.00× |
| `chat messages first page` | 108（108–115） | 111（108–115） | 1.02× |
| `chat messages offset` | 378（376–387） | 380（376–389） | 1.01× |
| `chat messages cursor` | 110（108–115） | 109（107–110） | 0.99× |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 1,318 | 957 | 0.73× ▼ |
| `export messages json with images` | 318 | 269 | 0.84× ▼ |
| `export messages json` | 287 | 295 | 1.03× |
| `export messages chatlab` | 300 | 284 | 0.95× |
| `export messages excel` | 281 | 256 | 0.91× |
| `export messages html` | 276 | 265 | 0.96× |
| `export messages txt` | 273 | 253 | 0.93× |
| `report annual` | 15 | 14 | 0.96× |
| `analytics overall` | 99 | 99 | 1.00× |
| `report dual` | 14 | 14 | 1.00× |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 9,375 | 8,828 | 0.94× |
| `export messages json with images` | 13,750 | 12,469 | 0.91× |
| `export messages json` | 6,281 | 5,141 | 0.82× ▼ |
| `export messages chatlab` | 5,312 | 4,297 | 0.81× ▼ |
| `export messages excel` | 6,281 | 4,766 | 0.76× ▼ |
| `export messages html` | 5,734 | 4,484 | 0.78× ▼ |
| `export messages txt` | 5,531 | 4,328 | 0.78× ▼ |
| `report annual` | 1,172 | 1,266 | 1.08× |
| `analytics overall` | 484 | 484 | 1.00× |
| `group members` | 469 | 516 | 1.10× |
| `group ranking` | 516 | 547 | 1.06× |
| `chat export-stats` | 469 | 422 | 0.90× |
| `chat date-counts` | 344 | 312 | 0.91× |
| `chat search` | 156 | 203 | 1.30× |
| `report dual` | 172 | 141 | 0.82× |
| `chat messages first page` | 78 | 78 | 1.00× |
| `chat messages offset` | 281 | 328 | 1.17× |
| `chat messages cursor` | 78 | 94 | 1.20× |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 16,766 | 16,406 | 0.98× |
| `export messages json with images` | 17,500 | 17,641 | 1.01× |
| `export messages json` | 672 | 891 | 1.33× ▲ |
| `export messages chatlab` | 547 | 562 | 1.03× |
| `export messages excel` | 609 | 641 | 1.05× |
| `export messages html` | 484 | 594 | 1.23× ▲ |
| `export messages txt` | 469 | 625 | 1.33× ▲ |
| `report annual` | 281 | 203 | 0.72× ▼ |
| `analytics overall` | 94 | 141 | 1.50× |
| `group members` | 156 | 141 | 0.90× |
| `group ranking` | 125 | 109 | 0.88× |
| `chat export-stats` | 94 | 141 | 1.50× |
| `chat date-counts` | 78 | 141 | 1.80× ▲ |
| `chat search` | 109 | 62 | 0.57× |
| `report dual` | 47 | 62 | 1.33× |
| `chat messages first page` | 31 | 31 | 1.00× |
| `chat messages offset` | 94 | 47 | 0.50× |
| `chat messages cursor` | 31 | 16 | 0.50× |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 210 | 206 | 0.98× |
| `export messages json with images` | 596 | 568 | 0.95× |
| `export messages json` | 235 | 214 | 0.91× |
| `export messages chatlab` | 237 | 212 | 0.90× ▼ |
| `export messages excel` | 258 | 230 | 0.89× ▼ |
| `export messages html` | 227 | 215 | 0.94× |
| `export messages txt` | 248 | 223 | 0.90× |
| `report annual` | 131 | 131 | 1.00× |
| `analytics overall` | 119 | 119 | 1.00× |
| `group members` | 208 | 206 | 0.99× |
| `group ranking` | 201 | 201 | 1.00× |
| `chat export-stats` | 201 | 201 | 1.00× |
| `chat date-counts` | 199 | 199 | 1.00× |
| `chat search` | 58.5 | 58.5 | 1.00× |
| `report dual` | 48.6 | 50.5 | 1.04× |
| `chat messages first page` | 34.5 | 34.4 | 1.00× |
| `chat messages offset` | 145 | 145 | 1.00× |
| `chat messages cursor` | 35.2 | 35.2 | 1.00× |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 406 | 406 | 1.00× |
| `export messages json with images` | 817 | 785 | 0.96× |
| `export messages json` | 428 | 395 | 0.92× |
| `export messages chatlab` | 431 | 400 | 0.93× |
| `export messages excel` | 434 | 390 | 0.90× ▼ |
| `export messages html` | 420 | 375 | 0.89× ▼ |
| `export messages txt` | 447 | 381 | 0.85× ▼ |
| `report annual` | 274 | 274 | 1.00× |
| `analytics overall` | 247 | 247 | 1.00× |
| `group members` | 339 | 337 | 0.99× |
| `group ranking` | 331 | 331 | 1.00× |
| `chat export-stats` | 330 | 330 | 1.00× |
| `chat date-counts` | 328 | 328 | 1.00× |
| `chat search` | 187 | 187 | 1.00× |
| `report dual` | 198 | 203 | 1.02× |
| `chat messages first page` | 159 | 159 | 1.00× |
| `chat messages offset` | 280 | 280 | 1.00× |
| `chat messages cursor` | 159 | 158 | 0.99× |

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
| `report annual` | 8 | 8 | 1.00× |
| `analytics overall` | 8 | 8 | 1.00× |
| `group members` | 8 | 8 | 1.00× |
| `group ranking` | 8 | 8 | 1.00× |
| `chat export-stats` | 10 | 8 | 0.80× ▼ |
| `chat date-counts` | 8 | 8 | 1.00× |
| `chat search` | 8 | 8 | 1.00× |
| `report dual` | 8 | 8 | 1.00× |
| `chat messages first page` | 8 | 8 | 1.00× |
| `chat messages offset` | 8 | 8 | 1.00× |
| `chat messages cursor` | 8 | 8 | 1.00× |

### 读取（MiB）

逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 847 | 847 | 1.00× |
| `export messages json with images` | 1,469 | 1,304 | 0.89× ▼ |
| `export messages json` | 812 | 654 | 0.80× ▼ |
| `export messages chatlab` | 653 | 489 | 0.75× ▼ |
| `export messages excel` | 687 | 522 | 0.76× ▼ |
| `export messages html` | 662 | 497 | 0.75× ▼ |
| `export messages txt` | 605 | 441 | 0.73× ▼ |
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
| `export media images` | 111,744 | 117,319 | 1.05× |
| `export messages json with images` | 229,395 | 235,393 | 1.03× |
| `export messages json` | 86,484 | 104,279 | 1.21× ▲ |
| `export messages chatlab` | 85,353 | 112,963 | 1.32× ▲ |
| `export messages excel` | 90,462 | 106,069 | 1.17× ▲ |
| `export messages html` | 81,528 | 103,771 | 1.27× ▲ |
| `export messages txt` | 83,436 | 109,305 | 1.31× ▲ |
| `report annual` | 35,196 | 35,240 | 1.00× |
| `analytics overall` | 30,601 | 30,644 | 1.00× |
| `group members` | 59,062 | 58,628 | 0.99× |
| `group ranking` | 57,059 | 57,066 | 1.00× |
| `chat export-stats` | 54,478 | 54,485 | 1.00× |
| `chat date-counts` | 53,361 | 53,367 | 1.00× |
| `chat search` | 15,186 | 15,188 | 1.00× |
| `report dual` | 17,047 | 17,591 | 1.03× |
| `chat messages first page` | 9,085 | 9,102 | 1.00× |
| `chat messages offset` | 37,280 | 37,203 | 1.00× |
| `chat messages cursor` | 9,314 | 9,299 | 1.00× |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | 10,762 | 11,236 | 1.04× |
| `export messages json` | 69,035 | 80,812 | 1.17× ▲ |
| `export messages chatlab` | 85,370 | 95,706 | 1.12× ▲ |
| `export messages excel` | 61,037 | 68,320 | 1.12× ▲ |
| `export messages html` | 90,635 | 100,659 | 1.11× ▲ |
| `export messages txt` | 99,583 | 113,201 | 1.14× ▲ |
| `report annual` | 230,169 | 231,436 | 1.01× |
| `group members` | 303,490 | 306,903 | 1.01× |
| `group ranking` | 316,687 | 305,600 | 0.96× |
| `chat export-stats` | 359,616 | 357,777 | 0.99× |
| `chat date-counts` | 444,323 | 443,372 | 1.00× |
| `report dual` | 48,000 | 47,806 | 1.00× |
| `chat messages first page` | 462 | 452 | 0.98× |
| `chat messages offset` | 132 | 131 | 0.99× |
| `chat messages cursor` | 455 | 460 | 1.01× |

### 分配次数

分配和重新分配的次数。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 33,069,369 | 32,927,058 | 1.00× |
| `export messages json with images` | 45,650,363 | 43,710,572 | 0.96× |
| `export messages json` | 14,481,525 | 12,590,295 | 0.87× ▼ |
| `export messages chatlab` | 14,473,041 | 12,381,984 | 0.86× ▼ |
| `export messages excel` | 16,631,236 | 14,536,423 | 0.87× ▼ |
| `export messages html` | 14,765,834 | 13,454,969 | 0.91× |
| `export messages txt` | 12,882,635 | 11,370,383 | 0.88× ▼ |
| `report annual` | 10,061,523 | 10,061,522 | 1.00× |
| `analytics overall` | 2,425,660 | 2,425,660 | 1.00× |
| `group members` | 123,377 | 123,377 | 1.00× |
| `group ranking` | 23,621 | 23,621 | 1.00× |
| `chat export-stats` | 105,511 | 105,510 | 1.00× |
| `chat date-counts` | 15,943 | 15,943 | 1.00× |
| `chat search` | 44,956 | 44,957 | 1.00× |
| `report dual` | 282,892 | 282,895 | 1.00× |
| `chat messages first page` | 8,181 | 8,181 | 1.00× |
| `chat messages offset` | 14,691 | 14,692 | 1.00× |
| `chat messages cursor` | 8,035 | 8,035 | 1.00× |

### 累计分配（MiB）

每次分配的大小之和，重新分配只计增长的部分。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 8,268 | 8,263 | 1.00× |
| `export messages json with images` | 45,490 | 44,591 | 0.98× |
| `export messages json` | 40,898 | 40,010 | 0.98× |
| `export messages chatlab` | 39,936 | 38,701 | 0.97× |
| `export messages excel` | 39,974 | 38,735 | 0.97× |
| `export messages html` | 38,528 | 37,964 | 0.99× |
| `export messages txt` | 38,989 | 38,167 | 0.98× |
| `report annual` | 1,166 | 1,166 | 1.00× |
| `analytics overall` | 400 | 400 | 1.00× |
| `group members` | 384 | 384 | 1.00× |
| `group ranking` | 375 | 375 | 1.00× |
| `chat export-stats` | 384 | 384 | 1.00× |
| `chat date-counts` | 373 | 373 | 1.00× |
| `chat search` | 204 | 204 | 1.00× |
| `report dual` | 622 | 622 | 1.00× |
| `chat messages first page` | 59.1 | 59.1 | 1.00× |
| `chat messages offset` | 341 | 341 | 1.00× |
| `chat messages cursor` | 57.4 | 57.4 | 1.00× |

### 峰值存活堆（MiB）

同一时刻仍未释放的分配之和的最大值。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 94.7 | 94.6 | 1.00× |
| `export messages json with images` | 424 | 424 | 1.00× |
| `export messages json` | 63.3 | 52.4 | 0.83× ▼ |
| `export messages chatlab` | 62.1 | 51.0 | 0.82× ▼ |
| `export messages excel` | 81.4 | 77.1 | 0.95× |
| `export messages html` | 63.2 | 51.5 | 0.82× ▼ |
| `export messages txt` | 63.3 | 51.5 | 0.81× ▼ |
| `report annual` | 15.6 | 15.6 | 1.00× |
| `analytics overall` | 11.8 | 11.8 | 1.00× |
| `group members` | 10.0 | 11.8 | 1.18× |
| `group ranking` | 10.6 | 10.6 | 1.00× |
| `chat export-stats` | 14.6 | 14.6 | 1.00× |
| `chat date-counts` | 10.6 | 10.6 | 1.00× |
| `chat search` | 10.6 | 10.6 | 1.00× |
| `report dual` | 11.7 | 11.7 | 1.00× |
| `chat messages first page` | 10.6 | 10.6 | 1.00× |
| `chat messages offset` | 15.3 | 15.3 | 1.00× |
| `chat messages cursor` | 10.6 | 10.6 | 1.00× |

## `weflow serve` 持续负载

每个版本启动一个 `weflow serve`，用 4 个并发连续请求 90 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。

|  | base | head | head / base |
|---|---|---|---|
| 每秒请求数 | 32 | 32 | 1.00× |
| 错误数 | 0 | 0 | - |
| 延迟 p50 (ms) | 59 | 58 | 0.99× |
| 延迟 p95 (ms) | 588 | 600 | 1.02× |
| 延迟 p99 (ms) | 1,003 | 1,036 | 1.03× |
| 启动到开始监听 (ms) | 15 | 16 | 1.08× |
| 工作集，第 1 段中位数 (MiB) | 363 | 361 | 0.99× |
| 工作集，第 2 段中位数 (MiB) | 364 | 359 | 0.98× |
| 工作集，第 3 段中位数 (MiB) | 366 | 365 | 1.00× |
| 工作集，第 4 段中位数 (MiB) | 370 | 380 | 1.03× |
| 第 2 段到第 4 段的增长 (MiB) | 5.1 | 21 | - |
| 峰值工作集 (MiB) | 419 | 406 | 0.97× |
| 峰值线程数 | 13 | 13 | 1.00× |
| CPU 时间 (ms) | 134,812 | 135,016 | 1.00× |

各路由：请求次数，延迟 p50 / p95（ms）。

| 路由 | base | head | p50 head / base |
|---|---|---|---|
| `/api/chat/messages`，大群任意一页 | 1009 次，117 / 251 | 1007 次，110 / 244 | 0.94× |
| `/api/chat/messages`，其他会话第一页 | 602 次，12 / 104 | 602 次，13 / 113 | 1.11× |
| `/api/chat/sessions` | 403 次，38 / 240 | 403 次，37 / 205 | 0.98× |
| `/api/chat/search`，全部会话 | 269 次，595 / 1,154 | 271 次，616 / 1,201 | 1.04× |
| `/api/chat/date-counts` | 265 次，12 / 106 | 266 次，8.1 / 124 | 0.67× |
| `/api/chat/contacts` | 299 次，17 / 37 | 298 次，18 / 37 | 1.07× |

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 0，head 0。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。

| 路由 | base | head | head / base |
|---|---|---|---|
| `16 pages at once (all of them) [api]` | 25 | 25 | 1.00× |
| `16 pages at once (all of them) [sub]` | 49 | 48 | 1.00× |
| `20 images at once (all of them) [api]` | 9.4 | 8.4 | 0.89× |
| `20 images at once (all of them) [sub]` | 35 | 33 | 0.94× |
| `analytics overall (cached) [api]` | 2.4 | 2.0 | 0.81× |
| `analytics overall (cached) [sub]` | 21 | 19 | 0.90× |
| `analytics overall --force [api]` | 314 | 295 | 0.94× |
| `analytics overall --force [sub]` | 483 | 483 | 1.00× |
| `analytics rankings (cached) [api]` | 5.4 | 5.5 | 1.02× |
| `analytics rankings (cached) [sub]` | 24 | 24 | 1.00× |
| `analytics time (cached) [api]` | 2.1 | 2.1 | 1.00× |
| `analytics time (cached) [sub]` | 20 | 19 | 0.98× |
| `biz pay-records [api]` | 0.8 | 0.8 | 0.98× |
| `biz pay-records [sub]` | 26 | 26 | 0.97× |
| `cache list [api]` | 0.5 | 0.5 | 1.02× |
| `cache list [sub]` | 16 | 15 | 0.90× |
| `chat date-counts [api]` | 63 | 63 | 0.99× |
| `chat date-counts [sub]` | 386 | 368 | 0.95× |
| `chat dates [api]` | 64 | 62 | 0.96× |
| `chat dates [sub]` | 375 | 369 | 0.98× |
| `chat detail --fast [api]` | 1.0 | 0.9 | 0.88× |
| `chat detail --fast [sub]` | 49 | 50 | 1.03× |
| `chat resources, all conversations [api]` | 56 | 48 | 0.86× ▼ |
| `chat resources, all conversations [sub]` | 139 | 125 | 0.90× ▼ |
| `chat resources, all conversations, far page [api]` | 90 | 78 | 0.87× ▼ |
| `chat resources, all conversations, far page [sub]` | 187 | 168 | 0.90× ▼ |
| `chat resources, all conversations, file [api]` | 70 | 64 | 0.91× |
| `chat resources, all conversations, file [sub]` | 153 | 146 | 0.95× |
| `chat resources, all conversations, image [api]` | 37 | 19 | 0.52× ▼ |
| `chat resources, all conversations, image [sub]` | 83 | 62 | 0.75× ▼ |
| `chat resources, all conversations, video [api]` | 38 | 30 | 0.80× ▼ |
| `chat resources, all conversations, video [sub]` | 90 | 85 | 0.95× |
| `chat resources, all conversations, voice [api]` | 19 | 19 | 1.02× |
| `chat resources, all conversations, voice [sub]` | 64 | 65 | 1.03× |
| `chat resources, one conversation [api]` | 33 | 25 | 0.74× ▼ |
| `chat resources, one conversation [sub]` | 113 | 106 | 0.94× |
| `chat statuses of 60 conversations [api]` | 1.4 | 1.2 | 0.85× |
| `chat statuses of 60 conversations [sub]` | 19 | 19 | 1.01× |
| `chat tab-counts [api]` | 1.6 | 1.7 | 1.04× |
| `chat tab-counts [sub]` | 19 | 19 | 1.01× |
| `config profiles [api]` | 0.4 | 0.5 | 1.24× |
| `config profiles [sub]` | 17 | 16 | 0.93× |
| `contacts, all [api]` | 11 | 10 | 0.92× |
| `contacts, all [sub]` | 53 | 51 | 0.97× |
| `contacts, all, br [api]` | 16 | 16 | 1.04× |
| `contacts, all, gzip [api]` | 13 | 12 | 0.96× |
| `contacts, all, the fields of a list [api]` | 10.0 | 8.7 | 0.87× |
| `contacts, all, the fields of a list, br [api]` | 12 | 11 | 0.87× |
| `counts of 60 conversations [api]` | 8.4 | 8.6 | 1.01× |
| `counts of 60 conversations [sub]` | 64 | 61 | 0.95× |
| `db test [api]` | 0.4 | 0.5 | 1.34× |
| `db test [sub]` | 16 | 15 | 0.97× |
| `first conversation opened after start (first page) [api]` | 7.3 | 7.3 | 1.00× |
| `first page, 1 at once (all of them) [api]` | 2.7 | 2.4 | 0.89× |
| `first page, 1 at once (all of them) [sub]` | 33 | 32 | 0.96× |
| `first page, 16 at once (all of them) [api]` | 20 | 18 | 0.93× |
| `first page, 16 at once (all of them) [sub]` | 49 | 46 | 0.93× |
| `first page, 32 at once (all of them) [api]` | 36 | 34 | 0.95× |
| `first page, 32 at once (all of them) [sub]` | 56 | 57 | 1.03× |
| `first page, 6 at once (all of them) [api]` | 8.1 | 7.5 | 0.92× |
| `first page, 6 at once (all of them) [sub]` | 49 | 47 | 0.95× |
| `first page, 64 at once (all of them) [api]` | 71 | 67 | 0.95× |
| `first read after start (sessions, 50) [api]` | 103 | 98 | 0.95× |
| `group list [api]` | 3.4 | 3.2 | 0.92× |
| `group list [sub]` | 21 | 21 | 1.04× |
| `group members [api]` | 1.4 | 1.2 | 0.80× |
| `group members [sub]` | 31 | 28 | 0.91× |
| `image, again (each) [api]` | 1.1 | 0.9 | 0.87× |
| `image, again (each) [sub]` | 32 | 31 | 0.95× |
| `image, first time (each) [api]` | 2.2 | 2.2 | 0.97× |
| `image, first time (each) [sub]` | 33 | 31 | 0.95× |
| `latest 20 (polling) [api]` | 1.8 | 1.7 | 0.97× |
| `latest 20 (polling) [sub]` | 31 | 30 | 0.97× |
| `messages, first page [api]` | 2.5 | 2.3 | 0.92× |
| `messages, first page [sub]` | 34 | 31 | 0.92× |
| `messages, first page, br [api]` | 4.3 | 4.3 | 1.00× |
| `messages, first page, gzip [api]` | 2.9 | 2.8 | 0.95× |
| `messages, first page, the fields shown [api]` | 2.9 | 2.6 | 0.91× |
| `messages, first page, the fields shown, br [api]` | 3.6 | 3.2 | 0.89× |
| `messages, page in the middle [api]` | 72 | 70 | 0.98× |
| `messages, page in the middle [sub]` | 315 | 297 | 0.94× |
| `messages, page in the middle, from a cursor [api]` | 2.4 | 2.0 | 0.84× |
| `messages, page in the middle, from a cursor [sub]` | 32 | 29 | 0.93× |
| `one contact [api]` | 0.7 | 0.6 | 0.87× |
| `one contact [sub]` | 19 | 17 | 0.91× |
| `one message [api]` | 2.1 | 1.2 | 0.56× |
| `one message [sub]` | 29 | 26 | 0.90× |
| `push engine, CPU ms per second while listened to [api]` | 0.0 | 0.0 | - |
| `report annual years [api]` | 83 | 80 | 0.96× |
| `report annual years [sub]` | 274 | 273 | 1.00× |
| `search in the conversation [api]` | 2.4 | 2.2 | 0.90× |
| `search in the conversation [sub]` | 33 | 31 | 0.94× |
| `search in the conversation, a word found nowhere [api]` | 48 | 46 | 0.96× |
| `search in the conversation, a word found nowhere [sub]` | 129 | 122 | 0.94× |
| `server start (until it listens) [-]` | 18 | 16 | 0.88× |
| `sessions, 50 [api]` | 12 | 11 | 0.98× |
| `sessions, 50 [sub]` | 41 | 39 | 0.96× |
| `sessions, all [api]` | 12 | 11 | 0.93× |
| `sessions, all [sub]` | 43 | 41 | 0.97× |
| `sns post-counts --prefer-cache [api]` | 0.6 | 0.5 | 0.75× |
| `sns post-counts --prefer-cache [sub]` | 30 | 27 | 0.90× |
| `sns stats --fast [api]` | 0.3 | 0.3 | 0.92× |
| `sns stats --fast [sub]` | 28 | 26 | 0.95× |
| `sns timeline, 20 [api]` | 37 | 37 | 0.99× |
| `sns timeline, 20 [sub]` | 63 | 62 | 0.98× |
| `sns timeline, 20, far page [api]` | 6.0 | 4.7 | 0.79× |
| `sns timeline, 20, far page [sub]` | 66 | 61 | 0.93× |
| `sns users [api]` | 3.7 | 3.2 | 0.87× |
| `sns users [sub]` | 27 | 27 | 0.97× |
| `thumbnail of the image (each) [api]` | 0.9 | 0.8 | 0.87× |
| `thumbnail of the image (each) [sub]` | 32 | 30 | 0.96× |
