// The comparison of two benchmark results (#601, #602): the `result.json` of two commits measured by
// `.github/scripts/bench.py` in the same environment. Used by the viewer on GitHub Pages (index.html, in the browser)
// and by cli.mjs (Node, for the run's page and the release notes), so it has no dependency and touches no DOM.
//
// The report is for people, so it is in Chinese like the repository's PRs and issues; command and route names, units
// and the field names of result.json stay as they are. It is built as blocks: {h, level, text}, {p, text},
// {ul, items}, {table, header, rows}, then written as Markdown or HTML. Text may hold `code` and **bold**.

export const MIB = 1024 * 1024;
export const SIDES = ['base', 'head'];
export const COLOURS = { base: '#2563eb', head: '#dc2626' };

const CPU_NOTE = '进程及其子进程的合计（Job Object）。Windows 的 CPU 时间精度是 15.6 ms，1 秒左右的命令差几十毫秒属于噪声。';
const IO_NOTE = '逻辑 I/O，包括命中系统文件缓存的部分；Windows 不按进程统计实际的磁盘 I/O。';
const ALLOC_NOTE = '用带 `alloc-stats` feature 编译的二进制另外运行一次得到（#513），不计时；没有这个 feature 的版本这一项为空。';
// [key, label, unit, scale, note]
const METRICS = [
  ['wall_ms', '耗时', 'ms', 1, '从启动到退出。括号里是最快和最慢的一次。'],
  ['first_batch_ms', '第一批结果', 'ms', 1, '从启动到第一个有进度的事件（`--progress`）。不输出进度的命令没有这一项。'],
  ['user_ms', '用户态 CPU', 'ms', 1, CPU_NOTE],
  ['kernel_ms', '内核态 CPU', 'ms', 1, CPU_NOTE],
  ['peak_working_set', '峰值工作集', 'MiB', MIB, '主进程占用物理内存的峰值。'],
  ['peak_job_commit', '峰值提交内存', 'MiB', MIB, '进程及其子进程提交内存的峰值，包括还没有占用物理内存的部分。'],
  ['peak_threads', '峰值线程数', '', 1, '每 20 ms 采样一次主进程得到的最大值。'],
  ['read_bytes', '读取', 'MiB', MIB, IO_NOTE],
  ['write_bytes', '写入', 'MiB', MIB, IO_NOTE],
  ['page_faults', '页错误', '', 1, '进程及其子进程，包括软页错误。'],
  ['items_per_s', '吞吐量', '消息/秒', 1, '命令处理的消息数除以耗时。这是唯一一个越高越好的指标。'],
  ['allocations', '分配次数', '', 1, '分配和重新分配的次数。' + ALLOC_NOTE],
  ['allocated_bytes', '累计分配', 'MiB', MIB, '每次分配的大小之和，重新分配只计增长的部分。' + ALLOC_NOTE],
  ['peak_live_bytes', '峰值存活堆', 'MiB', MIB, '同一时刻仍未释放的分配之和的最大值。' + ALLOC_NOTE],
];
// below these differences (in the metric's own unit) a ratio is not marked: the timer's resolution, the noise of a
// run, or a few MiB of a working set
const NOISE = {
  wall_ms: 20, first_batch_ms: 20, user_ms: 50, kernel_ms: 50, peak_working_set: 5 * MIB, peak_job_commit: 5 * MIB,
  peak_threads: 1, read_bytes: MIB, write_bytes: MIB, allocated_bytes: MIB, peak_live_bytes: 5 * MIB,
};
const NOISE_MS = 5;
// an anchor that moved more than this between the two runs makes their comparison doubtful
export const DRIFT_LIMIT = 0.05;
const RATIO = 'head / base';
const LOAD_ROUTES = {
  'messages-any-page': '`/api/chat/messages`，大群任意一页',
  'messages-first-page': '`/api/chat/messages`，其他会话第一页',
  sessions: '`/api/chat/sessions`',
  search: '`/api/chat/search`，全部会话',
  'date-counts': '`/api/chat/date-counts`',
  contacts: '`/api/chat/contacts`',
};

