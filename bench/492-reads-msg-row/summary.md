# 基准测试：base 与 head

- base：`0b0b598503163b732bb2cc48a23bf7d94591293d`（`0b0b598503`），weflow 5.0.1-188-g0b0b5985
- head：`8bccd3394ccc1b9940e87b53536977783422b382`（`8bccd3394c`），weflow 5.0.1-196-g8bccd339
- 基准账号：种子 1，325,798 条消息，316 个会话
- runner：4 个逻辑 CPU，16.0 GiB 内存，win32 win25-vs2026
- 每条命令先各运行一次不计时，再由 base 和 head 交替各运行 10 次，取成功运行的中位数
- 比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好

**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。

## 二进制体积

|  | base | head | head / base |
|---|---|---|---|
| `weflow.exe` | 30.8 MiB | 30.7 MiB | 1.00× |

## 命令

### 耗时（ms）

从启动到退出。括号里是最快和最慢的一次。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 20,367（19,541–22,191） | 20,113（19,635–21,795） | 0.99× |
| `export messages json with images` | 22,639（22,356–23,169） | 23,026（22,698–24,365） | 1.02× |
| `export messages json` | 3,855（3,813–4,031） | 3,834（3,778–4,018） | 0.99× |
| `export messages chatlab` | 3,161（3,107–3,253） | 3,133（3,086–3,183） | 0.99× |
| `export messages excel` | 4,096（4,056–4,171） | 4,075（4,016–4,124） | 0.99× |
| `export messages html` | 2,844（2,820–2,887） | 2,825（2,782–2,889） | 0.99× |
| `export messages txt` | 2,581（2,552–2,620） | 2,569（2,520–2,651） | 1.00× |
| `report annual` | 1,920（1,896–1,953） | 1,915（1,873–1,960） | 1.00× |
| `analytics overall` | 808（800–817） | 814（800–840） | 1.01× |
| `group members` | 882（869–908） | 870（866–890） | 0.99× |
| `group ranking` | 859（850–872） | 859（848–901） | 1.00× |
| `chat export-stats` | 795（787–824） | 787（774–801） | 0.99× |
| `chat date-counts` | 652（641–661） | 651（641–657） | 1.00× |
| `chat search` | 404（393–407） | 400（395–407） | 0.99× |
| `report dual` | 352（345–361） | 286（281–295） | 0.81× ▼ |
| `chat messages first page` | 160（159–162） | 160（158–167） | 1.00× |
| `chat messages offset` | 546（540–556） | 541（535–549） | 0.99× |
| `chat messages cursor` | 160（160–169） | 160（158–166） | 1.00× |

### 第一批结果（ms）

从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 1,813 | 1,669 | 0.92× |
| `export messages json with images` | 381 | 379 | 0.99× |
| `export messages json` | 370 | 371 | 1.00× |
| `export messages chatlab` | 409 | 408 | 1.00× |
| `export messages excel` | 371 | 373 | 1.01× |
| `export messages html` | 371 | 372 | 1.00× |
| `export messages txt` | 375 | 372 | 0.99× |
| `report annual` | 15 | 15 | 0.99× |
| `analytics overall` | 142 | 142 | 1.00× |
| `report dual` | 15 | 15 | 1.00× |

### 用户态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 12,945 | 12,312 | 0.95× |
| `export messages json with images` | 18,625 | 18,242 | 0.98× |
| `export messages json` | 7,930 | 7,867 | 0.99× |
| `export messages chatlab` | 7,297 | 7,234 | 0.99× |
| `export messages excel` | 7,891 | 7,883 | 1.00× |
| `export messages html` | 7,188 | 7,133 | 0.99× |
| `export messages txt` | 7,219 | 7,141 | 0.99× |
| `report annual` | 1,633 | 1,656 | 1.01× |
| `analytics overall` | 688 | 688 | 1.00× |
| `group members` | 750 | 703 | 0.94× |
| `group ranking` | 734 | 734 | 1.00× |
| `chat export-stats` | 633 | 656 | 1.04× |
| `chat date-counts` | 516 | 516 | 1.00× |
| `chat search` | 328 | 297 | 0.90× |
| `report dual` | 289 | 234 | 0.81× ▼ |
| `chat messages first page` | 133 | 133 | 1.00× |
| `chat messages offset` | 430 | 422 | 0.98× |
| `chat messages cursor` | 148 | 133 | 0.89× |

### 内核态 CPU（ms）

