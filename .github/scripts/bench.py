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
Then, for each binary: a sustained load on `weflow serve` (`--load-seconds`, default 90) with the latency, the
throughput and the memory curve of the server, whose median by quarter of the time says whether it stays stable; and
`serve_bench` (when given), the latency of each route. Last, the size of each binary.

Writes `results.json` (everything, every run and sample), `summary.md` (also appended to `$GITHUB_STEP_SUMMARY`),
`report.html` (the tables and the memory curves) and the output of every command under `logs/`. The numbers are
reported, not judged: nothing fails because head is slower.
"""

import argparse
import ctypes
import ctypes.wintypes as wt
import html
import json
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

def commands(manifest, out):
    """The heaviest commands of the CLI on the benchmark account, heaviest first: (name, argv, items, output)."""
    convs = manifest["conversations"]
    group = convs[0]
    private = max((c for c in convs if c["kind"] == "private"), key=lambda c: c["messages"])
    big, n = group["username"], group["messages"]
    o = lambda name: os.path.join(out, name)
    return [
        ("export media images", ["export", "media", "--type", "image", "--session", big, "--out", o("media")],
         None, o("media")),
        ("export messages json", ["export", "messages", big, "--format", "json", "--out", o("m.json")], n,
         o("m.json")),
        ("export messages excel", ["export", "messages", big, "--format", "excel", "--out", o("m.xlsx")], n,
         o("m.xlsx")),
        ("export messages html", ["export", "messages", big, "--format", "html", "--out", o("m.html")], n,
         o("m.html")),
        ("export messages txt", ["export", "messages", big, "--out", o("m.txt")], n, o("m.txt")),
        ("report annual", ["report", "annual", "generate"], manifest["messages"], None),
        ("analytics overall", ["analytics", "overall", "--force"], None, None),
        ("group members", ["group", "members", big, "--counts", "--refresh"], n, None),
        ("group ranking", ["group", "ranking", big], n, None),
        ("chat export-stats", ["chat", "export-stats", big], n, None),
        ("chat date-counts", ["chat", "date-counts", big], n, None),
        ("chat search", ["chat", "search", "the", "--limit", "200"], None, None),
        ("report dual", ["report", "dual", "generate", "--friend", private["username"]], private["messages"], None),
    ]


def run_command(exe, home, args, out_dir, log):
    clear_caches(home)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    job = Job()
    begin = time.perf_counter()
    proc = job.start([exe, "--json", "--progress", *args], env=environment(home), stdin=subprocess.DEVNULL,
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


METRICS = [
    # (key, label, unit, scale)
    ("wall_ms", "wall", "ms", 1),
    ("first_batch_ms", "first batch", "ms", 1),
    ("user_ms", "user CPU", "ms", 1),
    ("kernel_ms", "kernel CPU", "ms", 1),
    ("peak_working_set", "peak working set", "MiB", MIB),
    ("peak_job_commit", "peak commit", "MiB", MIB),
    ("peak_threads", "peak threads", "", 1),
    ("read_bytes", "read", "MiB", MIB),
    ("write_bytes", "written", "MiB", MIB),
    ("page_faults", "page faults", "", 1),
]


def bench_commands(args, exes, homes, manifest, out):
    work = os.path.join(out, "work", "output")
    results = {}
    for name, argv, items, _ in commands(manifest, work):
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
        results[name] = entry
    shutil.rmtree(work, ignore_errors=True)
    return results


# ---- sustained load on `weflow serve` ----

def start_server(job, exe, home, log):
    proc = job.start([exe, "--json", "serve", "--port", "0", "--api-token", TOKEN, "--api-only"],
                     env=environment(home), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.PIPE)
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
        raise RuntimeError("weflow serve exited before it listened:\n" + "\n".join(lines))
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
            return "messages", f"/api/chat/messages?session-id={q(big['username'])}&limit=50&offset={offset}"
        if pick < 0.55:
            c = rng.choice(others)
            return "messages", f"/api/chat/messages?session-id={q(c['username'])}&limit=50"
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

def fmt(v, scale=1, unit=""):
    if v is None:
        return "-"
    v = v / scale
    text = f"{v:,.1f}" if abs(v) < 100 and scale != 1 else f"{v:,.0f}"
    return f"{text} {unit}".strip()


def fmt_ratio(r):
    if r is None:
        return "-"
    mark = " ▲" if r > 1.10 else " ▼" if r < 0.90 else ""
    return f"{r:.2f}×{mark}"


def summary_markdown(doc):
    lines = ["# Benchmark: base vs head", ""]
    meta = doc["meta"]
    lines += [f"- base `{meta['base']['commit']}`, head `{meta['head']['commit']}`",
              f"- account: seed {meta['account']['seed']}, {meta['account']['messages']:,} messages, "
              f"{len(meta['account']['conversations'])} conversations",
              f"- {meta['runs']} runs each, base and head taking turns, medians of the successful runs (wall: the fastest and the slowest run in brackets); "
              f"ratio = head / base (▲ more than 10 % up, ▼ more than 10 % down)",
              f"- runner: {meta['machine']['cpus']} logical CPUs, {meta['machine']['memory_gib']:.1f} GiB, "
              f"{meta['machine']['platform']}", ""]
    lines += ["## Binary size", "", "| | base | head | ratio |", "|---|---|---|---|",
              f"| weflow.exe | {fmt(doc['binary']['base'], MIB, 'MiB')} | {fmt(doc['binary']['head'], MIB, 'MiB')} "
              f"| {fmt_ratio(ratio(doc['binary']['head'], doc['binary']['base']))} |", ""]
    if doc["commands"]:
        lines += ["## Commands", ""]
        for key, label, unit, scale in METRICS + [("items_per_s", "throughput", "items/s", 1)]:
            lines += [f"### {label}" + (f" ({unit})" if unit else ""), "",
                      "| command | base | head | ratio |", "|---|---|---|---|"]
            for name, e in doc["commands"].items():
                b, h = e["median"]["base"].get(key), e["median"]["head"].get(key)
                if b is None and h is None:
                    continue
                cells = [fmt(b, scale), fmt(h, scale)]
                if key == "wall_ms":
                    # some commands take one of two times run to run: the range shows whether a ratio is noise
                    for i, v in enumerate(VERSIONS):
                        walls = [r["wall_ms"] for r in e["runs"][v] if r["exit_code"] == 0]
                        if walls:
                            cells[i] += f" ({fmt(min(walls))}–{fmt(max(walls))})"
                lines.append(f"| {name} | {cells[0]} | {cells[1]} | {fmt_ratio(ratio(h, b))} |")
            lines.append("")
        failed = [(n, v, e["median"][v]["failed_runs"]) for n, e in doc["commands"].items() for v in VERSIONS
                  if e["median"][v]["failed_runs"]]
        if failed:
            lines += ["**Failed runs** (see `logs/`): " + ", ".join(f"{n} ({v}: {k})" for n, v, k in failed), ""]
    if doc.get("load"):
        lines += ["## Sustained load on `weflow serve`", "",
                  "| | base | head | ratio |", "|---|---|---|---|"]
        rows = [("requests/s", lambda d: d["requests_per_s"], 1, ""),
                ("errors", lambda d: d["errors"], 1, ""),
                ("latency p50 (ms)", lambda d: d["latency_ms"]["p50"], 1, ""),
                ("latency p95 (ms)", lambda d: d["latency_ms"]["p95"], 1, ""),
                ("latency p99 (ms)", lambda d: d["latency_ms"]["p99"], 1, ""),
                ("start to listen (ms)", lambda d: d["started_ms"], 1, ""),
                *[(f"working set, median of quarter {q + 1} (MiB)",
                   lambda d, q=q: d["working_set_mib"]["quarter_medians"][q], 1, "") for q in range(4)],
                ("growth, quarter 2 to 4 (MiB)", lambda d: d["working_set_mib"]["growth"], 1, ""),
                ("peak working set (MiB)", lambda d: d["working_set_mib"]["peak"], 1, ""),
                ("peak threads", lambda d: d["peak_threads"], 1, ""),
                ("CPU (ms)", lambda d: d["user_ms"] + d["kernel_ms"], 1, "")]
        for label, pick, scale, unit in rows:
            b, h = pick(doc["load"]["base"]), pick(doc["load"]["head"])
            # a growth near zero makes a ratio meaningless
            r = None if label.startswith("growth") else ratio(h, b)
            lines.append(f"| {label} | {fmt(b, scale, unit)} | {fmt(h, scale, unit)} | {fmt_ratio(r)} |")
        lines += ["", "| route (requests, p50 / p95 ms) | base | head | p50 ratio |", "|---|---|---|---|"]
        rb, rh = doc["load"]["base"]["by_route"], doc["load"]["head"]["by_route"]
        for route in sorted(set(rb) | set(rh)):
            b, h = rb.get(route), rh.get(route)
            cell = lambda d: f"{d['n']}, {fmt(d['p50'])} / {fmt(d['p95'])}" if d else "-"
            lines.append(f"| {route} | {cell(b)} | {cell(h)} | "
                         f"{fmt_ratio(ratio(h and h['p50'], b and b['p50']))} |")
        lines.append("")
    sb = doc.get("serve_bench")
    if sb:
        lines += ["## serve_bench (median ms per route)", "",
                  f"exit code: base {sb['base']['exit_code']}, head {sb['head']['exit_code']} "
                  "(non-zero: a check on its synthetic account failed, see `logs/serve-bench-*.txt`)", "",
                  "| route | base | head | ratio |", "|---|---|---|---|"]
        rb = (sb["base"]["results"] or {}).get("results", {})
        rh = (sb["head"]["results"] or {}).get("results", {})
        for route in sorted(set(rb) | set(rh)):
            b, h = (rb.get(route) or {}).get("median"), (rh.get(route) or {}).get("median")
            lines.append(f"| {route} | {fmt(b)} | {fmt(h)} | {fmt_ratio(ratio(h, b))} |")
        lines.append("")
    return "\n".join(lines)


def svg_curves(series, width=640, height=180):
    """A line per (label, colour, [(t, bytes)]): working set over time."""
    points = [p for _, _, s in series for p in s]
    if not points:
        return ""
    tmax = max(p[0] for p in points) or 1
    ymax = max(p[1] for p in points) or 1
    pad = 40
    body = []
    for label, colour, s in series:
        pts = " ".join(f"{pad + p[0] / tmax * (width - pad - 8):.1f},{height - 20 - p[1] / ymax * (height - 30):.1f}"
                       for p in s)
        body.append(f'<polyline fill="none" stroke="{colour}" stroke-width="1.5" points="{pts}"><title>{label}'
                    f'</title></polyline>')
    axis = (f'<text x="2" y="14" font-size="11">{ymax / MIB:.0f} MiB</text>'
            f'<text x="{width - 60}" y="{height - 4}" font-size="11">{tmax:.1f} s</text>'
            f'<line x1="{pad}" y1="{height - 20}" x2="{width - 8}" y2="{height - 20}" stroke="#999"/>')
    return f'<svg viewBox="0 0 {width} {height}" width="100%" style="max-width:{width}px">{axis}{"".join(body)}</svg>'


def report_html(doc, markdown):
    parts = ["<!doctype html><meta charset='utf-8'><title>weflow benchmark</title>",
             "<style>body{font:14px system-ui;margin:16px;max-width:900px}pre{white-space:pre-wrap}"
             ".b{color:#2563eb}.h{color:#dc2626}</style>",
             "<h1>weflow benchmark</h1><p><span class=b>■ base</span> <span class=h>■ head</span>: "
             "working set of the median run</p>"]
    for name, e in doc["commands"].items():
        series = []
        for v, colour in (("base", "#2563eb"), ("head", "#dc2626")):
            ok = sorted((r for r in e["runs"][v] if r["exit_code"] == 0), key=lambda r: r["wall_ms"])
            if ok:
                series.append((v, colour, [(t, rss) for t, rss, _ in ok[len(ok) // 2]["samples"]]))
        parts.append(f"<h2>{html.escape(name)}</h2>{svg_curves(series)}")
    if doc.get("load"):
        series = [(v, c, [(t, rss) for t, rss, _ in doc["load"][v]["samples"]])
                  for v, c in (("base", "#2563eb"), ("head", "#dc2626"))]
        parts.append(f"<h2>weflow serve under sustained load</h2>{svg_curves(series)}")
    parts.append(f"<h2>Tables</h2><pre>{html.escape(markdown)}</pre>")
    return "\n".join(parts)


def binary_info(exe):
    out = subprocess.run([exe, "--version"], capture_output=True, text=True).stdout.strip()
    return {"path": os.path.abspath(exe), "version": out, "bytes": os.path.getsize(exe)}


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--base", required=True)
    p.add_argument("--head", required=True)
    p.add_argument("--base-commit", default="")
    p.add_argument("--head-commit", default="")
    p.add_argument("--account", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--runs", type=int, default=5)
    p.add_argument("--load-seconds", type=int, default=90)
    p.add_argument("--load-workers", type=int, default=4)
    p.add_argument("--serve-bench")
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
            "base": {**binary_info(exes["base"]), "commit": args.base_commit},
            "head": {**binary_info(exes["head"]), "commit": args.head_commit},
            "runs": args.runs,
            "account": {k: manifest[k] for k in ("seed", "messages", "messages_in_wal", "by_type", "conversations")},
            "machine": {"cpus": os.cpu_count(), "memory_gib": psutil.virtual_memory().total / 1024 ** 3,
                        "platform": f"{sys.platform} {os.environ.get('ImageOS', '')}".strip(),
                        "runner": os.environ.get("RUNNER_NAME")},
        },
        "binary": {v: os.path.getsize(exes[v]) for v in VERSIONS},
    }
    doc["commands"] = bench_commands(args, exes, homes, manifest, out)
    if wanted("load") and args.load_seconds > 0:
        print("== sustained load", flush=True)
        doc["load"] = {v: sustained_load(exes[v], template, manifest, args.load_seconds, args.load_workers, out, v)
                       for v in VERSIONS}
    if args.serve_bench and wanted("serve_bench"):
        print("== serve_bench", flush=True)
        doc["serve_bench"] = {v: run_serve_bench(args.serve_bench, exes[v], homes[v], args.runs, out, v)
                              for v in VERSIONS}
    shutil.rmtree(os.path.join(out, "work"), ignore_errors=True)

    markdown = summary_markdown(doc)
    with open(os.path.join(out, "results.json"), "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, indent=1)
    with open(os.path.join(out, "summary.md"), "w", encoding="utf-8") as f:
        f.write(markdown)
    with open(os.path.join(out, "report.html"), "w", encoding="utf-8") as f:
        f.write(report_html(doc, markdown))
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write(markdown + "\n")
    print(markdown)


if __name__ == "__main__":
    main()