const isNum = v => typeof v === 'number' && Number.isFinite(v);
export const ratio = (h, b) => (isNum(h) && isNum(b) && b ? h / b : null);
const diff = (h, b) => (isNum(h) && isNum(b) ? h - b : null);
const median = xs => {
  const s = xs.filter(isNum).sort((a, b) => a - b);
  if (!s.length) return null;
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

export function fmt(v, scale = 1, unit = '') {
  if (!isNum(v)) return '-';
  if (Number.isInteger(v) && scale === 1) return `${v.toLocaleString('en-US')} ${unit}`.trim();
  const x = v / scale;
  const small = Math.abs(x) < 10 || (Math.abs(x) < 100 && scale !== 1);
  const text = x.toLocaleString('en-US', { minimumFractionDigits: small ? 1 : 0, maximumFractionDigits: small ? 1 : 0 });
  return `${text} ${unit}`.trim();
}

/** ▲ or ▼ when `r` is past 10 %, unless the difference is within `noise`. */
export function mark(r, d = null, noise = 0) {
  if (r === null || (d !== null && Math.abs(d) <= noise)) return '';
  return r > 1.1 ? '▲' : r < 0.9 ? '▼' : '';
}

function fmtRatio(r, d = null, noise = 0) {
  if (r === null) return '-';
  const m = mark(r, d, noise);
  return `${r.toFixed(2)}×${m ? ' ' + m : ''}`;
}

/** Why the two results cannot be compared, or null when they can. */
export function incomparable(base, head) {
  if (!base || !head) return '缺少结果';
  if (base.format !== 1 || head.format !== 1) return '结果格式不认识';
  if (base.env?.key !== head.env?.key) return `两份结果的环境不同（${base.env?.key} 与 ${head.env?.key}），不能放在一起比`;
  return null;
}

// ---- the anchor ----

const anchorCommand = (r, name) => r.anchor?.commands?.[name] ?? null;

/** How much the anchor's median wall time of a command moved from the base run to the head run (null: unknown). */
export function commandDrift(base, head, name) {
  const a = anchorCommand(base, name), b = anchorCommand(head, name);
  if (!a || !b || a.failed_runs || b.failed_runs) return null;
  return ratio(b.median?.wall_ms, a.median?.wall_ms);
}

function driftRows(base, head) {
  const rows = [];
  for (const name of Object.keys(base.commands ?? {})) {
    if (!(name in (head.commands ?? {}))) continue;
    const a = anchorCommand(base, name)?.median?.wall_ms, b = anchorCommand(head, name)?.median?.wall_ms;
    rows.push({ label: `\`${name}\` 耗时 (ms)`, a, b, drift: commandDrift(base, head, name), noise: NOISE.wall_ms });
  }
  const la = base.anchor?.load, lb = head.anchor?.load;
  if (la && lb) {
    rows.push({ label: '持续负载：每秒请求数', a: la.requests_per_s, b: lb.requests_per_s, drift: ratio(lb.requests_per_s, la.requests_per_s), noise: 0 });
    rows.push({ label: '持续负载：延迟 p50 (ms)', a: la.latency_ms?.p50, b: lb.latency_ms?.p50, drift: ratio(lb.latency_ms?.p50, la.latency_ms?.p50), noise: NOISE_MS });
  }
  const sa = base.anchor?.serve_bench?.results?.results, sb = head.anchor?.serve_bench?.results?.results;
  if (sa && sb) {
    const each = Object.keys(sa).filter(k => k in sb).map(k => ratio(sb[k].median, sa[k].median)).filter(isNum);
    if (each.length) rows.push({ label: `serve_bench：${each.length} 个路由漂移的中位数`, a: null, b: null, drift: median(each), noise: 0 });
  }
  return rows;
}

/** The median drift of the commands' wall times, the overall word on the two runs. */
export function overallDrift(base, head) {
  return median(Object.keys(base.commands ?? {}).map(n => commandDrift(base, head, n)));
}

// ---- the report ----

function sideLine(r, side) {
  const commit = (r.commit ?? '').slice(0, 10);
  const named = r.ref && !commit.startsWith(r.ref) ? `\`${r.ref}\`（\`${commit}\`）` : `\`${commit || r.ref}\``;
  const version = (r.version || '版本未知').split(' (')[0];
  return `${side}：${named}，${version}，测于 ${r.measured_at ?? '-'}${r.os ? `，${r.os}` : ''}`;
}

function quietText(q) {
  if (!q || !q.samples) return '没有记录';
  return `其他进程占用 CPU 中位数 ${q.p50_pct}%，p95 ${q.p95_pct}%，超过 10% 的时间占 ${Math.round(q.busy_share * 100)}%`;
}

const commandNames = (base, head) => Object.keys(base.commands ?? {}).filter(n => n in (head.commands ?? {}));

function spans(e) {
  const walls = (e?.runs ?? []).filter(r => r.exit_code === 0).map(r => r.wall_ms);
  return walls.length ? [Math.min(...walls), Math.max(...walls)] : null;
}

const overlap = (a, b) => a && b && a[0] <= b[1] && b[0] <= a[1];

export function reportBlocks(base, head) {
  const env = head.env;
  const m = env.machine, acc = env.account;
  const drift = overallDrift(base, head);
  const blocks = [
    { h: 1, text: '基准测试：base 与 head' },
    {
      ul: [
        sideLine(base, 'base'),
        sideLine(head, 'head'),
        `环境 \`${env.key}\`：${m.cpu}，${m.cpus} 个逻辑 CPU，${m.memory_gib} GiB 内存；测法版本 ${env.method}`,
        `基准账号：种子 ${acc.seed}，${fmt(acc.messages)} 条消息，${acc.conversations} 个会话（manifest \`${acc.manifest}\`，`
          + (acc.profile ? `profile \`${acc.profile.hash}\`，测于 ${acc.profile.measured}）` : '默认 profile）'),
        `两个提交分开测量，各自先运行一次不计时，再和锚点 \`${(head.anchor?.commit ?? '').slice(0, 10)}\` 交替各运行 ${base.runs}／${head.runs} 次，取成功运行的中位数`,
        `比值是 head / base：▲ 表示增加超过 10%，▼ 表示减少超过 10%；差值小于噪声下限时不标（耗时 20 ms，CPU 50 ms，内存 5 MiB，路由延迟 5 ms）。除了吞吐量和每秒请求数，其余指标都是越低越好`,
      ],
    },
    {
      p: '**怎么看**：同一份代码对比时，耗时比值一般在 0.98–1.02 之间，个别命令（如 json 导出）每次在两个耗时之间跳。'
        + '比值超过约 10%、两边最快到最慢的范围不重叠、锚点也没有明显漂移，才算真正的变化。',
    },
    { h: 2, text: '两次测量的环境' },
    {
      p: `锚点是这个环境里每次都一起测的同一个提交：它在两次测量之间变了多少，机器就漂了多少。`
        + (drift === null ? '这两份结果没有可比的锚点数据。'
          : Math.abs(drift - 1) > DRIFT_LIMIT ? `**锚点耗时整体变了 ${((drift - 1) * 100).toFixed(1)}%**，超过 ${DRIFT_LIMIT * 100}%，下面的比值要打折扣看；耗时表的「校正后」一列除掉了每条命令的锚点漂移。`
            : `锚点耗时整体变了 ${((drift - 1) * 100).toFixed(1)}%，在 ${DRIFT_LIMIT * 100}% 以内。`),
    },
    {
      ul: [`base 测量时：${quietText(base.quiet)}`, `head 测量时：${quietText(head.quiet)}`,
        ...(base.os && head.os && base.os !== head.os
          ? [`两次测量的 Windows 版本不同（${base.os} → ${head.os}）：系统更新带来的变化算在锚点漂移里`] : [])],
    },
  ];
  const dr = driftRows(base, head);
  if (dr.length) {
    blocks.push({
      table: ['锚点', 'base 那次', 'head 那次', '漂移'],
      rows: dr.map(r => [r.label, fmt(r.a), fmt(r.b), r.drift === null ? '-' : `${r.drift.toFixed(3)}×${Math.abs(r.drift - 1) > DRIFT_LIMIT ? ' ⚠' : ''}`]),
    });
  }
  blocks.push(
    { h: 2, text: '二进制体积' },
    { table: ['', 'base', 'head', RATIO], rows: [['`weflow.exe`', fmt(base.binary_bytes, MIB, 'MiB'), fmt(head.binary_bytes, MIB, 'MiB'), fmtRatio(ratio(head.binary_bytes, base.binary_bytes))]] },
  );

  const names = commandNames(base, head);
  const onlyOne = [...Object.keys(base.commands ?? {}), ...Object.keys(head.commands ?? {})].filter(n => !names.includes(n));
  if (names.length || onlyOne.length) blocks.push({ h: 2, text: '命令' });
  if (onlyOne.length) blocks.push({ p: '只有一边测过的命令不比：' + [...new Set(onlyOne)].map(n => `\`${n}\``).join('，') });
  const failed = [];
  for (const n of names) for (const [side, r] of [['base', base], ['head', head]]) {
    const k = r.commands[n].failed_runs;
    if (k) failed.push(`\`${n}\`（${side} ${k} 次）`);
  }
  if (failed.length) blocks.push({ p: '**有运行失败**（不计入中位数）：' + failed.join('，') });
  for (const [key, label, unit, scale, note] of METRICS) {
    const rows = [];
    for (const n of names) {
      const eb = base.commands[n], eh = head.commands[n];
      const b = eb.median?.[key] ?? null, h = eh.median?.[key] ?? null;
      if (b === null && h === null) continue;
      const cells = [fmt(b, scale), fmt(h, scale)];
      let ratioCell = fmtRatio(ratio(h, b), diff(h, b), NOISE[key] ?? 0);
      if (key === 'wall_ms') {
        const sb = spans(eb), sh = spans(eh);
        if (sb) cells[0] += `（${fmt(sb[0])}–${fmt(sb[1])}）`;
        if (sh) cells[1] += `（${fmt(sh[0])}–${fmt(sh[1])}）`;
        // a ratio past 10 % whose ranges overlap is likely a command with two run times, not a change
        if (/[▲▼]$/.test(ratioCell) && overlap(sb, sh)) ratioCell += '，范围重叠';
        const d = commandDrift(base, head, n), r = ratio(h, b);
        const corrected = d && r !== null ? r / d : null;
        rows.push([`\`${n}\``, ...cells, ratioCell, d === null ? '-' : d.toFixed(3) + '×', corrected === null ? '-' : fmtRatio(corrected, diff(h, isNum(b) ? b * d : null), NOISE.wall_ms)]);
        continue;
      }
      rows.push([`\`${n}\``, ...cells, ratioCell]);
    }
    if (!rows.length) continue;
    const header = key === 'wall_ms' ? ['命令', 'base', 'head', RATIO, '锚点漂移', '校正后'] : ['命令', 'base', 'head', RATIO];
    blocks.push({ h: 3, text: unit ? `${label}（${unit}）` : label }, { p: note }, { table: header, rows });
  }

  blocks.push(...loadBlocks(base, head), ...serveBenchBlocks(base, head));
  return blocks;
}

function loadBlocks(base, head) {
  if (!base.load && !base.load_failed && !head.load && !head.load_failed) return [];
  const out = [{ h: 2, text: '`weflow serve` 持续负载' }];
  const failed = [['base', base], ['head', head]].filter(([, r]) => r.load_failed || !r.load);
  if (failed.length) {
    out.push({ p: '没有完成或没有测：' + failed.map(([s, r]) => `${s}：${(r.load_failed ?? '没有测').replace(/\n/g, ' ').slice(0, 300)}`).join('；') });
    return out;
  }
  const lb = base.load, lh = head.load;
  out.push({
    p: `每次测量启动一个 \`weflow serve\`，用 ${lh.workers} 个并发连续请求 ${lh.seconds} 秒，混合消息分页、会话列表、全局搜索、日期统计和联系人。`
      + '内存按时间分成四段，各取中位数：第 2 段（缓存已经热了）到第 4 段持续上升，才说明内存不稳定。',
  });
  // [label, pick, noise]; a null noise: no ratio, as for a growth or an error count near zero
  const rows = [
    ['每秒请求数', d => d.requests_per_s, 0],
    ['错误数', d => d.errors, null],
    ['延迟 p50 (ms)', d => d.latency_ms.p50, NOISE_MS],
    ['延迟 p95 (ms)', d => d.latency_ms.p95, NOISE_MS],
    ['延迟 p99 (ms)', d => d.latency_ms.p99, NOISE_MS],
    ['启动到开始监听 (ms)', d => d.started_ms, NOISE_MS],
    ...[0, 1, 2, 3].map(q => [`工作集，第 ${q + 1} 段中位数 (MiB)`, d => d.working_set_mib.quarter_medians[q], 5]),
    ['第 2 段到第 4 段的增长 (MiB)', d => d.working_set_mib.growth, null],
    ['峰值工作集 (MiB)', d => d.working_set_mib.peak, 5],
    ['峰值线程数', d => d.peak_threads, 1],
    ['CPU 时间 (ms)', d => d.user_ms + d.kernel_ms, 50],
  ];
  out.push({
    table: ['', 'base', 'head', RATIO],
    rows: rows.map(([label, pick, noise]) => {
      const b = pick(lb) ?? null, h = pick(lh) ?? null;
      return [label, fmt(b), fmt(h), noise === null ? '-' : fmtRatio(ratio(h, b), diff(h, b), noise)];
    }),
  });
  const rb = lb.by_route ?? {}, rh = lh.by_route ?? {};
  const cell = d => (d ? `${d.n} 次，${fmt(d.p50)} / ${fmt(d.p95)}` : '-');
  const keys = [...Object.keys(LOAD_ROUTES).filter(k => k in rb || k in rh),
    ...[...new Set([...Object.keys(rb), ...Object.keys(rh)])].filter(k => !(k in LOAD_ROUTES)).sort()];
  out.push({ p: '各路由：请求次数，延迟 p50 / p95（ms）。' }, {
    table: ['路由', 'base', 'head', `p50 ${RATIO}`],
    rows: keys.map(k => {
      const b = rb[k]?.p50 ?? null, h = rh[k]?.p50 ?? null;
      return [LOAD_ROUTES[k] ?? `\`${k}\``, cell(rb[k]), cell(rh[k]), fmtRatio(ratio(h, b), diff(h, b), NOISE_MS)];
    }),
  });
  return out;
}

/** serve_bench notes the answers that were not 200 as `status {404: 5}`. */
function status(row) {
  const note = row?.note ?? '';
  const part = note.split(';').map(s => s.trim()).find(s => s.startsWith('status {'));
  return part ? part.replace(/^status /, '') : null;
}

function serveBenchBlocks(base, head) {
  const sb = base.serve_bench, sh = head.serve_bench;
  if (!sb && !sh) return [];
  const code = s => (s ? s.exit_code : '没有测');
  const out = [
    { h: 2, text: 'serve_bench' },
    {
      p: '各路由响应时间的中位数（ms），由 `serve_bench` 在同一个基准账号上测得。路由名是它自己的；`[api]` 是进程内的 `/api/` 路由，`[sub]` 是启动子进程的路由。'
        + `退出码：base ${code(sb)}，head ${code(sh)}`
        + (sb?.exit_code === 0 && sh?.exit_code === 0 ? '。' : '。非 0 表示它在自己的合成账号上有检查没通过（旧版本缺少某个路由时会这样），计时照常保留。')
        + '某一边不是每次都返回 200 时（例如旧版本还没有这个 `/api/` 路由），格子里注明返回的状态，不计算比值。',
    },
  ];
  const rb = sb?.results?.results ?? {}, rh = sh?.results?.results ?? {};
  const rows = [...new Set([...Object.keys(rb), ...Object.keys(rh)])].sort().map(route => {
    const b = rb[route]?.median ?? null, h = rh[route]?.median ?? null;
    const xb = status(rb[route]), xh = status(rh[route]);
    return [`\`${route}\``, fmt(b) + (xb ? `（状态 ${xb}）` : ''), fmt(h) + (xh ? `（状态 ${xh}）` : ''),
      xb || xh ? '-' : fmtRatio(ratio(h, b), diff(h, b), NOISE_MS)];
  });
  if (rows.length) out.push({ table: ['路由', 'base', 'head', RATIO], rows });
  return out;
}

/** A few lines for the release notes (#516): what changed past the noise and the drift, the rest in the report. */
export function highlights(base, head) {
  const out = [];
  out.push(`二进制体积：${fmt(base.binary_bytes, MIB, 'MiB')} → ${fmt(head.binary_bytes, MIB, 'MiB')}（${ratio(head.binary_bytes, base.binary_bytes).toFixed(2)}×）`);
  const drift = overallDrift(base, head);
  if (drift !== null && Math.abs(drift - 1) > DRIFT_LIMIT) {
    out.push(`两次测量之间机器漂移了 ${((drift - 1) * 100).toFixed(1)}%（锚点耗时），下面的耗时已按锚点校正，仍要谨慎看`);
  }
  let names = commandNames(base, head);
  for (const [side, r] of [['base', base], ['head', head]]) {
    const f = names.filter(n => r.commands[n].failed_runs);
    if (f.length && f.length === names.length) out.push(`${side} 的命令全部运行失败，见报告`);
    else if (f.length) out.push(`${side} 有运行失败的命令：` + f.map(n => `\`${n}\``).join('，'));
  }
  // a failed run ends early: only the commands that ran through on both sides are compared
  names = names.filter(n => !base.commands[n].failed_runs && !head.commands[n].failed_runs);
  if (names.length) {
    const changed = [], drifted = [];
    for (const n of names) {
      const mb = base.commands[n].median?.wall_ms, mh = head.commands[n].median?.wall_ms;
      const d = commandDrift(base, head, n);
      if (d !== null && Math.abs(d - 1) > 2 * DRIFT_LIMIT) { drifted.push(`\`${n}\``); continue; }
      const r = ratio(mh, mb);
      const corrected = r === null ? null : r / (d ?? 1);
      if (!mark(corrected, diff(mh, mb), NOISE.wall_ms)) continue;
      if (overlap(spans(base.commands[n]), spans(head.commands[n]))) continue; // a command with two run times
      changed.push(`\`${n}\` ${fmt(mb)} → ${fmt(mh)} ms（${corrected.toFixed(2)}×${d !== null ? '，已按锚点校正' : ''}）`);
    }
    const rest = names.length - changed.length - drifted.length;
    if (changed.length) out.push(`命令耗时有变化：${changed.join('；')}；其余 ${rest} 条在噪声范围内`);
    else out.push(`命令耗时：${rest} 条命令都在噪声范围内`);
    if (drifted.length) out.push(`锚点漂移超过 ${2 * DRIFT_LIMIT * 100}%、不下结论的命令：${drifted.join('，')}`);
  }
  if (base.load_failed || head.load_failed) out.push('`weflow serve` 持续负载没有完成，见报告');
  else if (base.load && head.load) {
    const lb = base.load, lh = head.load;
    let line = `\`weflow serve\` 持续负载：每秒请求数 ${fmt(lb.requests_per_s)} → ${fmt(lh.requests_per_s)}，`
      + `延迟 p50 ${fmt(lb.latency_ms.p50)} → ${fmt(lh.latency_ms.p50)} ms，`
      + `工作集 ${fmt(lb.working_set_mib.quarter_medians[3])} → ${fmt(lh.working_set_mib.quarter_medians[3])} MiB`;
    const growth = lh.working_set_mib.growth;
    if (isNum(growth) && growth > 20) line += `；head 的内存在后半段上升了 ${fmt(growth)} MiB`;
    out.push(line);
  }
  if (base.serve_bench && head.serve_bench) {
    const rb = base.serve_bench.results?.results ?? {}, rh = head.serve_bench.results?.results ?? {};
    let ups = 0, downs = 0, compared = 0;
    for (const route of Object.keys(rb).filter(k => k in rh)) {
      const notes = (rb[route].note ?? '') + (rh[route].note ?? '');
      const mb = rb[route].median, mh = rh[route].median;
      if (notes.includes('status {') || !isNum(mb) || !isNum(mh)) continue;
      compared += 1;
      const m = mark(ratio(mh, mb), mh - mb, NOISE_MS);
      ups += m === '▲'; downs += m === '▼';
    }
    if (!compared) out.push('serve_bench：没有两边都能计时的路由，见报告');
    else if (ups || downs) out.push(`serve_bench：${downs} 个路由快了 10% 以上，${ups} 个慢了 10% 以上`);
    else out.push(`serve_bench：${compared} 个路由都在噪声范围内`);
  }
  return out;
}

// ---- writing it out ----

export function toMarkdown(blocks) {
  const row = cells => '| ' + cells.map(c => String(c).replace(/\|/g, '\\|')).join(' | ') + ' |';
  return blocks.map(b => {
    if ('h' in b) return '#'.repeat(b.h) + ' ' + b.text;
    if ('p' in b) return b.p;
    if ('ul' in b) return b.ul.map(i => `- ${i}`).join('\n');
    return [row(b.table), '|' + '---|'.repeat(b.table.length), ...b.rows.map(row)].join('\n');
  }).join('\n\n') + '\n';
}

const escape = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** `code` and **bold** of a report text, the rest escaped. */
export function inlineHtml(text) {
  const code = escape(text).split('`').map((p, i) => (i % 2 ? `<code>${p}</code>` : p)).join('');
  return code.split('**').map((p, i) => (i % 2 ? `<strong>${p}</strong>` : p)).join('');
}

export function toHtml(blocks) {
  return blocks.map(b => {
    if ('h' in b) return `<h${b.h}>${inlineHtml(b.text)}</h${b.h}>`;
    if ('p' in b) return `<p>${inlineHtml(b.p)}</p>`;
    if ('ul' in b) return '<ul>' + b.ul.map(i => `<li>${inlineHtml(i)}</li>`).join('') + '</ul>';
    const head = b.table.map(c => `<th>${inlineHtml(c)}</th>`).join('');
    const body = b.rows.map(r => '<tr>' + r.map(c => `<td>${inlineHtml(c)}</td>`).join('') + '</tr>').join('');
    return `<div class="scroll"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
  }).join('\n');
}

export const CURVES_NOTE = '主进程的工作集，每 20 ms 采样一次（持续负载每 0.5 s 一次），每条曲线最多保留 300 个点（每段取峰值）。命令取耗时居中的那次运行。';

/** [title, [[side, [[seconds, MiB]]]]] of each chart: each command's median run, then the sustained load. */
export function memorySeries(base, head) {
  const charts = [];
  for (const n of commandNames(base, head)) {
    charts.push([`\`${n}\``, SIDES.map((s, i) => [s, [base, head][i].commands[n].curve ?? []]).filter(([, c]) => c.length)]);
  }
  if (base.load && head.load) charts.push(['`weflow serve` 持续负载', [['base', base.load.curve ?? []], ['head', head.load.curve ?? []]]]);
  return charts;
}