进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 24,109 | 24,188 | 1.00× |
| `export messages json with images` | 23,781 | 23,344 | 0.98× |
| `export messages json` | 797 | 672 | 0.84× ▼ |
| `export messages chatlab` | 586 | 602 | 1.03× |
| `export messages excel` | 766 | 750 | 0.98× |
| `export messages html` | 594 | 609 | 1.03× |
| `export messages txt` | 578 | 555 | 0.96× |
| `report annual` | 281 | 273 | 0.97× |
| `analytics overall` | 125 | 125 | 1.00× |
| `group members` | 148 | 164 | 1.11× |
| `group ranking` | 133 | 133 | 1.00× |
| `chat export-stats` | 148 | 141 | 0.95× |
| `chat date-counts` | 125 | 141 | 1.12× |
| `chat search` | 78 | 94 | 1.20× |
| `report dual` | 62 | 47 | 0.75× |
| `chat messages first page` | 16 | 23 | 1.50× |
| `chat messages offset` | 117 | 109 | 0.93× |
| `chat messages cursor` | 16 | 31 | 2.00× |

### 峰值工作集（MiB）

主进程占用物理内存的峰值。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 282 | 206 | 0.73× ▼ |
| `export messages json with images` | 590 | 592 | 1.00× |
| `export messages json` | 224 | 226 | 1.01× |
| `export messages chatlab` | 228 | 227 | 0.99× |
| `export messages excel` | 248 | 251 | 1.01× |
| `export messages html` | 225 | 224 | 0.99× |
| `export messages txt` | 236 | 240 | 1.02× |
| `report annual` | 134 | 135 | 1.00× |
| `analytics overall` | 120 | 120 | 1.00× |
| `group members` | 206 | 206 | 1.00× |
| `group ranking` | 198 | 198 | 1.00× |
| `chat export-stats` | 198 | 198 | 1.00× |
| `chat date-counts` | 195 | 195 | 1.00× |
| `chat search` | 58.5 | 58.5 | 1.00× |
| `report dual` | 49.2 | 49.7 | 1.01× |
| `chat messages first page` | 31.1 | 31.4 | 1.01× |
| `chat messages offset` | 145 | 145 | 1.00× |
| `chat messages cursor` | 31.0 | 31.6 | 1.02× |

### 峰值提交内存（MiB）

进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 426 | 402 | 0.94× |
| `export messages json with images` | 823 | 822 | 1.00× |
| `export messages json` | 417 | 429 | 1.03× |
| `export messages chatlab` | 423 | 432 | 1.02× |
| `export messages excel` | 430 | 430 | 1.00× |
| `export messages html` | 414 | 422 | 1.02× |
| `export messages txt` | 442 | 447 | 1.01× |
| `report annual` | 276 | 276 | 1.00× |
| `analytics overall` | 247 | 247 | 1.00× |
| `group members` | 339 | 339 | 1.00× |
| `group ranking` | 328 | 328 | 1.00× |
| `chat export-stats` | 328 | 328 | 1.00× |
| `chat date-counts` | 324 | 324 | 1.00× |
| `chat search` | 186 | 186 | 1.00× |
| `report dual` | 189 | 197 | 1.04× |
| `chat messages first page` | 157 | 157 | 1.00× |
| `chat messages offset` | 292 | 292 | 1.00× |
| `chat messages cursor` | 158 | 159 | 1.00× |

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
| `export messages json with images` | 1,468 | 1,468 | 1.00× |
| `export messages json` | 809 | 810 | 1.00× |
| `export messages chatlab` | 652 | 652 | 1.00× |
| `export messages excel` | 686 | 687 | 1.00× |
| `export messages html` | 661 | 661 | 1.00× |
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
| `chat search` | 0.1 | 0.1 | 0.92× |
| `report dual` | 0.0 | 0.0 | 1.00× |
| `chat messages first page` | 0.1 | 0.1 | 0.82× |
| `chat messages offset` | 0.1 | 0.1 | 0.86× |
| `chat messages cursor` | 0.1 | 0.1 | 0.86× |

### 页错误

