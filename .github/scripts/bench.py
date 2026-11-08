"""Benchmark one weflow binary on the benchmark account of #488, for the results of one commit (#601, #602).

usage: python bench.py measure --exe EXE --commit SHA --account DIR --out DIR
                               [--ref NAME] [--alloc EXE] [--anchor EXE --anchor-commit SHA [--anchor-alloc EXE]]
                               [--runs N] [--load-seconds S] [--serve-bench EXE] [--only NAME[,NAME...]]
       python bench.py env --account DIR [--load-seconds S]

Each commit is measured on its own and its `result.json` kept; any two results of the same environment are compared
later by `.github/bench/compare.mjs`. The environment (`env`, printed by the `env` mode) is what must stay the same
for two results to be compared, and only that, so that a long history stays one environment: the machine (CPU,
logical CPUs, memory), [`METHOD`] (how this script measures), the account (the hash of its `manifest.json`, which
names its profile, and the generator's layout from `spec.json`) and the length of the sustained load; its `key` names
it. The Windows build is kept with each result but not in the key: what an update changes shows in the anchor.

Two commits measured at different times lose what measuring them in turns gave: the machine drifts in between
(updates, heat, other work). So the `--anchor` binary, the same commit for every result of an environment, is measured
in turns with the target, through the same commands, load and `serve_bench`, and its numbers are kept next to the
target's: two results whose anchors differ have drifted by that much. When the target is the anchor itself its own
numbers are the anchor's. The CPU the machine's other processes use while measuring is sampled too (`quiet`).

`--account` is the work folder of `bench_account` (its `home` and `manifest.json`). Each binary gets a home of its
own, a copy of that configuration; its caches are removed before every run, so each run starts from the same state
(the files of the account stay in the system's file cache: a first, untimed run of every command fills it).

Measured, for the heaviest commands, each `--runs` times, target and anchor taking turns:
  - wall time, the time to the first progress event and to the first batch of work (`--progress`)
  - user and kernel CPU time, logical bytes read and written, page faults: the process and its children, from a job
    object; disk I/O apart from the system's file cache is not counted per process by Windows
  - peak working set of the process and peak committed memory of the job; a memory and thread-count curve sampled
    every 20 ms (the curve of the median run is kept), and the peak thread count
  - the size of what the command wrote and of the caches it left
  - with `--alloc` / `--anchor-alloc` (builds with the `alloc-stats` feature, #513), one more untimed run counting
    the allocations, the bytes allocated and the peak of live heap
Then, for each binary: a sustained load on `weflow serve` (`--load-seconds`, default 90) with the latency, the
throughput and the memory curve of the server, whose median by quarter of the time says whether it stays stable; and
`serve_bench` (when given), the latency of each route. Last, the size of the binary.

Writes `result.json` (field names in English, for compare.mjs and scripts) and the output of every command under
`logs/`. Nothing is judged here: nothing fails because a commit is slower.
"""

import argparse
import ctypes
import ctypes.wintypes as wt
import datetime
import hashlib
import json
import math
import os
import platform
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

