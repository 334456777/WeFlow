"""Benchmark two weflow binaries on the same Windows machine (#259), on the benchmark account of #488.

usage: python bench.py --base EXE --head EXE --account DIR --out DIR [--runs N] [--load-seconds S]
                       [--serve-bench EXE] [--only NAME[,NAME...]]

`--account` is the work folder of `bench_account` (its `home` and `manifest.json`). Each binary gets a home of its
own, a copy of that configuration; its caches are removed before every run, so each run starts from the same state
(the files of the account stay in the system's file cache: a first, untimed run of every command fills it).

Measured, for the heaviest commands, each `--runs` times, base and head taking turns:
  - wall time, the time to the first progress event and to the first batch of work (`--progress`)
  - user and kernel CPU time, logical bytes read and written, page faults: the process and its children, from a job
    object; disk I/O apart from the system's file cache is not counted per process by Windows
  - peak working set of the process and peak committed memory of the job; a memory and thread-count curve sampled
    every 20 ms, and the peak thread count
  - the size of what the command wrote and of the caches it left
  - with `--base-alloc` / `--head-alloc` (builds with the `alloc-stats` feature, #513), one more untimed run counting
    the allocations, the bytes allocated and the peak of live heap
Then, for each binary: a sustained load on `weflow serve` (`--load-seconds`, default 90) with the latency, the
throughput and the memory curve of the server, whose median by quarter of the time says whether it stays stable; and
`serve_bench` (when given), the latency of each route. Last, the size of each binary.

Writes `results.json` (everything, every run and sample), `summary.md` (the tables and the memory curves as Mermaid
charts, since the job summary drops SVG; also appended to `$GITHUB_STEP_SUMMARY`), `report.html` (the tables and
the memory curves), both in Chinese for the one who reads them, and the output of
every command under `logs/`; results.json keeps English field names for scripts, and `highlights.md` holds the few
lines release.yml puts in the release notes (#516). The numbers are
reported, not judged: nothing fails because head is slower.
"""

import argparse
import ctypes
import ctypes.wintypes as wt
import html
import json
import math
import os
import random
import shutil
import statistics
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request

import psutil

VERSIONS = ("base", "head")
TOKEN = "bench"
SAMPLE_SECONDS = 0.02
MIB = 1024 * 1024

# ---- Windows job objects: CPU, I/O and peak memory of a process and its children ----

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
ntdll = ctypes.WinDLL("ntdll")


class IoCounters(ctypes.Structure):
    _fields_ = [(n, ctypes.c_ulonglong) for n in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]


class BasicAccounting(ctypes.Structure):
    _fields_ = [
        ("TotalUserTime", ctypes.c_longlong),
        ("TotalKernelTime", ctypes.c_longlong),
        ("ThisPeriodTotalUserTime", ctypes.c_longlong),
        ("ThisPeriodTotalKernelTime", ctypes.c_longlong),
        ("TotalPageFaultCount", wt.DWORD),
        ("TotalProcesses", wt.DWORD),
        ("ActiveProcesses", wt.DWORD),
        ("TotalTerminatedProcesses", wt.DWORD),
    ]


class BasicAndIoAccounting(ctypes.Structure):
    _fields_ = [("Basic", BasicAccounting), ("Io", IoCounters)]


class BasicLimit(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_longlong),
        ("PerJobUserTimeLimit", ctypes.c_longlong),
        ("LimitFlags", wt.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wt.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wt.DWORD),
        ("SchedulingClass", wt.DWORD),
    ]