进程及其子进程，包括软页错误。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 133,828 | 118,578 | 0.89× ▼ |
| `export messages json with images` | 239,654 | 235,660 | 0.98× |
| `export messages json` | 86,860 | 85,798 | 0.99× |
| `export messages chatlab` | 85,456 | 86,332 | 1.01× |
| `export messages excel` | 91,162 | 91,434 | 1.00× |
| `export messages html` | 82,528 | 84,088 | 1.02× |
| `export messages txt` | 87,402 | 88,305 | 1.01× |
| `report annual` | 35,920 | 36,347 | 1.01× |
| `analytics overall` | 30,845 | 30,810 | 1.00× |
| `group members` | 58,615 | 58,598 | 1.00× |
| `group ranking` | 56,502 | 56,488 | 1.00× |
| `chat export-stats` | 54,560 | 54,494 | 1.00× |
| `chat date-counts` | 52,406 | 52,466 | 1.00× |
| `chat search` | 15,203 | 15,199 | 1.00× |
| `report dual` | 21,294 | 17,019 | 0.80× ▼ |
| `chat messages first page` | 8,232 | 8,286 | 1.01× |
| `chat messages offset` | 37,326 | 37,276 | 1.00× |
| `chat messages cursor` | 8,254 | 8,389 | 1.02× |

### 吞吐量（消息/秒）