# The version of the way a commit is measured: bump it when what is measured changes (the commands and their options,
# the load, how serve_bench is run or what it times, the numbers kept), so that results measured differently are
# never compared. A change that measures the same (a comment, a name in the report) keeps it, and with it the history.
METHOD = 1
# the commit measured and the one every result of its environment is measured next to
ROLES = ("target", "anchor")
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
    said = (stdout.decode("utf-8", "replace") + "\n".join(err_lines)).lower()
    return {
        "exit_code": proc.returncode,
        # the command line refused: a command, an option or a value this version does not have yet
        "unsupported": proc.returncode != 0 and any(s in said for s in UNSUPPORTED),
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


# the medians kept for each command
METRICS = ("wall_ms", "first_batch_ms", "user_ms", "kernel_ms", "cpu_ms", "peak_working_set", "peak_job_commit",
           "peak_threads", "read_bytes", "write_bytes", "page_faults", "output_bytes", "cache_bytes")
# at most this many points of a memory curve are kept
CURVE_POINTS = 300
# what weflow's command line (clap) says when it refuses what it does not know
UNSUPPORTED = ("unrecognized subcommand", "unexpected argument", "invalid value", "unrecognized option")


def curve(samples):
    """A memory curve as [[seconds, MiB]], thinned to CURVE_POINTS by keeping each stretch's peak."""
    if not samples:
        return []
    step = max(1, math.ceil(len(samples) / CURVE_POINTS))
    out = []
    for i in range(0, len(samples), step):
        part = samples[i:i + step]
        top = max(part, key=lambda s: s[1])
        out.append([round(part[0][0], 3), round(top[1] / MIB, 1)])
    return out


def summarize(runs, items):
    """What a command's runs by one binary come to: the medians of the runs that succeeded, every run without its
    samples, and the memory curve of the median run; `unsupported` when every run was refused by the command line
    (the version does not have the command yet), which is no failure."""
    ok = sorted((r for r in runs if r["exit_code"] == 0), key=lambda r: r["wall_ms"])
    unsupported = bool(runs) and all(r.get("unsupported") for r in runs)
    med = {k: median([r[k] for r in ok]) for k in METRICS}
    if items and med["wall_ms"]:
        med["items_per_s"] = items / (med["wall_ms"] / 1000)
    return {
        "median": med,
        "failed_runs": 0 if unsupported else len(runs) - len(ok),
        "unsupported": unsupported,
        "runs": [{k: v for k, v in r.items() if k != "samples"} for r in runs],
        "curve": curve(ok[len(ok) // 2]["samples"]) if ok else [],
    }


def bench_commands(args, exes, alloc_exes, homes, manifest, out):
    """{role: {command: summary}} for the roles in `exes`, which take turns run by run."""
    work = os.path.join(out, "work", "output")
    roles = [r for r in ROLES if r in exes]
    results = {r: {} for r in roles}
    cursor = middle_cursor(exes["target"], homes["target"], manifest)
    # a version whose messages carry no `_cursor` has no paging by cursor yet: noted like a command it does not know
    if not cursor and (not args.only or "chat messages cursor" in args.only):
        results["target"]["chat messages cursor"] = {"argv": None, "items": 50, **summarize([], 50),
                                                     "unsupported": True}
    for name, argv, items, _ in commands(manifest, work, cursor):
        if args.only and name not in args.only:
            continue
        print(f"== {name}", flush=True)
        runs = {r: [] for r in roles}
        slug = name.replace(" ", "-")
        for r in roles:  # untimed: fills the system's file cache with the account
            run_command(exes[r], homes[r], argv, work, os.path.join(out, "logs", f"{slug}-{r}-warmup.txt"))
        for i in range(args.runs):
            for r in (roles if i % 2 == 0 else roles[::-1]):
                run = run_command(exes[r], homes[r], argv, work, os.path.join(out, "logs", f"{slug}-{r}-{i}.txt"))
                runs[r].append(run)
                print(f"   {r} run {i}: {run['wall_ms']:.0f} ms, exit {run['exit_code']}", flush=True)
        for r in roles:
            entry = {"argv": argv, "items": items, **summarize(runs[r], items)}
            # the counting build runs apart from the timed ones, so its few atomic operations per call cost nothing
            # to the timings
            if alloc_exes.get(r):
                counts = count_allocations(alloc_exes[r], homes[r], argv, work,
                                           os.path.join(out, "logs", f"{slug}-{r}-alloc.txt"))
                entry["median"].update(counts or {})
            results[r][name] = entry
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


def sustained_load(exe, template, manifest, seconds, workers, out, role):
    home = os.path.join(out, "work", f"serve-home-{role}")
    fresh_home(template, home)
    job = Job()
    proc, url, started_ms = start_server(job, exe, home, os.path.join(out, "logs", f"serve-load-{role}.txt"))
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
        "curve": curve(samples),
    }


def run_serve_bench(tool, exe, home, runs, out, role):
    save = os.path.join(out, "work", f"serve-bench-{role}.json")
    work = os.path.join(out, "work", "serve-bench")
    args = [os.path.abspath(tool), work, "--home", home, "--weflow", exe, "--runs", str(runs), "--save", save]
    with open(os.path.join(out, "logs", f"serve-bench-{role}.txt"), "wb") as log:
        code = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                              env=dict(os.environ, WEFLOW_LANG="en")).returncode
    doc = None
    if os.path.exists(save):
        with open(save, encoding="utf-8") as f:
            doc = json.load(f)
    # a failed check on its synthetic account (an older base may lack a route) still leaves the timings
    return {"exit_code": code, "results": doc}


# ---- the machine while measuring ----

class OtherLoad(threading.Thread):
    """The CPU the machine's other processes use, as a share of all its CPUs, every `interval` seconds: this script
    and everything it starts are left out, so a high share means something else ran during the measurement."""

    def __init__(self, interval=2.0):
        super().__init__(daemon=True)
        self.interval, self.shares, self.done = interval, [], threading.Event()
        self.seen = {}  # pid -> (create time, CPU seconds), so a pid taken again by a new process starts afresh

    def _cpu(self):
        mine = {os.getpid()} | {c.pid for c in psutil.Process().children(recursive=True)}
        total, now = 0.0, {}
        for proc in psutil.process_iter(["pid", "create_time", "cpu_times"]):
            info = proc.info
            # pid 0 is the System Idle Process, whose CPU time is the idle time
            if info["pid"] in mine or info["pid"] == 0 or not info["cpu_times"]:
                continue
            spent = info["cpu_times"].user + info["cpu_times"].system
            now[info["pid"]] = (info["create_time"], spent)
            before = self.seen.get(info["pid"])
            if before and before[0] == info["create_time"]:
                total += max(0.0, spent - before[1])
        self.seen = now
        return total

    def run(self):
        self._cpu()
        last = time.perf_counter()
        while not self.done.wait(self.interval):
            spent, now = self._cpu(), time.perf_counter()
            self.shares.append(spent / ((now - last) * os.cpu_count()) * 100)
            last = now

    def stop(self):
        self.done.set()
        self.join()
        s = sorted(self.shares)
        if not s:
            return {"samples": 0}
        pick = lambda q: round(s[min(len(s) - 1, int(q * len(s)))], 1)
        return {"samples": len(s), "p50_pct": pick(0.5), "p95_pct": pick(0.95), "max_pct": round(s[-1], 1),
                "busy_share": round(sum(x > 10 for x in s) / len(s), 3)}


def cpu_name():
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0") as k:
            return winreg.QueryValueEx(k, "ProcessorNameString")[0].strip()
    except OSError:
        return platform.processor()


def environment_of(account, load_seconds, load_workers):
    """What two results must share to be compared, and the key that names it."""
    with open(os.path.join(account, "manifest.json"), "rb") as f:
        manifest_bytes = f.read()
    manifest = json.loads(manifest_bytes)
    with open(os.path.join(account, "spec.json"), encoding="utf-8") as f:
        layout = json.load(f)["layout"]
    env = {
        "machine": {"cpu": cpu_name(), "cpus": os.cpu_count(),
                    "memory_gib": round(psutil.virtual_memory().total / 1024 ** 3)},
        "method": METHOD,
        # what the account holds is known by its manifest, so only its hash, its size and its profile's name are kept
        "account": {"manifest": hashlib.sha256(manifest_bytes).hexdigest()[:16], "layout": layout,
                    "seed": manifest["seed"], "messages": manifest["messages"],
                    "conversations": len(manifest["conversations"]), "profile": manifest.get("profile")},
        "load": {"seconds": load_seconds, "workers": load_workers},
    }
    canonical = json.dumps(env, sort_keys=True, separators=(",", ":")).encode()
    return {"key": hashlib.sha256(canonical).hexdigest()[:12], **env}


def binary_info(exe):
    out = subprocess.run([exe, "--version"], capture_output=True, text=True).stdout.strip()
    return {"version": out, "bytes": os.path.getsize(exe)}


def measure(args):
    out = os.path.abspath(args.out)
    shutil.rmtree(os.path.join(out, "logs"), ignore_errors=True)
    os.makedirs(os.path.join(out, "logs"))
    template = os.path.join(args.account, "home")
    with open(os.path.join(args.account, "manifest.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    # the anchor is measured apart only when it is another commit
    own_anchor = not args.anchor or args.anchor_commit == args.commit
    exes = {"target": os.path.abspath(args.exe)}
    alloc_exes = {"target": args.alloc}
    if not own_anchor:
        exes["anchor"] = os.path.abspath(args.anchor)
        alloc_exes["anchor"] = args.anchor_alloc
    alloc_exes = {r: os.path.abspath(x) for r, x in alloc_exes.items() if x}
    homes = {r: os.path.join(out, "work", f"home-{r}") for r in exes}
    for r in exes:
        fresh_home(template, homes[r])

    quiet = OtherLoad()
    quiet.start()
    wanted = lambda part: not args.only or part in args.only
    parts = {r: {} for r in exes}
    for r, cmds in bench_commands(args, exes, alloc_exes, homes, manifest, out).items():
        parts[r]["commands"] = cmds
    if wanted("load") and args.load_seconds > 0:
        print("== sustained load", flush=True)
        for r in exes:
            # a commit whose server does not start must not lose the numbers measured so far
            try:
                parts[r]["load"] = sustained_load(exes[r], template, manifest, args.load_seconds, args.load_workers,
                                                  out, r)
            except Exception as e:  # noqa: BLE001
                parts[r]["load_failed"] = str(e)
    if args.serve_bench and wanted("serve_bench"):
        print("== serve_bench", flush=True)
        for r in exes:
            parts[r]["serve_bench"] = run_serve_bench(args.serve_bench, exes[r], homes[r], args.runs, out, r)
    shutil.rmtree(os.path.join(out, "work"), ignore_errors=True)

    target = binary_info(exes["target"])
    doc = {
        "format": 1,
        "commit": args.commit,
        "ref": args.ref or None,
        "version": target["version"],
        "binary_bytes": target["bytes"],
        "measured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "runs": args.runs,
        "only": args.only or None,
        "env": environment_of(args.account, args.load_seconds, args.load_workers),
        "os": f"{platform.system()} {platform.version()}",
        "quiet": quiet.stop(),
        **parts["target"],
    }
    if own_anchor:
        anchor = {"commit": args.commit, "version": target["version"], "binary_bytes": target["bytes"],
                  **json.loads(json.dumps(parts["target"]))}
    else:
        info = binary_info(exes["anchor"])
        anchor = {"commit": args.anchor_commit, "version": info["version"], "binary_bytes": info["bytes"],
                  **parts["anchor"]}
    # the drift is read from the medians and the wall times: the runs and the curves stay with the target
    for e in (anchor.get("commands") or {}).values():
        e["walls"] = [r["wall_ms"] for r in e["runs"] if r["exit_code"] == 0]
        for k in ("runs", "curve", "argv"):
            e.pop(k, None)
    if anchor.get("load"):
        anchor["load"].pop("curve", None)
    doc["anchor"] = anchor
    with open(os.path.join(out, "result.json"), "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, indent=1)
    print(f"result: {os.path.join(out, 'result.json')}; other processes used {doc['quiet'].get('p50_pct')} % of "
          f"the CPU (p95 {doc['quiet'].get('p95_pct')} %)")


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    modes = p.add_subparsers(dest="mode", required=True)
    m = modes.add_parser("measure", help="measure one commit")
    m.add_argument("--exe", required=True)
    m.add_argument("--commit", required=True)
    m.add_argument("--ref", default="", help="the name given for the commit (a tag, a branch)")
    m.add_argument("--alloc", help="the commit built with the alloc-stats feature (#513): allocation counts")
    m.add_argument("--anchor", help="the anchor of the environment, measured in turns with --exe")
    m.add_argument("--anchor-commit", default="")
    m.add_argument("--anchor-alloc")
    m.add_argument("--runs", type=int, default=5)
    m.add_argument("--serve-bench")
    m.add_argument("--only", type=lambda s: [x.strip() for x in s.split(",") if x.strip()],
                   help="comma separated: command names, plus `load` and `serve_bench`")
    m.add_argument("--out", required=True)
    e = modes.add_parser("env", help="print the environment and its key as JSON")
    for sub in (m, e):
        sub.add_argument("--account", required=True)
        sub.add_argument("--load-seconds", type=int, default=90)
        sub.add_argument("--load-workers", type=int, default=4)
    args = p.parse_args()
    # the runner's console may be cp1252
    sys.stdout.reconfigure(encoding="utf-8")
    if args.mode == "env":
        print(json.dumps(environment_of(args.account, args.load_seconds, args.load_workers),
                         ensure_ascii=False))
        return
    if args.anchor and not args.anchor_commit:
        p.error("--anchor needs --anchor-commit")
    measure(args)


if __name__ == "__main__":
    main()