class ExtendedLimit(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", BasicLimit),
        ("IoInfo", IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("PageFaultCount", wt.DWORD)] + [(n, ctypes.c_size_t) for n in (
        "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
        "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]


kernel32.CreateJobObjectW.restype = wt.HANDLE
kernel32.CreateJobObjectW.argtypes = (ctypes.c_void_p, wt.LPCWSTR)
kernel32.SetInformationJobObject.argtypes = (wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.DWORD)
kernel32.QueryInformationJobObject.argtypes = (wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.DWORD, ctypes.c_void_p)
kernel32.AssignProcessToJobObject.argtypes = (wt.HANDLE, wt.HANDLE)
kernel32.CloseHandle.argtypes = (wt.HANDLE,)
kernel32.K32GetProcessMemoryInfo.argtypes = (wt.HANDLE, ctypes.c_void_p, wt.DWORD)
ntdll.NtResumeProcess.argtypes = (wt.HANDLE,)

CREATE_SUSPENDED = 0x4
JOB_BASIC_AND_IO_ACCOUNTING = 8
JOB_EXTENDED_LIMIT = 9
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000


def check(ok, what):
    if not ok:
        raise OSError(ctypes.get_last_error(), what)


class Job:
    """A job object holding one process (started suspended, so its children are in it from the start)."""

    def __init__(self):
        self.handle = kernel32.CreateJobObjectW(None, None)
        check(self.handle, "CreateJobObject")
        limit = ExtendedLimit()
        limit.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        check(kernel32.SetInformationJobObject(self.handle, JOB_EXTENDED_LIMIT, ctypes.byref(limit),
                                               ctypes.sizeof(limit)), "SetInformationJobObject")

    def start(self, args, **kw):
        proc = subprocess.Popen(args, creationflags=CREATE_SUSPENDED, **kw)
        check(kernel32.AssignProcessToJobObject(self.handle, int(proc._handle)), "AssignProcessToJobObject")
        ntdll.NtResumeProcess(int(proc._handle))
        return proc

    def accounting(self, proc):
        acc, ext = BasicAndIoAccounting(), ExtendedLimit()
        check(kernel32.QueryInformationJobObject(self.handle, JOB_BASIC_AND_IO_ACCOUNTING, ctypes.byref(acc),
                                                 ctypes.sizeof(acc), None), "QueryInformationJobObject")
        check(kernel32.QueryInformationJobObject(self.handle, JOB_EXTENDED_LIMIT, ctypes.byref(ext),
                                                 ctypes.sizeof(ext), None), "QueryInformationJobObject")
        mem = ProcessMemoryCounters()
        mem.cb = ctypes.sizeof(mem)
        # the process handle stays open after the exit, and with it these counters
        check(kernel32.K32GetProcessMemoryInfo(int(proc._handle), ctypes.byref(mem), mem.cb), "GetProcessMemoryInfo")
        return {
            "user_ms": acc.Basic.TotalUserTime / 1e4,
            "kernel_ms": acc.Basic.TotalKernelTime / 1e4,
            "page_faults": acc.Basic.TotalPageFaultCount,
            "processes": acc.Basic.TotalProcesses,
            "read_bytes": acc.Io.ReadTransferCount,
            "write_bytes": acc.Io.WriteTransferCount,
            "other_io_bytes": acc.Io.OtherTransferCount,
            "read_ops": acc.Io.ReadOperationCount,
            "write_ops": acc.Io.WriteOperationCount,
            "peak_working_set": mem.PeakWorkingSetSize,
            "peak_job_commit": ext.PeakJobMemoryUsed,
        }

    def close(self):
        kernel32.CloseHandle(self.handle)


class Sampler(threading.Thread):
    """Working set and thread count of one process, every `interval` seconds, until it exits or `stop` is called."""

    def __init__(self, pid, start, interval=SAMPLE_SECONDS):
        super().__init__(daemon=True)
        self.proc, self.start_time, self.interval = psutil.Process(pid), start, interval
        self.samples, self.done = [], threading.Event()

    def run(self):
        while not self.done.is_set():
            try:
                with self.proc.oneshot():
                    rss, threads = self.proc.memory_info().rss, self.proc.num_threads()
            except psutil.Error:
                return
            self.samples.append((round(time.perf_counter() - self.start_time, 3), rss, threads))
            self.done.wait(self.interval)

    def stop(self):
        self.done.set()
        self.join()


# ---- helpers ----

def dir_bytes(path):
    total = 0
    for base, _, names in os.walk(path):
        for name in names:
            try:
                total += os.path.getsize(os.path.join(base, name))
            except OSError:
                pass
    return total


def median(values):
    values = [v for v in values if v is not None]
    return statistics.median(values) if values else None


def ratio(head, base):
    return head / base if head is not None and base else None


def fresh_home(template, home):
    """A home holding the configuration of `template` only: no cache, no saved state."""
    shutil.rmtree(home, ignore_errors=True)
    os.makedirs(home)
    shutil.copy2(os.path.join(template, "config.json"), home)


def clear_caches(home):
    shutil.rmtree(os.path.join(home, "cache"), ignore_errors=True)


def environment(home):
    return dict(os.environ, WEFLOW_HOME=home, WEFLOW_LANG="en", NO_COLOR="1")


# ---- the commands ----

def commands(manifest, out, cursor):
    """The heaviest commands of the CLI on the benchmark account, heaviest first, then the message reads of #489:
    (name, argv, items, output). `cursor` is the `_cursor` of a message in the middle of the biggest group."""
    convs = manifest["conversations"]
    group = convs[0]
    private = max((c for c in convs if c["kind"] == "private"), key=lambda c: c["messages"])
    big, n = group["username"], group["messages"]
    o = lambda name: os.path.join(out, name)
    pages = [
        ("chat messages first page", ["chat", "messages", big, "--limit", "50"], 50, None),
        ("chat messages offset", ["chat", "messages", big, "--limit", "50", "--offset", str(n // 2)], 50, None),
    ]
    if cursor:
        pages.append(("chat messages cursor", ["chat", "messages", big, "--limit", "50", "--before", cursor], 50, None))
    return [
        ("export media images", ["export", "media", "--type", "image", "--session", big, "--out", o("media")],
         None, o("media")),
        ("export messages json with images",
         ["export", "messages", big, "--format", "json", "--media", "image", "--out", o("mi.json")], n, o("mi.json")),
        ("export messages json", ["export", "messages", big, "--format", "json", "--out", o("m.json")], n,
         o("m.json")),
        ("export messages chatlab", ["export", "messages", big, "--format", "chatlab", "--out", o("m.chatlab.json")],
         n, o("m.chatlab.json")),
        ("export messages excel", ["export", "messages", big, "--format", "excel", "--out", o("m.xlsx")], n,
         o("m.xlsx")),
        ("export messages html", ["export", "messages", big, "--format", "html", "--out", o("m.html")], n,
         o("m.html")),
        ("export messages txt", ["export", "messages", big, "--out", o("m.txt")], n, o("m.txt")),
        ("report annual", ["report", "annual", "generate"], manifest["messages"], None),
        # every run starts without a cache folder (clear_caches), so a version that kept the aggregate counts too
        ("analytics overall", ["analytics", "overall"], None, None),
        ("group members", ["group", "members", big, "--counts"], n, None),
        ("group ranking", ["group", "ranking", big], n, None),
        ("chat export-stats", ["chat", "export-stats", big], n, None),
        ("chat date-counts", ["chat", "date-counts", big], n, None),
        ("chat search", ["chat", "search", "the", "--limit", "200"], None, None),
        ("report dual", ["report", "dual", "generate", "--friend", private["username"]], private["messages"], None),
        *pages,
    ]


def middle_cursor(exe, home, manifest):
    """The `_cursor` of the message in the middle of the biggest group, as `exe` gives it (None if it does not)."""
    big = manifest["conversations"][0]
    proc = subprocess.run([exe, "chat", "messages", big["username"], "--limit", "1",
                           "--offset", str(big["messages"] // 2), "--json"],
                          env=environment(home), capture_output=True, stdin=subprocess.DEVNULL)
    try:
        return json.loads(proc.stdout)["data"][0]["_cursor"]
    except (ValueError, LookupError, TypeError):
        return None


def count_allocations(exe, home, argv, out_dir, log):
    """One more run of a command with a binary built with the `alloc-stats` feature (#513), untimed: what it
    allocated, or None when it failed or wrote no counts."""
    clear_caches(home)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    report = log + ".alloc.json"
    proc = subprocess.run([exe, *argv, "--json"], env=dict(environment(home), WEFLOW_ALLOC_STATS=report),
                          capture_output=True, stdin=subprocess.DEVNULL)
    with open(log, "wb") as f:
        f.write(proc.stdout + b"\n--- stderr ---\n" + proc.stderr)
    if proc.returncode != 0 or not os.path.exists(report):
        return None
    with open(report, encoding="utf-8") as f:
        counts = json.load(f)
    return {"allocations": counts["allocations"] + counts["reallocations"],
            "allocated_bytes": counts["allocatedBytes"], "peak_live_bytes": counts["peakLiveBytes"]}


def run_command(exe, home, args, out_dir, log):
    clear_caches(home)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    job = Job()
    begin = time.perf_counter()
    # the options go after the command, the one place every release takes them (v4.0.0 refused them before it)
    proc = job.start([exe, *args, "--json", "--progress"], env=environment(home), stdin=subprocess.DEVNULL,
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    sampler = Sampler(proc.pid, begin)
    sampler.start()
    first = {"event": None, "batch": None}
    err_lines = []

    def read_err():
        for raw in proc.stderr:
            now = time.perf_counter() - begin
            line = raw.decode("utf-8", "replace").rstrip()
            err_lines.append(line)
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get("type") != "progress":
                continue
            first["event"] = first["event"] or now
            if (event.get("total") or 0) > 1 and (event.get("current") or 0) > 0:
                first["batch"] = first["batch"] or now

    reader = threading.Thread(target=read_err, daemon=True)
    reader.start()
    stdout = proc.stdout.read()
    proc.wait()
    wall = time.perf_counter() - begin
    reader.join()
    sampler.stop()
    acc = job.accounting(proc)
    job.close()
    with open(log, "wb") as f:
        f.write(stdout + b"\n--- stderr ---\n" + "\n".join(err_lines).encode())
    samples = sampler.samples
    return {
        "exit_code": proc.returncode,
        "wall_ms": wall * 1000,
        "first_event_ms": first["event"] and first["event"] * 1000,
        "first_batch_ms": first["batch"] and first["batch"] * 1000,
        **acc,
        "cpu_ms": acc["user_ms"] + acc["kernel_ms"],
        "peak_threads": max((s[2] for s in samples), default=None),
        "stdout_bytes": len(stdout),
        "output_bytes": dir_bytes(out_dir),
        "cache_bytes": dir_bytes(os.path.join(home, "cache")),
        "samples": samples,
    }


CPU_NOTE = "进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。"
IO_NOTE = "逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。"
METRICS = [
    # (key, label, unit, scale, note): the labels and notes are the report's, which is in Chinese (#259)
    ("wall_ms", "耗时", "ms", 1, "从启动到退出。括号里是最快和最慢的一次。"),
    ("first_batch_ms", "第一批结果", "ms", 1,
     "从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。"),
    ("user_ms", "用户态 CPU", "ms", 1, CPU_NOTE),
    ("kernel_ms", "内核态 CPU", "ms", 1, CPU_NOTE),
    ("peak_working_set", "峰值工作集", "MiB", MIB, "主进程占用物理内存的峰值。"),
    ("peak_job_commit", "峰值提交内存", "MiB", MIB, "进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。"),
    ("peak_threads", "峰值线程数", "", 1, "每 20 ms 采样一次主进程得到的最大值。"),
    ("read_bytes", "读取", "MiB", MIB, IO_NOTE),
    ("write_bytes", "写入", "MiB", MIB, IO_NOTE),
    ("page_faults", "页错误", "", 1, "进程及其子进程，包括软页错误。"),
]
# below these differences (in the metric's own unit) a ratio is not marked: the timer's resolution, the noise of a
# run, or a few MiB of a working set
NOISE = {"wall_ms": 20, "first_batch_ms": 20, "user_ms": 50, "kernel_ms": 50, "peak_working_set": 5 * MIB,
         "peak_job_commit": 5 * MIB, "peak_threads": 1, "read_bytes": MIB, "write_bytes": MIB,
         "allocated_bytes": MIB, "peak_live_bytes": 5 * MIB}
NOISE_MS = 5
THROUGHPUT = ("items_per_s", "吞吐量", "消息/秒", 1, "命令处理的消息数除以耗时。这是唯一一个越高越好的指标。")
ALLOC_NOTE = ("用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；"
              "没有这个 feature 的版本（#513 之前）这一项为空。")
ALLOC_METRICS = [
    ("allocations", "分配次数", "", 1, "分配和重新分配的次数。" + ALLOC_NOTE),
    ("allocated_bytes", "累计分配", "MiB", MIB, "每次分配的大小之和，重新分配只计增长的部分。" + ALLOC_NOTE),
    ("peak_live_bytes", "峰值存活堆", "MiB", MIB, "同一时刻仍未释放的分配之和的最大值。" + ALLOC_NOTE),
]


def bench_commands(args, exes, alloc_exes, homes, manifest, out):
    work = os.path.join(out, "work", "output")
    results = {}
    cursor = middle_cursor(exes["head"], homes["head"], manifest)
    for name, argv, items, _ in commands(manifest, work, cursor):
        if args.only and name not in args.only:
            continue
        print(f"== {name}", flush=True)
        runs = {v: [] for v in VERSIONS}
        slug = name.replace(" ", "-")
        for v in VERSIONS:  # untimed: fills the system's file cache with the account
            run_command(exes[v], homes[v], argv, work, os.path.join(out, "logs", f"{slug}-{v}-warmup.txt"))
        for i in range(args.runs):
            order = VERSIONS if i % 2 == 0 else VERSIONS[::-1]
            for v in order:
                r = run_command(exes[v], homes[v], argv, work, os.path.join(out, "logs", f"{slug}-{v}-{i}.txt"))
                runs[v].append(r)
                print(f"   {v} run {i}: {r['wall_ms']:.0f} ms, exit {r['exit_code']}", flush=True)
        entry = {"argv": argv, "items": items, "runs": runs, "median": {}}
        for v in VERSIONS:
            ok = [r for r in runs[v] if r["exit_code"] == 0]
            entry["median"][v] = {k: median([r[k] for r in ok]) for k, *_ in METRICS + [("cpu_ms",),
                                                                                      ("output_bytes",),
                                                                                      ("cache_bytes",)]}
            entry["median"][v]["failed_runs"] = len(runs[v]) - len(ok)
            if items and entry["median"][v]["wall_ms"]:
                entry["median"][v]["items_per_s"] = items / (entry["median"][v]["wall_ms"] / 1000)
            # the counting build runs apart from the timed ones, so its few atomic operations per call cost nothing
            # to the timings
            if alloc_exes.get(v):
                counts = count_allocations(alloc_exes[v], homes[v], argv, work,
                                           os.path.join(out, "logs", f"{slug}-{v}-alloc.txt"))
                entry["median"][v].update(counts or {})
        results[name] = entry
    shutil.rmtree(work, ignore_errors=True)
    return results


# ---- sustained load on `weflow serve` ----

def start_server(job, exe, home, log):
    # a refused argument is answered on stdout: keep it for the error below
    stdout = open(log + ".stdout", "w+b")
    proc = job.start([exe, "serve", "--port", "0", "--api-token", TOKEN, "--api-only", "--json"],
                     env=environment(home), stdin=subprocess.DEVNULL, stdout=stdout, stderr=subprocess.PIPE)
    begin = time.perf_counter()
    lines = []
    url = None
    for raw in proc.stderr:
        line = raw.decode("utf-8", "replace").rstrip()
        lines.append(line)
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "server_started":
            url = event["url"].rstrip("/")
            break
    if url is None:
        proc.wait()
        stdout.seek(0)
        said = stdout.read().decode("utf-8", "replace").strip()
        stdout.close()
        raise RuntimeError(f"weflow serve exited ({proc.returncode}) before it listened: "
                           + "\n".join(filter(None, [said, *lines])))
    stdout.close()
    started_ms = (time.perf_counter() - begin) * 1000

    def drain():
        with open(log, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
            for raw in proc.stderr:
                f.write(raw.decode("utf-8", "replace"))

    threading.Thread(target=drain, daemon=True).start()
    return proc, url, started_ms


def get(url, path):
    req = urllib.request.Request(url + path, headers={"Authorization": f"Bearer {TOKEN}"})
    begin = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            body = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        body, status = e.read(), e.code
    return status, len(body), (time.perf_counter() - begin) * 1000


def load_routes(manifest):
    convs = manifest["conversations"]
    big = convs[0]
    others = [c for c in convs[1:] if c["messages"] > 0][:50]
    q = urllib.parse.quote

    def route(rng):
        pick = rng.random()
        if pick < 0.35:
            offset = rng.randrange(max(1, big["messages"] - 50))
            return "messages-any-page", f"/api/chat/messages?session-id={q(big['username'])}&limit=50&offset={offset}"
        if pick < 0.55:
            c = rng.choice(others)
            return "messages-first-page", f"/api/chat/messages?session-id={q(c['username'])}&limit=50"
        if pick < 0.70:
            return "sessions", "/api/chat/sessions?limit=60"
        if pick < 0.80:
            word = rng.choice(["the", "ok", "好", "明天", "哈哈", "test"])
            return "search", f"/api/chat/search?keyword={q(word)}&limit=50"
        if pick < 0.90:
            c = rng.choice(others)
            return "date-counts", f"/api/chat/date-counts?session-id={q(c['username'])}"
        return "contacts", "/api/chat/contacts"

    return route


def sustained_load(exe, template, manifest, seconds, workers, out, version):
    home = os.path.join(out, "work", f"serve-home-{version}")
    fresh_home(template, home)
    job = Job()
    proc, url, started_ms = start_server(job, exe, home, os.path.join(out, "logs", f"serve-load-{version}.txt"))
    begin = time.perf_counter()
    sampler = Sampler(proc.pid, begin, interval=0.5)
    sampler.start()
    route = load_routes(manifest)
    replies, lock = [], threading.Lock()
    deadline = begin + seconds

    def worker(seed):
        rng = random.Random(seed)
        while time.perf_counter() < deadline:
            kind, path = route(rng)
            status, size, ms = get(url, path)
            with lock:
                replies.append((round(time.perf_counter() - begin, 3), kind, status, size, ms))

    threads = [threading.Thread(target=worker, args=(i,), daemon=True) for i in range(workers)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    elapsed = time.perf_counter() - begin
    sampler.stop()
    proc.terminate()
    proc.wait()
    acc = job.accounting(proc)
    job.close()

    samples = sampler.samples
    # the working set swings by tens of MiB from one request to the next (a search allocates, the allocator hands
    # pages back), so the trend is read from the median of each quarter of the time: a leak raises every quarter
    quarters = [median([s[1] for s in samples if q * seconds / 4 <= s[0] < (q + 1) * seconds / 4]) for q in range(4)]
    quarters = [q / MIB if q is not None else None for q in quarters]
    by_kind = {}
    for _, kind, status, _, ms in replies:
        k = by_kind.setdefault(kind, {"ms": [], "errors": 0})
        k["ms"].append(ms)
        k["errors"] += status != 200
    lat = sorted(r[4] for r in replies)
    pct = lambda xs, p: xs[min(len(xs) - 1, int(p * len(xs)))] if xs else None
    return {
        "seconds": seconds,
        "workers": workers,
        "started_ms": started_ms,
        "requests": len(replies),
        "errors": sum(r[2] != 200 for r in replies),
        "requests_per_s": len(replies) / elapsed,
        "latency_ms": {"p50": pct(lat, 0.5), "p95": pct(lat, 0.95), "p99": pct(lat, 0.99), "max": lat[-1] if lat else None},
        "by_route": {k: {"n": len(v["ms"]), "errors": v["errors"], "p50": pct(sorted(v["ms"]), 0.5),
                         "p95": pct(sorted(v["ms"]), 0.95)} for k, v in sorted(by_kind.items())},
        "working_set_mib": {
            "quarter_medians": quarters,
            "peak": max(s[1] for s in samples) / MIB if samples else None,
            # from the 2nd quarter (the caches are warm) to the last
            "growth": quarters[3] - quarters[1] if None not in (quarters[1], quarters[3]) else None,
        },
        "peak_threads": max((s[2] for s in samples), default=None),
        **{k: acc[k] for k in ("user_ms", "kernel_ms", "read_bytes", "write_bytes", "peak_job_commit")},
        "samples": samples,
    }


def run_serve_bench(tool, exe, home, runs, out, version):
    save = os.path.join(out, f"serve-bench-{version}.json")
    work = os.path.join(out, "work", "serve-bench")
    args = [os.path.abspath(tool), work, "--home", home, "--weflow", exe, "--runs", str(runs), "--save", save]
    with open(os.path.join(out, "logs", f"serve-bench-{version}.txt"), "wb") as log:
        code = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                              env=dict(os.environ, WEFLOW_LANG="en")).returncode
    doc = None
    if os.path.exists(save):
        with open(save, encoding="utf-8") as f:
            doc = json.load(f)
    # a failed check on its synthetic account (an older base may lack a route) still leaves the timings
    return {"exit_code": code, "results": doc}


# ---- the report ----
#
# The report is for people (the one who started the run), so it is in Chinese like the repository's PRs and issues;
# command and route names, units and the field names of results.json stay as they are. It is built once as blocks:
# ("h", level, text), ("p", text), ("ul", [text]), ("table", [header], [[cell]]), then written as Markdown (the run's
# page, summary.md) and as HTML (report.html). Text may hold `code` and **bold**.

RATIO_HEADER = "head / base"
LOAD_ROUTES = {
    "messages-any-page": "`/api/chat/messages`，大群任意一页",
    "messages-first-page": "`/api/chat/messages`，其他会话第一页",
    "sessions": "`/api/chat/sessions`",
    "search": "`/api/chat/search`，全部会话",
    "date-counts": "`/api/chat/date-counts`",
    "contacts": "`/api/chat/contacts`",
}


def fmt(v, scale=1, unit=""):
    if v is None:
        return "-"
    if isinstance(v, int) and scale == 1:
        return f"{v:,} {unit}".strip()  # a count
    v = v / scale
    text = f"{v:,.1f}" if abs(v) < 10 or (abs(v) < 100 and scale != 1) else f"{v:,.0f}"
    return f"{text} {unit}".strip()


def mark(r, diff=None, noise=0):
    """▲ or ▼ when `r` is past 10 %, unless `diff` (head - base) is within `noise`."""
    if r is None or (diff is not None and abs(diff) <= noise):
        return ""
    return "▲" if r > 1.10 else "▼" if r < 0.90 else ""


def fmt_ratio(r, diff=None, noise=0):
    if r is None:
        return "-"
    m = mark(r, diff, noise)
    return f"{r:.2f}×" + (f" {m}" if m else "")


def ref_line(meta, v):
    m = meta[v]
    ref = m.get("ref") or ""
    commit = (m.get("commit") or "")[:10]
    named = f"`{ref}`（`{commit}`）" if ref and not commit.startswith(ref) else f"`{commit or ref}`"
    return f"{v}：{named}，{(m['version'] or '版本未知').split(' (')[0]}"


def report_blocks(doc):
    meta = doc["meta"]
    acc, machine = meta["account"], meta["machine"]
    blocks = [
        ("h", 1, "基准测试：base 与 head"),
        ("ul", [
            ref_line(meta, "base"),
            ref_line(meta, "head"),
            f"基准账号：种子 {acc['seed']}，{acc['messages']:,} 条消息，{len(acc['conversations'])} 个会话",
            f"runner：{machine['cpus']} 个逻辑 CPU，{machine['memory_gib']:.1f} GiB 内存，{machine['platform']}",
            f"每条命令先各运行一次不计时，再由 base 和 head 交替各运行 {meta['runs']} 次，取成功运行的中位数",
            "比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标"
            "（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好",
        ]),
        ("p", "**怎么看**：同一份代码在 runner 上对比时，耗时比值在 0.99–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。"
              "比值超过约 10%，并且两边最快到最慢的范围不重叠，才算真正的变化。"),
        ("h", 2, "二进制体积"),
        ("table", ["", "base", "head", RATIO_HEADER],
         [["`weflow.exe`", fmt(doc["binary"]["base"], MIB, "MiB"), fmt(doc["binary"]["head"], MIB, "MiB"),
           fmt_ratio(ratio(doc["binary"]["head"], doc["binary"]["base"]))]]),
    ]
    if doc["commands"]:
        blocks.append(("h", 2, "命令"))
        failed = [(n, v, e["median"][v]["failed_runs"]) for n, e in doc["commands"].items() for v in VERSIONS
                  if e["median"][v]["failed_runs"]]
        if failed:
            blocks.append(("p", "**有运行失败**（输出见 `logs/`，不计入中位数）："
                                + "，".join(f"`{n}`（{v} {k} 次）" for n, v, k in failed)))
        for key, label, unit, scale, note in METRICS + [THROUGHPUT] + ALLOC_METRICS:
            rows = []
            for name, e in doc["commands"].items():
                b, h = e["median"]["base"].get(key), e["median"]["head"].get(key)
                if b is None and h is None:
                    continue
                cells, r = [fmt(b, scale), fmt(h, scale)], ratio(h, b)
                ratio_cell = fmt_ratio(r, None if b is None or h is None else h - b, NOISE.get(key, 0))
                if key == "wall_ms":
                    spans = []
                    for i, v in enumerate(VERSIONS):
                        walls = [x["wall_ms"] for x in e["runs"][v] if x["exit_code"] == 0]
                        if walls:
                            spans.append((min(walls), max(walls)))
                            cells[i] += f"（{fmt(min(walls))}–{fmt(max(walls))}）"
                    # a ratio past 10 % whose ranges overlap is likely a command with two run times, not a change
                    if ratio_cell[-1] in "▲▼" and len(spans) == 2 \
                            and spans[0][0] <= spans[1][1] and spans[1][0] <= spans[0][1]:
                        ratio_cell += "，范围重叠"
                rows.append([f"`{name}`", *cells, ratio_cell])
            if rows:
                blocks += [("h", 3, f"{label}（{unit}）" if unit else label), ("p", note),
                           ("table", ["命令", "base", "head", RATIO_HEADER], rows)]
    if doc.get("load_failed"):
        blocks += [("h", 2, "`weflow serve` 持续负载"),
                   ("p", "没有完成：" + "；".join(f"{v}：{msg.replace(chr(10), ' ')[:300]}"
                                                for v, msg in doc["load_failed"].items())
                         + "。完整信息见 results.json 的 `load_failed`。")]
    if doc.get("load"):
        load = doc["load"]
        first = load["base"]
        blocks += [
            ("h", 2, "`weflow serve` 持续负载"),
            ("p", f"每个版本启动一个 `weflow serve`，用 {first['workers']} 个并发连续请求 {first['seconds']} 秒，"
                  "混合消息分页、会话列表、全局搜索、日期统计和联系人。内存按时间分成四段，各取中位数："
                  "第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。"),
        ]
        # (label, value, noise); no noise: no ratio, as for a growth or an error count near zero
        rows = [("每秒请求数", lambda d: d["requests_per_s"], 0),
                ("错误数", lambda d: d["errors"], None),
                ("延迟 p50 (ms)", lambda d: d["latency_ms"]["p50"], NOISE_MS),
                ("延迟 p95 (ms)", lambda d: d["latency_ms"]["p95"], NOISE_MS),
                ("延迟 p99 (ms)", lambda d: d["latency_ms"]["p99"], NOISE_MS),
                ("启动到开始监听 (ms)", lambda d: d["started_ms"], NOISE_MS),
                *[(f"工作集，第 {q + 1} 段中位数 (MiB)", lambda d, q=q: d["working_set_mib"]["quarter_medians"][q], 5)
                  for q in range(4)],
                ("第 2 段到第 4 段的增长 (MiB)", lambda d: d["working_set_mib"]["growth"], None),
                ("峰值工作集 (MiB)", lambda d: d["working_set_mib"]["peak"], 5),
                ("峰值线程数", lambda d: d["peak_threads"], 1),
                ("CPU 时间 (ms)", lambda d: d["user_ms"] + d["kernel_ms"], 50)]
        table = []
        for label, pick, noise in rows:
            b, h = pick(load["base"]), pick(load["head"])
            r = None if noise is None else ratio(h, b)
            diff = None if b is None or h is None else h - b
            table.append([label, fmt(b), fmt(h), fmt_ratio(r, diff, noise or 0)])
        blocks.append(("table", ["", "base", "head", RATIO_HEADER], table))
        rb, rh = load["base"]["by_route"], load["head"]["by_route"]
        cell = lambda d: f"{d['n']} 次，{fmt(d['p50'])} / {fmt(d['p95'])}" if d else "-"
        p50 = lambda d, k: (d.get(k) or {}).get("p50")
        blocks += [
            ("p", "各路由：请求次数，延迟 p50 / p95（ms）。"),
            ("table", ["路由", "base", "head", f"p50 {RATIO_HEADER}"],
             [[LOAD_ROUTES.get(k, f"`{k}`"), cell(rb.get(k)), cell(rh.get(k)),
               fmt_ratio(ratio(p50(rh, k), p50(rb, k)),
                         None if None in (p50(rh, k), p50(rb, k)) else p50(rh, k) - p50(rb, k), NOISE_MS)]
              for k in [k for k in LOAD_ROUTES if k in rb or k in rh] + sorted((set(rb) | set(rh)) - set(LOAD_ROUTES))]),
        ]
    sb = doc.get("serve_bench")
    if sb:
        code = lambda v: sb[v]["exit_code"]
        blocks += [
            ("h", 2, "serve_bench"),
            ("p", "各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；"
                  "`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。"
                  f"退出码：base {code('base')}，head {code('head')}"
                  + ("。" if code("base") == 0 and code("head") == 0 else
                     "。非 0 表示它在自己的合成账号上有检查没通过（旧版本缺少某个路由时会这样），"
                     "计时照常保留，原因见 `logs/serve-bench-*.txt`。")
                  + "某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由，返回 404 只要零点几毫秒），"
                    "格子里注明返回的状态，不计算比值。"
                  + "".join(f"{v} 没有计时结果：它的 `weflow serve` 没有启动（v5.0.0 之前的版本没有现在的命令 API），"
                            f"见 `logs/serve-bench-{v}.txt`。"
                            for v in VERSIONS if not sb[v]["results"])),
        ]
        rb = (sb["base"]["results"] or {}).get("results", {})
        rh = (sb["head"]["results"] or {}).get("results", {})

        def status(row):
            """serve_bench notes the answers that were not 200 as `status {404: 5}`."""
            note = (row or {}).get("note") or ""
            return next((part.strip().removeprefix("status ") for part in note.split(";")
                         if part.strip().startswith("status {")), None)

        rows = []
        for route in sorted(set(rb) | set(rh)):
            b, h = (rb.get(route) or {}).get("median"), (rh.get(route) or {}).get("median")
            sb_, sh = status(rb.get(route)), status(rh.get(route))
            cells = [fmt(b) + (f"（状态 {sb_}）" if sb_ else ""), fmt(h) + (f"（状态 {sh}）" if sh else "")]
            r = "-" if sb_ or sh else fmt_ratio(ratio(h, b), None if None in (b, h) else h - b, NOISE_MS)
            rows.append([f"`{route}`", *cells, r])
        if rows:
            blocks.append(("table", ["路由", "base", "head", RATIO_HEADER], rows))
    return blocks


def highlights(doc):
    """A few lines for the release notes (#516): what changed past the noise, the rest in the report."""
    out = []
    b, h = doc["binary"]["base"], doc["binary"]["head"]
    out.append(f"二进制体积：{fmt(b, MIB, 'MiB')} → {fmt(h, MIB, 'MiB')}（{ratio(h, b):.2f}×）")
    commands = doc.get("commands") or {}
    if commands:
        failed = {v: [n for n, e in commands.items() if e["median"][v]["failed_runs"]] for v in VERSIONS}
        for v in VERSIONS:
            if len(failed[v]) == len(commands):
                out.append(f"{v} 的命令全部运行失败，见报告")
            elif failed[v]:
                out.append(f"{v} 有运行失败的命令：" + "，".join(f"`{n}`" for n in failed[v]))
        # a failed run ends early: only the commands that ran through on both sides are compared
        commands = {n: e for n, e in commands.items() if all(n not in failed[v] for v in VERSIONS)}
    if commands:
        changed = []
        for name, e in commands.items():
            mb, mh = e["median"]["base"].get("wall_ms"), e["median"]["head"].get("wall_ms")
            r = ratio(mh, mb)
            if not mark(r, None if None in (mb, mh) else mh - mb, NOISE["wall_ms"]):
                continue
            spans = [(min(w), max(w)) for w in ([x["wall_ms"] for x in e["runs"][v] if x["exit_code"] == 0]
                                                 for v in VERSIONS) if w]
            if len(spans) == 2 and spans[0][0] <= spans[1][1] and spans[1][0] <= spans[0][1]:
                continue  # the ranges overlap: a command with two run times, not a change
            changed.append(f"`{name}` {fmt(mb)} → {fmt(mh)} ms（{r:.2f}×）")
        if changed:
            out.append(f"命令耗时有变化：{'；'.join(changed)}；其余 {len(commands) - len(changed)} 条在噪声范围内")
        else:
            out.append(f"命令耗时：{len(commands)} 条命令都在噪声范围内")
    if doc.get("load_failed"):
        out.append("`weflow serve` 持续负载没有完成，见报告")
    elif doc.get("load"):
        lb, lh = doc["load"]["base"], doc["load"]["head"]
        qb, qh = lb["working_set_mib"]["quarter_medians"][3], lh["working_set_mib"]["quarter_medians"][3]
        line = (f"`weflow serve` 持续负载：每秒请求数 {fmt(lb['requests_per_s'])} → {fmt(lh['requests_per_s'])}，"
                f"延迟 p50 {fmt(lb['latency_ms']['p50'])} → {fmt(lh['latency_ms']['p50'])} ms，"
                f"工作集 {fmt(qb)} → {fmt(qh)} MiB")
        growth = lh["working_set_mib"]["growth"]
        if growth is not None and growth > 20:
            line += f"；head 的内存在后半段上升了 {fmt(growth)} MiB"
        out.append(line)
    sb = doc.get("serve_bench")
    if sb:
        rb = (sb["base"]["results"] or {}).get("results", {})
        rh = (sb["head"]["results"] or {}).get("results", {})
        ups = downs = compared = 0
        for route in set(rb) & set(rh):
            notes = (rb[route].get("note") or "") + (rh[route].get("note") or "")
            mb, mh = rb[route].get("median"), rh[route].get("median")
            if "status {" in notes or None in (mb, mh):
                continue
            compared += 1
            m = mark(ratio(mh, mb), mh - mb, NOISE_MS)
            ups, downs = ups + (m == "▲"), downs + (m == "▼")
        if not compared:
            # an older base whose server never started (or answers none of these routes) leaves nothing to compare
            out.append("serve_bench：没有两边都能计时的路由，见报告")
        elif ups or downs:
            out.append(f"serve_bench：{downs} 个路由快了 10% 以上，{ups} 个慢了 10% 以上")
        else:
            out.append(f"serve_bench：{compared} 个路由都在噪声范围内")
    return out


def to_markdown(blocks):
    out = []
    for block in blocks:
        kind = block[0]
        if kind == "h":
            out.append("#" * block[1] + " " + block[2])
        elif kind == "p":
            out.append(block[1])
        elif kind == "ul":
            out.append("\n".join(f"- {item}" for item in block[1]))
        elif kind == "table":
            row = lambda cells: "| " + " | ".join(c.replace("|", "\\|") for c in cells) + " |"
            out.append("\n".join([row(block[1]), "|" + "---|" * len(block[1])] + [row(r) for r in block[2]]))
        out.append("")
    return "\n".join(out)


def inline_html(text):
    """`code` and **bold** of a report text, the rest escaped."""
    parts = html.escape(text).split("`")
    text = "".join(f"<code>{p}</code>" if i % 2 else p for i, p in enumerate(parts))
    parts = text.split("**")
    return "".join(f"<strong>{p}</strong>" if i % 2 else p for i, p in enumerate(parts))


def to_html(blocks):
    out = []
    for block in blocks:
        kind = block[0]
        if kind == "h":
            out.append(f"<h{block[1]}>{inline_html(block[2])}</h{block[1]}>")
        elif kind == "p":
            out.append(f"<p>{inline_html(block[1])}</p>")
        elif kind == "ul":
            out.append("<ul>" + "".join(f"<li>{inline_html(i)}</li>" for i in block[1]) + "</ul>")
        elif kind == "table":
            head = "".join(f"<th>{inline_html(c)}</th>" for c in block[1])
            body = "".join("<tr>" + "".join(f"<td>{inline_html(c)}</td>" for c in r) + "</tr>" for r in block[2])
            out.append(f"<div class=scroll><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>")
    return "\n".join(out)


COLOURS = {"base": "#2563eb", "head": "#dc2626"}


def nice_step(span, ticks=4):
    """A step of 1, 2 or 5 times a power of ten that cuts `span` into about `ticks` parts."""
    raw = span / ticks
    power = 10 ** math.floor(math.log10(raw))
    return next(m * power for m in (1, 2, 5, 10) if m * power >= raw)


def svg_curves(series, width=720, height=200):
    """Working set over time, a line per (version, [(seconds, bytes)]), from 0 on both axes."""
    points = [p for _, s in series for p in s]
    if not points:
        return "<p>没有采样。</p>"
    tstep = nice_step(max(p[0] for p in points) or 1)
    ystep = nice_step(max(p[1] for p in points) / MIB or 1)
    tmax = math.ceil(max(p[0] for p in points) / tstep) * tstep
    ymax = math.ceil(max(p[1] for p in points) / MIB / ystep) * ystep
    left, bottom, top, right = 64, 24, 10, 24
    x = lambda t: left + t / tmax * (width - left - right)
    y = lambda mib: height - bottom - mib / ymax * (height - bottom - top)
    label = lambda v: f"{v:g}"
    grid = []
    for k in range(round(ymax / ystep) + 1):
        mib = k * ystep
        grid.append(f'<line x1="{left}" y1="{y(mib):.1f}" x2="{width - right}" y2="{y(mib):.1f}" class=grid />'
                    f'<text x="{left - 6}" y="{y(mib) + 4:.1f}" text-anchor="end">{label(mib)} MiB</text>')
    for k in range(round(tmax / tstep) + 1):
        t = k * tstep
        grid.append(f'<text x="{x(t):.1f}" y="{height - 6}" text-anchor="middle">{label(round(t, 6))} s</text>')
    y_bytes = lambda b: y(b / MIB)
    lines = [f'<polyline fill="none" class={v} stroke-width="1.5" '
             f'points="{" ".join(f"{x(t):.1f},{y_bytes(b):.1f}" for t, b in s)}"><title>{v}</title></polyline>'
             for v, s in series]
    return (f'<svg viewBox="0 0 {width} {height}" role="img" aria-label="工作集随时间变化">'
            f'{"".join(grid)}{"".join(lines)}</svg>')


CURVES_NOTE = "主进程的工作集，每 20 ms 采样一次（持续负载每 0.5 s 一次）。命令取耗时居中的那次运行。"


def memory_series(doc):
    """(title, [(version, [(seconds, bytes)])]) of each chart: a command's median run, then the sustained load."""
    charts = []
    for name, e in doc["commands"].items():
        series = []
        for v in VERSIONS:
            ok = sorted((r for r in e["runs"][v] if r["exit_code"] == 0), key=lambda r: r["wall_ms"])
            if ok:
                series.append((v, [(t, rss) for t, rss, _ in ok[len(ok) // 2]["samples"]]))
        charts.append((f"`{name}`", series))
    if doc.get("load"):
        series = [(v, [(t, rss) for t, rss, _ in doc["load"][v]["samples"]]) for v in VERSIONS]
        charts.append(("`weflow serve` 持续负载", series))
    return charts


def mermaid_curves(series, points=60):
    """The curves of `svg_curves` as a Mermaid xychart for the Actions job summary, which drops inline SVG.

    A linear x-axis spreads each line evenly over the whole axis, so every line gets the same `points` columns: the
    peak of the samples falling in each column (a peak is never averaged away), the last value carried over a column
    with no sample, and 0 after the version's last sample (its process has exited).
    """
    series = [(v, s) for v, s in series if s]
    if not series:
        return "没有采样。"
    tmax = max(t for _, s in series for t, _ in s) or 1
    ystep = nice_step(max(b for _, s in series for _, b in s) / MIB or 1)
    ymax = math.ceil(max(b for _, s in series for _, b in s) / MIB / ystep) * ystep
    lines = []
    for _, s in series:
        columns = [None] * points
        for t, b in s:
            i = round(t / tmax * (points - 1))
            columns[i] = max(columns[i] or 0, b)
        last = max(i for i, c in enumerate(columns) if c is not None)
        value, values = next(c for c in columns if c is not None), []
        for i, c in enumerate(columns):
            value = c if c is not None else value
            values.append(value / MIB if i <= last else 0)
        lines.append("    line [" + ", ".join(f"{m:.1f}" for m in values) + "]")
    config = {"xyChart": {"width": 720, "height": 260},
              "themeVariables": {"xyChart": {"plotColorPalette": ", ".join(COLOURS[v] for v, _ in series)}}}
    return "\n".join([
        "```mermaid",
        f"%%{{init: {json.dumps(config)}}}%%",
        "xychart-beta",
        f'    x-axis "秒" 0 --> {tmax:g}',
        f'    y-axis "MiB" 0 --> {ymax:g}',
        *lines,
        "```",
    ])


def markdown_curves(doc):
    """The 内存曲线 section of summary.md and of the job summary."""
    legend = "，".join(f"{'蓝' if v == 'base' else '红'}线 {v}" for v in VERSIONS)
    out = ["## 内存曲线", "", f"{CURVES_NOTE}{legend}；先结束的版本在退出后记为 0。", ""]
    for title, series in memory_series(doc):
        out += [f"### {title}", "", mermaid_curves(series), ""]
    return "\n".join(out)


# follows the system's light or dark theme; the light curve colours are COLOURS, which the Mermaid charts also use
REPORT_CSS = f"""
:root{{color-scheme:light dark;--fg:#1f2328;--bg:#fff;--muted:#57606a;--border:#d0d7de;--subtle:#f6f8fa;
--stripe:#fafbfc;--grid:#e5e7eb;--base:{COLOURS["base"]};--head:{COLOURS["head"]}}}
@media (prefers-color-scheme:dark){{:root{{--fg:#e6edf3;--bg:#0d1117;--muted:#9198a1;--border:#3d444d;
--subtle:#151b23;--stripe:#10151c;--grid:#262c36;--base:#4493f8;--head:#f85149}}}}
body{{font:14px/1.6 system-ui,"Microsoft YaHei",sans-serif;margin:0;padding:16px;max-width:960px;
color:var(--fg);background:var(--bg)}}
h1{{font-size:22px}}h2{{font-size:18px;margin-top:32px;border-bottom:1px solid var(--border)}}h3{{font-size:15px}}
code{{font:12px ui-monospace,Consolas,monospace;background:var(--subtle);padding:1px 4px;border-radius:4px}}
.scroll{{overflow-x:auto}}table{{border-collapse:collapse;margin:8px 0 16px}}
th,td{{border:1px solid var(--border);padding:4px 10px;text-align:right;white-space:nowrap}}
th:first-child,td:first-child{{text-align:left}}th{{background:var(--subtle)}}
tbody tr:nth-child(even){{background:var(--stripe)}}
svg{{width:100%;max-width:720px;font:11px system-ui;fill:var(--muted)}}.grid{{stroke:var(--grid)}}
polyline.base{{stroke:var(--base)}}polyline.head{{stroke:var(--head)}}i.base{{background:var(--base)}}i.head{{background:var(--head)}}
.legend span{{margin-right:16px}}.legend i{{display:inline-block;width:12px;height:3px;margin-right:6px;vertical-align:middle}}
""".strip()


def report_html(doc, blocks):
    charts = [f"<h3>{inline_html(title)}</h3>{svg_curves(series)}" for title, series in memory_series(doc)]
    legend = "".join(f'<span><i class={v}></i>{v}</span>' for v in VERSIONS)
    return f"""<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>WeFlow 基准测试</title>
<style>
{REPORT_CSS}
</style></head><body>
{to_html(blocks)}
<h2>内存曲线</h2>
<p>{CURVES_NOTE}</p>
<p class=legend>{legend}</p>
{"".join(charts)}
</body></html>
"""


def binary_info(exe):
    out = subprocess.run([exe, "--version"], capture_output=True, text=True).stdout.strip()
    return {"path": os.path.abspath(exe), "version": out, "bytes": os.path.getsize(exe)}


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--base", required=True)
    p.add_argument("--head", required=True)
    p.add_argument("--base-commit", default="")
    p.add_argument("--head-commit", default="")
    p.add_argument("--base-ref", default="", help="the name given for base (a tag, a branch), shown in the report")
    p.add_argument("--head-ref", default="")
    p.add_argument("--account", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--load-seconds", type=int, default=90)
    p.add_argument("--load-workers", type=int, default=4)
    p.add_argument("--serve-bench")
    p.add_argument("--base-alloc", help="base built with the alloc-stats feature (#513): allocation counts")
    p.add_argument("--head-alloc")
    p.add_argument("--only", type=lambda s: [x.strip() for x in s.split(",") if x.strip()],
                   help="comma separated: command names, plus `load` and `serve_bench`")
    args = p.parse_args()
    # the runner's console is cp1252, which has no ▲
    sys.stdout.reconfigure(encoding="utf-8")

    out = os.path.abspath(args.out)
    shutil.rmtree(os.path.join(out, "logs"), ignore_errors=True)
    os.makedirs(os.path.join(out, "logs"))
    template = os.path.join(args.account, "home")
    with open(os.path.join(args.account, "manifest.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    exes = {"base": os.path.abspath(args.base), "head": os.path.abspath(args.head)}
    homes = {v: os.path.join(out, "work", f"home-{v}") for v in VERSIONS}
    for v in VERSIONS:
        fresh_home(template, homes[v])

    wanted = lambda part: not args.only or part in args.only
    doc = {
        "meta": {
            "base": {**binary_info(exes["base"]), "commit": args.base_commit, "ref": args.base_ref},
            "head": {**binary_info(exes["head"]), "commit": args.head_commit, "ref": args.head_ref},
            "runs": args.runs,
            "account": {k: manifest[k] for k in ("seed", "messages", "messages_in_wal", "by_type", "conversations")},
            "machine": {"cpus": os.cpu_count(), "memory_gib": psutil.virtual_memory().total / 1024 ** 3,
                        "platform": f"{sys.platform} {os.environ.get('ImageOS', '')}".strip(),
                        "runner": os.environ.get("RUNNER_NAME")},
        },
        "binary": {v: os.path.getsize(exes[v]) for v in VERSIONS},
    }
    alloc_exes = {v: os.path.abspath(x) for v, x in (("base", args.base_alloc), ("head", args.head_alloc)) if x}
    doc["commands"] = bench_commands(args, exes, alloc_exes, homes, manifest, out)
    if wanted("load") and args.load_seconds > 0:
        print("== sustained load", flush=True)
        load, failed = {}, {}
        for v in VERSIONS:
            # an older base whose server does not start must not lose the numbers measured so far
            try:
                load[v] = sustained_load(exes[v], template, manifest, args.load_seconds, args.load_workers, out, v)
            except Exception as e:  # noqa: BLE001
                failed[v] = str(e)
        if failed:
            doc["load_failed"] = failed
        else:
            doc["load"] = load
    if args.serve_bench and wanted("serve_bench"):
        print("== serve_bench", flush=True)
        doc["serve_bench"] = {v: run_serve_bench(args.serve_bench, exes[v], homes[v], args.runs, out, v)
                              for v in VERSIONS}
    shutil.rmtree(os.path.join(out, "work"), ignore_errors=True)

    blocks = report_blocks(doc)
    markdown = to_markdown(blocks) + markdown_curves(doc)
    with open(os.path.join(out, "results.json"), "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, indent=1)
    with open(os.path.join(out, "summary.md"), "w", encoding="utf-8") as f:
        f.write(markdown)
    with open(os.path.join(out, "report.html"), "w", encoding="utf-8") as f:
        f.write(report_html(doc, blocks))
    with open(os.path.join(out, "highlights.md"), "w", encoding="utf-8") as f:
        f.write("".join(f"- {line}\n" for line in highlights(doc)))
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write(markdown + "\n")
    print(markdown)


if __name__ == "__main__":
    main()