命令处理的消息数除以耗时。这是唯一一个越高越好的指标。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export messages json with images` | 8,834 | 8,686 | 0.98× |
| `export messages json` | 51,875 | 52,171 | 1.01× |
| `export messages chatlab` | 63,279 | 63,843 | 1.01× |
| `export messages excel` | 48,826 | 49,080 | 1.01× |
| `export messages html` | 70,315 | 70,785 | 1.01× |
| `export messages txt` | 77,482 | 77,850 | 1.00× |
| `report annual` | 169,707 | 170,125 | 1.00× |
| `group members` | 226,807 | 229,824 | 1.01× |
| `group ranking` | 232,951 | 232,803 | 1.00× |
| `chat export-stats` | 251,452 | 254,118 | 1.01× |
| `chat date-counts` | 306,646 | 307,160 | 1.00× |
| `report dual` | 28,422 | 34,913 | 1.23× ▲ |
| `chat messages first page` | 313 | 313 | 1.00× |
| `chat messages offset` | 92 | 92 | 1.01× |
| `chat messages cursor` | 313 | 313 | 1.00× |

### 分配次数

分配和重新分配的次数。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 33,881,866 | 33,065,698 | 0.98× |
| `export messages json with images` | 53,806,122 | 53,809,635 | 1.00× |
| `export messages json` | 22,637,753 | 22,637,515 | 1.00× |
| `export messages chatlab` | 18,664,120 | 18,664,408 | 1.00× |
| `export messages excel` | 16,827,205 | 16,827,374 | 1.00× |
| `export messages html` | 18,610,991 | 18,610,935 | 1.00× |
| `export messages txt` | 13,072,337 | 13,072,288 | 1.00× |
| `report annual` | 10,160,502 | 10,160,501 | 1.00× |
| `analytics overall` | 2,425,673 | 2,425,673 | 1.00× |
| `group members` | 123,390 | 123,389 | 1.00× |
| `group ranking` | 23,633 | 23,633 | 1.00× |
| `chat export-stats` | 105,523 | 105,523 | 1.00× |
| `chat date-counts` | 15,956 | 15,955 | 1.00× |
| `chat search` | 45,990 | 44,968 | 0.98× |
| `report dual` | 616,331 | 282,904 | 0.46× ▼ |
| `chat messages first page` | 11,496 | 8,193 | 0.71× ▼ |
| `chat messages offset` | 17,715 | 14,703 | 0.83× ▼ |
| `chat messages cursor` | 11,059 | 8,047 | 0.73× ▼ |

### 累计分配（MiB）

每次分配的大小之和，重新分配只计增长的部分。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 8,402 | 8,269 | 0.98× |
| `export messages json with images` | 46,681 | 46,684 | 1.00× |
| `export messages json` | 42,093 | 42,085 | 1.00× |
| `export messages chatlab` | 40,183 | 40,183 | 1.00× |
| `export messages excel` | 39,961 | 39,968 | 1.00× |
| `export messages html` | 38,949 | 38,947 | 1.00× |
| `export messages txt` | 38,987 | 38,985 | 1.00× |
| `report annual` | 1,171 | 1,171 | 1.00× |
| `analytics overall` | 400 | 400 | 1.00× |
| `group members` | 384 | 384 | 1.00× |
| `group ranking` | 375 | 375 | 1.00× |
| `chat export-stats` | 384 | 384 | 1.00× |
| `chat date-counts` | 373 | 373 | 1.00× |
| `chat search` | 204 | 204 | 1.00× |
| `report dual` | 1,871 | 622 | 0.33× ▼ |
| `chat messages first page` | 59.3 | 59.1 | 1.00× |
| `chat messages offset` | 341 | 341 | 1.00× |
| `chat messages cursor` | 57.6 | 57.5 | 1.00× |

### 峰值存活堆（MiB）

同一时刻仍未释放的分配之和的最大值。用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本（#513 之前）这一项为空。

| 命令 | base | head | head / base |
|---|---|---|---|
| `export media images` | 176 | 95.9 | 0.54× ▼ |
| `export messages json with images` | 424 | 424 | 1.00× |
| `export messages json` | 63.3 | 63.4 | 1.00× |
| `export messages chatlab` | 62.0 | 59.1 | 0.95× |
| `export messages excel` | 81.5 | 81.3 | 1.00× |
| `export messages html` | 63.2 | 63.2 | 1.00× |
| `export messages txt` | 63.3 | 63.3 | 1.00× |
| `report annual` | 15.8 | 15.8 | 1.00× |
| `analytics overall` | 11.8 | 11.8 | 1.00× |
| `group members` | 11.8 | 11.8 | 1.00× |
| `group ranking` | 8.8 | 10.6 | 1.20× |
| `chat export-stats` | 14.6 | 14.6 | 1.00× |
| `chat date-counts` | 10.6 | 8.5 | 0.81× |
| `chat search` | 10.1 | 10.6 | 1.05× |
| `report dual` | 12.2 | 11.7 | 0.96× |
| `chat messages first page` | 8.3 | 8.3 | 1.01× |
| `chat messages offset` | 15.3 | 15.3 | 1.00× |
| `chat messages cursor` | 8.4 | 10.6 | 1.26× |

## `weflow serve` 持续负载

每个版本启动一个 `weflow serve`，用 4 个并发连续请求 90 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。

|  | base | head | head / base |
|---|---|---|---|
| 每秒请求数 | 24 | 25 | 1.01× |
| 错误数 | 0 | 0 | - |
| 延迟 p50 (ms) | 60 | 59 | 0.99× |
| 延迟 p95 (ms) | 891 | 848 | 0.95× |
| 延迟 p99 (ms) | 1,512 | 1,470 | 0.97× |
| 启动到开始监听 (ms) | 17 | 17 | 0.98× |
| 工作集，第 1 段中位数 (MiB) | 355 | 362 | 1.02× |
| 工作集，第 2 段中位数 (MiB) | 362 | 368 | 1.02× |
| 工作集，第 3 段中位数 (MiB) | 361 | 377 | 1.05× |
| 工作集，第 4 段中位数 (MiB) | 360 | 381 | 1.06× |
| 第 2 段到第 4 段的增长 (MiB) | -2.3 | 13 | - |
| 峰值工作集 (MiB) | 398 | 425 | 1.07× |
| 峰值线程数 | 13 | 13 | 1.00× |
| CPU 时间 (ms) | 136,047 | 137,141 | 1.01× |

各路由：请求次数，延迟 p50 / p95（ms）。

| 路由 | base | head | p50 head / base |
|---|---|---|---|
| `/api/chat/messages`，大群任意一页 | 774 次，129 / 291 | 787 次，130 / 282 | 1.01× |
| `/api/chat/messages`，其他会话第一页 | 477 次，4.2 / 123 | 481 次，3.4 / 132 | 0.82× |
| `/api/chat/sessions` | 295 次，38 / 212 | 299 次，39 / 251 | 1.02× |
| `/api/chat/search`，全部会话 | 210 次，908 / 1,676 | 212 次，882 / 1,640 | 0.97× |
| `/api/chat/date-counts` | 206 次，3.1 / 112 | 207 次，3.3 / 139 | 1.07× |
| `/api/chat/contacts` | 230 次，19 / 36 | 232 次，19 / 38 | 0.99× |

## serve_bench

各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。退出码：base 0，head 0。某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），格子里注明返回的状态，不计算比值。

| 路由 | base | head | head / base |
|---|---|---|---|
| `16 pages at once (all of them) [api]` | 41 | 32 | 0.80× ▼ |
| `16 pages at once (all of them) [sub]` | 63 | 58 | 0.92× |
| `20 images at once (all of them) [api]` | 12 | 12 | 0.99× |
| `20 images at once (all of them) [sub]` | 38 | 37 | 0.98× |
| `analytics overall (cached) [api]` | 2.6 | 2.0 | 0.76× |
| `analytics overall (cached) [sub]` | 21 | 20 | 0.98× |
| `analytics overall --force [api]` | 412 | 415 | 1.01× |
| `analytics overall --force [sub]` | 707 | 697 | 0.99× |
| `analytics rankings (cached) [api]` | 7.8 | 7.3 | 0.94× |
| `analytics rankings (cached) [sub]` | 27 | 26 | 0.96× |
| `analytics time (cached) [api]` | 2.6 | 2.2 | 0.84× |
| `analytics time (cached) [sub]` | 21 | 20 | 0.95× |
| `biz pay-records [api]` | 0.7 | 0.7 | 0.96× |
| `biz pay-records [sub]` | 29 | 28 | 0.97× |
| `cache list [api]` | 0.6 | 0.6 | 1.00× |
| `cache list [sub]` | 17 | 17 | 1.01× |
| `chat date-counts [api]` | 69 | 68 | 0.99× |
| `chat date-counts [sub]` | 532 | 524 | 0.98× |
| `chat dates [api]` | 69 | 68 | 0.99× |
| `chat dates [sub]` | 529 | 523 | 0.99× |
| `chat detail --fast [api]` | 1.1 | 1.1 | 1.03× |
| `chat detail --fast [sub]` | 58 | 56 | 0.97× |
| `chat resources, all conversations [api]` | 80 | 66 | 0.82× ▼ |
| `chat resources, all conversations [sub]` | 196 | 175 | 0.89× ▼ |
| `chat resources, all conversations, far page [api]` | 142 | 117 | 0.83× ▼ |
| `chat resources, all conversations, far page [sub]` | 264 | 234 | 0.89× ▼ |
| `chat resources, all conversations, file [api]` | 99 | 87 | 0.87× ▼ |
| `chat resources, all conversations, file [sub]` | 189 | 178 | 0.94× |
| `chat resources, all conversations, image [api]` | 51 | 45 | 0.88× ▼ |
| `chat resources, all conversations, image [sub]` | 108 | 98 | 0.91× |
| `chat resources, all conversations, video [api]` | 58 | 49 | 0.84× ▼ |
| `chat resources, all conversations, video [sub]` | 129 | 114 | 0.88× ▼ |
| `chat resources, all conversations, voice [api]` | 31 | 25 | 0.81× ▼ |
| `chat resources, all conversations, voice [sub]` | 91 | 82 | 0.90× |
| `chat resources, one conversation [api]` | 49 | 39 | 0.81× ▼ |
| `chat resources, one conversation [sub]` | 160 | 140 | 0.87× ▼ |
| `chat statuses of 60 conversations [api]` | 1.4 | 1.3 | 0.97× |
| `chat statuses of 60 conversations [sub]` | 20 | 19 | 0.98× |
| `chat tab-counts [api]` | 1.7 | 1.7 | 1.01× |
| `chat tab-counts [sub]` | 20 | 20 | 0.98× |
| `config profiles [api]` | 0.5 | 0.5 | 1.00× |
| `config profiles [sub]` | 17 | 17 | 1.03× |
| `contacts, all [api]` | 13 | 14 | 1.03× |
| `contacts, all [sub]` | 47 | 46 | 0.97× |
| `contacts, all, br [api]` | 22 | 20 | 0.93× |
| `contacts, all, gzip [api]` | 16 | 16 | 1.02× |
| `contacts, all, the fields of a list [api]` | 13 | 13 | 1.00× |
| `contacts, all, the fields of a list, br [api]` | 15 | 14 | 0.95× |
| `counts of 60 conversations [api]` | 11 | 11 | 1.04× |
| `counts of 60 conversations [sub]` | 72 | 73 | 1.01× |
| `db test [api]` | 0.5 | 0.5 | 0.98× |
| `db test [sub]` | 18 | 18 | 1.00× |
| `first conversation opened after start (first page) [api]` | 7.9 | 8.8 | 1.12× |
| `first page, 1 at once (all of them) [api]` | 4.1 | 3.1 | 0.76× |
| `first page, 1 at once (all of them) [sub]` | 36 | 34 | 0.96× |
| `first page, 16 at once (all of them) [api]` | 31 | 23 | 0.76× ▼ |
| `first page, 16 at once (all of them) [sub]` | 59 | 57 | 0.97× |
| `first page, 32 at once (all of them) [api]` | 61 | 45 | 0.74× ▼ |
| `first page, 32 at once (all of them) [sub]` | 62 | 57 | 0.93× |
| `first page, 6 at once (all of them) [api]` | 13 | 9.6 | 0.72× |
| `first page, 6 at once (all of them) [sub]` | 58 | 55 | 0.95× |
| `first page, 64 at once (all of them) [api]` | 116 | 88 | 0.76× ▼ |
| `first read after start (sessions, 50) [api]` | 148 | 154 | 1.04× |
| `group list [api]` | 4.1 | 4.2 | 1.03× |
| `group list [sub]` | 23 | 23 | 1.04× |
| `group members [api]` | 1.2 | 1.3 | 1.07× |
| `group members [sub]` | 32 | 32 | 1.00× |
| `image, again (each) [api]` | 1.3 | 1.2 | 0.97× |
| `image, again (each) [sub]` | 36 | 34 | 0.96× |
| `image, first time (each) [api]` | 2.4 | 3.0 | 1.25× |
| `image, first time (each) [sub]` | 38 | 36 | 0.94× |
| `latest 20 (polling) [api]` | 2.2 | 1.8 | 0.85× |
| `latest 20 (polling) [sub]` | 32 | 32 | 0.99× |
| `messages, first page [api]` | 3.1 | 2.5 | 0.82× |
| `messages, first page [sub]` | 34 | 34 | 0.98× |
| `messages, first page, br [api]` | 4.9 | 3.8 | 0.79× |
| `messages, first page, gzip [api]` | 3.7 | 3.2 | 0.85× |
| `messages, first page, the fields shown [api]` | 3.3 | 2.6 | 0.79× |
| `messages, first page, the fields shown, br [api]` | 5.6 | 4.0 | 0.70× |
| `messages, page in the middle [api]` | 90 | 90 | 1.00× |
| `messages, page in the middle [sub]` | 419 | 412 | 0.98× |
| `messages, page in the middle, from a cursor [api]` | 3.2 | 2.5 | 0.80× |
| `messages, page in the middle, from a cursor [sub]` | 35 | 34 | 0.97× |
| `one contact [api]` | 0.7 | 0.7 | 1.03× |
| `one contact [sub]` | 19 | 19 | 1.00× |
| `one message [api]` | 0.9 | 0.8 | 0.95× |
| `one message [sub]` | 31 | 30 | 0.98× |
| `push engine, CPU ms per second while listened to [api]` | 0.0 | 0.0 | - |
| `report annual years [api]` | 106 | 103 | 0.97× |
| `report annual years [sub]` | 385 | 382 | 0.99× |
| `search in the conversation [api]` | 2.6 | 2.4 | 0.94× |
| `search in the conversation [sub]` | 35 | 35 | 1.00× |
| `search in the conversation, a word found nowhere [api]` | 57 | 57 | 0.99× |
| `search in the conversation, a word found nowhere [sub]` | 156 | 155 | 0.99× |
| `server start (until it listens) [-]` | 17 | 17 | 1.00× |
| `sessions, 50 [api]` | 14 | 13 | 0.99× |
| `sessions, 50 [sub]` | 45 | 45 | 1.00× |
| `sessions, all [api]` | 14 | 15 | 1.02× |
| `sessions, all [sub]` | 47 | 47 | 1.00× |
| `sns post-counts --prefer-cache [api]` | 0.5 | 0.5 | 0.99× |
| `sns post-counts --prefer-cache [sub]` | 32 | 32 | 1.00× |
| `sns stats --fast [api]` | 0.4 | 0.4 | 1.00× |
| `sns stats --fast [sub]` | 33 | 31 | 0.94× |
| `sns timeline, 20 [api]` | 7.5 | 7.2 | 0.96× |
| `sns timeline, 20 [sub]` | 74 | 75 | 1.01× |
| `sns timeline, 20, far page [api]` | 7.5 | 6.5 | 0.87× |
| `sns timeline, 20, far page [sub]` | 74 | 73 | 0.99× |
| `sns users [api]` | 4.9 | 3.9 | 0.79× |
| `sns users [sub]` | 33 | 32 | 0.98× |
| `thumbnail of the image (each) [api]` | 1.1 | 1.1 | 0.99× |
| `thumbnail of the image (each) [sub]` | 35 | 34 | 0.98× |