/** A step of 1, 2 or 5 times a power of ten that cuts `span` into about `ticks` parts. */
function niceStep(span, ticks = 4) {
  const raw = span / ticks;
  const power = 10 ** Math.floor(Math.log10(raw));
  return [1, 2, 5, 10].map(m => m * power).find(s => s >= raw);
}

/** Working set over time, a line per side, from 0 on both axes; the lines take the classes `base` and `head`. */
export function svgCurves(series, width = 720, height = 200) {
  const points = series.flatMap(([, s]) => s);
  if (!points.length) return '<p>没有采样。</p>';
  const tMax0 = Math.max(...points.map(p => p[0])) || 1, yMax0 = Math.max(...points.map(p => p[1])) || 1;
  const tstep = niceStep(tMax0), ystep = niceStep(yMax0);
  const tmax = Math.ceil(tMax0 / tstep) * tstep, ymax = Math.ceil(yMax0 / ystep) * ystep;
  const [left, bottom, top, right] = [64, 24, 10, 24];
  const x = t => left + (t / tmax) * (width - left - right);
  const y = v => height - bottom - (v / ymax) * (height - bottom - top);
  const label = v => String(+v.toPrecision(6));
  const grid = [];
  for (let k = 0; k <= Math.round(ymax / ystep); k++) {
    const v = k * ystep;
    grid.push(`<line x1="${left}" y1="${y(v).toFixed(1)}" x2="${width - right}" y2="${y(v).toFixed(1)}" class="grid" />`
      + `<text x="${left - 6}" y="${(y(v) + 4).toFixed(1)}" text-anchor="end">${label(v)} MiB</text>`);
  }
  for (let k = 0; k <= Math.round(tmax / tstep); k++) {
    const t = k * tstep;
    grid.push(`<text x="${x(t).toFixed(1)}" y="${height - 6}" text-anchor="middle">${label(t)} s</text>`);
  }
  const lines = series.map(([side, s]) => `<polyline fill="none" class="${side}" stroke-width="1.5" points="${s.map(([t, v]) => `${x(t).toFixed(1)},${y(v).toFixed(1)}`).join(' ')}"><title>${side}</title></polyline>`);
  return `<svg viewBox="0 0 ${width} ${height}" role="img" aria-label="工作集随时间变化">${grid.join('')}${lines.join('')}</svg>`;
}

