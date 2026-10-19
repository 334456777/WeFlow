// The command console of the settings (#217): any command of `/openapi.json`, found by name or words, filled in by a
// form made from its parameters, shown as the command line it runs, and run at once or as a job of the task centre.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { ArrowClockwise, Check, CircleNotch, MagnifyingGlass, Play, Prohibit, Terminal } from '@phosphor-icons/react';
import {
  commandLine, formProblems, getOpenApi, paramsOf, runRaw, shellLine, type ConsoleCommand, type Field, type RunAnswer,
} from '../api/console';
import { runJob } from '../jobs/useJobs';
import { Segmented } from '../Segmented';
import { ListSkeleton } from '../Skeleton';

type Values = Record<string, string | boolean>;
/** The answer shown is cut here, so a long list does not freeze the page. */
const SHOWN_CHARS = 200_000;

function initialValues(cmd: ConsoleCommand): Values {
  return Object.fromEntries(cmd.fields.map(f => [f.name, f.type === 'boolean' ? false : f.type === 'choice' && f.required ? f.options![0] : '']));
}

/** The commands whose name, summary or words hold every word of `query`. */
export function findCommands(commands: readonly ConsoleCommand[], query: string): ConsoleCommand[] {
  const words = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [...commands];
  return commands.filter(c => {
    const text = (c.command + ' ' + c.summary + ' ' + c.description).toLocaleLowerCase();
    return words.every(w => text.includes(w));
  });
}

function FieldInput({ field, value, onChange }: { field: Field; value: string | boolean; onChange(value: string | boolean): void }) {
  const id = 'console-field-' + field.name;
  const label = <label htmlFor={id} className="console-field-name"><code>{field.name}</code>{field.required && <span className="console-required" aria-label="必填">*</span>}</label>;
  let input;
  if (field.type === 'boolean') {
    input = <label className="toggle"><input id={id} type="checkbox" aria-label={field.name} checked={value === true} onChange={event => onChange(event.target.checked)} /><span /></label>;
  } else if (field.type === 'choice') {
    input = <select id={id} className="console-input" value={String(value)} onChange={event => onChange(event.target.value)}>
      {!field.required && <option value="">（默认{field.defaultValue ? '：' + field.defaultValue : ''}）</option>}
      {field.options!.map(o => <option key={o} value={o}>{o}</option>)}
    </select>;
  } else if (field.type === 'list') {
    input = <textarea id={id} className="console-input" rows={2} spellCheck={false} placeholder={field.integers ? '每行或用逗号分隔一个整数' : '每行或用逗号分隔一项'} value={String(value)} onChange={event => onChange(event.target.value)} />;
  } else {
    input = <input id={id} className="console-input" type="text" inputMode={field.type === 'string' ? undefined : 'numeric'} spellCheck={false}
      placeholder={field.defaultValue ? '默认 ' + field.defaultValue : field.type === 'integer' ? '整数' : ''} value={String(value)} onChange={event => onChange(event.target.value)} />;
  }
  return <div className={'console-field' + (field.type === 'boolean' ? ' inline' : '')}>{label}{input}{field.description && <small>{field.description}</small>}</div>;
}

function Answer({ answer }: { answer: RunAnswer }) {
  const ok = answer.status >= 200 && answer.status < 300;
  const text = typeof answer.body === 'string' ? answer.body : JSON.stringify(answer.body, null, 2);
  const cut = text.length > SHOWN_CHARS;
  return <div className={'console-answer' + (ok ? '' : ' failed')} aria-label="运行结果">
    <p className="console-answer-head"><strong>HTTP {answer.status}</strong>{answer.exitCode !== null && <span>退出码 {answer.exitCode}</span>}<span className="num">{answer.ms} ms</span></p>
    <pre className="console-output scroll-area">{cut ? text.slice(0, SHOWN_CHARS) : text}</pre>
    {cut && <p className="card-note">只显示前 {SHOWN_CHARS.toLocaleString('zh-CN')} 个字符，共 {text.length.toLocaleString('zh-CN')} 个。</p>}
  </div>;
}