/**
 * The curves of `svgCurves` as a Mermaid xychart for the Actions job summary, which drops inline SVG. A linear x-axis
 * spreads each line evenly over the whole axis, so every line gets the same `points` columns: the peak of the samples
 * falling in each column, the last value carried over a column with no sample, and 0 after the side's last sample.
 */
export function mermaidCurves(series, points = 60) {
  series = series.filter(([, s]) => s.length);
  if (!series.length) return '没有采样。';
  const all = series.flatMap(([, s]) => s);
  const tmax = Math.max(...all.map(p => p[0])) || 1;
  const yMax0 = Math.max(...all.map(p => p[1])) || 1;
  const ystep = niceStep(yMax0), ymax = Math.ceil(yMax0 / ystep) * ystep;
  const lines = series.map(([, s]) => {
    const columns = new Array(points).fill(null);
    for (const [t, v] of s) {
      const i = Math.round((t / tmax) * (points - 1));
      columns[i] = Math.max(columns[i] ?? 0, v);
    }
    const last = columns.reduce((acc, c, i) => (c !== null ? i : acc), 0);
    let value = columns.find(c => c !== null);
    const values = columns.map((c, i) => { value = c ?? value; return i <= last ? value : 0; });
    return '    line [' + values.map(v => v.toFixed(1)).join(', ') + ']';
  });
  const config = { xyChart: { width: 720, height: 260 }, themeVariables: { xyChart: { plotColorPalette: series.map(([s]) => COLOURS[s]).join(', ') } } };
  return ['```mermaid', `%%{init: ${JSON.stringify(config)}}%%`, 'xychart-beta', `    x-axis "秒" 0 --> ${+tmax.toPrecision(6)}`,
    `    y-axis "MiB" 0 --> ${+ymax.toPrecision(6)}`, ...lines, '```'].join('\n');
}

/** The 内存曲线 section of the Markdown report. */
export function markdownCurves(base, head) {
  const out = ['## 内存曲线', '', `${CURVES_NOTE}蓝线 base，红线 head；先结束的一边在退出后记为 0。`, ''];
  for (const [title, series] of memorySeries(base, head)) out.push(`### ${title}`, '', mermaidCurves(series), '');
  return out.join('\n');
}