function CommandForm({ cmd }: { cmd: ConsoleCommand }) {
  const [values, setValues] = useState<Values>(() => initialValues(cmd));
  const [output, setOutput] = useState<'json' | 'out'>('json');
  const [asJob, setAsJob] = useState(cmd.file);
  const [argv, setArgv] = useState<string[] | null>(null);
  const [answer, setAnswer] = useState<RunAnswer | null>(null);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const [running, setRunning] = useState(false);
  const run = useRef<AbortController | null>(null);
  useEffect(() => () => run.current?.abort(), []);
  const problems = formProblems(cmd, values);
  const params = useMemo(() => paramsOf(cmd, values), [cmd, values]);
  const job = asJob && cmd.method === 'POST';

  // the command line it runs, asked of the server as the form changes
  useEffect(() => {
    if (problems.length) { setArgv(null); return; }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      commandLine(cmd, params, controller.signal).then(setArgv, () => { if (!controller.signal.aborted) setArgv(null); });
    }, 250);
    return () => { window.clearTimeout(timer); controller.abort(); };
  }, [cmd, params, problems.length]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (problems.length || cmd.refused) return;
    setNote(null); setAnswer(null);
    if (job) {
      try {
        await runJob('命令 · ' + cmd.command, 'POST', cmd.path.slice(1), params, cmd.file);
        setNote({ text: '已加入任务中心' + (cmd.file ? '，完成后在那里下载写出的文件。' : '。') });
      } catch (problem) {
        setNote({ text: (problem instanceof Error && problem.message) || '没有开始，请重试。', error: true });
      }
      return;
    }
    run.current?.abort();
    const controller = new AbortController();
    run.current = controller;
    setRunning(true);
    try {
      setAnswer(await runRaw(cmd, params, output === 'out' ? 'out' : undefined, controller.signal));
    } catch (problem) {
      if (!controller.signal.aborted) setNote({ text: '没有收到回复：' + ((problem instanceof Error && problem.message) || '连接中断'), error: true });
    } finally {
      if (run.current === controller) { setRunning(false); run.current = null; }
    }
  };

  return <form className="console-form" onSubmit={submit} aria-label={'运行 ' + cmd.command}>
    <header>
      <h3><Terminal size={16} aria-hidden="true" /><code>weflow {cmd.command}</code><span className="badge">{cmd.method}</span>{cmd.refused && <span className="badge refused">只读拒绝</span>}</h3>
      <p>{cmd.summary}</p>
      {cmd.description && <p className="console-description">{cmd.description}</p>}
    </header>
    {cmd.refused
      ? <p className="setting-warning"><Prohibit size={14} /> 这条命令要写入微信的数据库，而 WeFlow 只读打开数据库，所以它总是被拒绝（HTTP 501）。</p>
      : <>
        {cmd.fields.length ? <div className="console-fields">{cmd.fields.map(f => <FieldInput key={f.name} field={f} value={values[f.name]} onChange={value => setValues(old => ({ ...old, [f.name]: value }))} />)}</div>
          : <p className="muted">这条命令没有参数。</p>}
        <div className="console-options">
          {cmd.method === 'POST' && <label className="checkbox"><input type="checkbox" aria-label="作为任务运行" checked={asJob} onChange={event => setAsJob(event.target.checked)} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>作为任务运行 <small>{cmd.file ? '在任务中心看进度，并下载写出的文件' : '在任务中心看进度'}</small></span></label>}
          {!job && <Segmented className="console-output-choice" count={2} index={output === 'json' ? 0 : 1} role="group" aria-label="输出">
            <button type="button" aria-pressed={output === 'json'} onClick={() => setOutput('json')}>JSON</button>
            <button type="button" aria-pressed={output === 'out'} onClick={() => setOutput('out')}>终端文字</button>
          </Segmented>}
        </div>
        <p className="console-line" aria-label="等价的命令行">{problems.length ? <span className="setting-warning">{problems.join(' ')}</span> : argv ? <code>{shellLine(argv)}</code> : <span className="muted">…</span>}</p>
        <div className="console-actions">
          <button type="submit" className="primary-button" disabled={Boolean(problems.length) || running}>{running ? <CircleNotch size={16} className="spin" /> : <Play size={16} />}{running ? '正在运行' : job ? '加入任务' : '运行'}</button>
          {running && <button type="button" className="secondary-button" onClick={() => run.current?.abort()}>不再等待</button>}
        </div>
      </>}
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined}>{note.text}</p>}
    {answer && <Answer answer={answer} />}
  </form>;
}

export function CommandConsole({ pick, onPicked }: {
  /** A command to open, chosen from outside (the tools above the console). */
  pick: string | null;
  onPicked(): void;
}) {
  const [state, setState] = useState<{ status: 'loading' } | { status: 'ready'; commands: ConsoleCommand[] } | { status: 'error'; message: string }>({ status: 'loading' });
  const [round, setRound] = useState(0);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const formRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const controller = new AbortController();
    setState({ status: 'loading' });
    getOpenApi(controller.signal).then(commands => setState({ status: 'ready', commands }), problem => {
      if (!controller.signal.aborted) setState({ status: 'error', message: (problem instanceof Error && problem.message) || '读不到命令列表。' });
    });
    return () => controller.abort();
  }, [round]);
  const commands = state.status === 'ready' ? state.commands : [];
  useEffect(() => {
    if (!pick || !commands.length) return;
    const found = commands.find(c => c.id === pick || c.command === pick);
    if (found) { setSelected(found.id); setQuery(''); window.setTimeout(() => formRef.current?.scrollIntoView?.({ block: 'start', behavior: 'smooth' }), 0); }
    onPicked();
  }, [pick, commands, onPicked]);
  const shown = useMemo(() => findCommands(commands, query), [commands, query]);
  const groups = useMemo(() => {
    const map = new Map<string, ConsoleCommand[]>();
    for (const c of shown) map.set(c.tag, [...(map.get(c.tag) ?? []), c]);
    return [...map.entries()];
  }, [shown]);
  const chosen = commands.find(c => c.id === selected);

  return <section className="command-console" aria-label="命令控制台">
    <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" />
      <input type="search" aria-label="搜索命令" placeholder="搜索命令，如 chat messages 或 导出" value={query} onChange={event => setQuery(event.target.value)} />
    </div>
    {state.status === 'loading'
      ? <ListSkeleton label="正在读取命令列表" rows={4} />
      : state.status === 'error'
        ? <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={() => setRound(n => n + 1)}><ArrowClockwise size={14} />重试</button></div>
        : <div className="command-list scroll-area" role="list" aria-label="命令">
          {groups.length ? groups.map(([tag, list]) => <div key={tag} className="command-group" role="listitem">
            <h4>{tag}</h4>
            {list.map(c => <button type="button" key={c.id} className={'command-item' + (c.refused ? ' refused' : '')} aria-pressed={c.id === selected} onClick={() => setSelected(c.id)}>
              <code>{c.command}</code><span className="command-summary clamp-1">{c.summary}</span><span className="command-method">{c.method}</span>
            </button>)}
          </div>) : <p className="muted">没有匹配的命令。</p>}
        </div>}
    <div ref={formRef}>{chosen && <CommandForm key={chosen.id} cmd={chosen} />}</div>
  </section>;
}
