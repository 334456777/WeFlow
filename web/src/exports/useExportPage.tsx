// The export page (#211): the conversations picked from the list, and how they are exported, one job each.
import { useEffect, useMemo, useState } from 'react';
import type { Page, Shell } from '../shell/page';
import { BrowserPanel, ConnectedList } from '../shell/panels';
import { batchSummary, BatchExport, DEFAULT_BATCH, SessionsExport, type BatchOptions } from './ExportCards';
import { ExportPicker, PickedList } from './ExportPicker';
import { t } from '../i18n';

export function useExportPage(shell: Shell): Page {
  const { connected, scope, sessionList, setMobileDetail } = shell;
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  const [batch, setBatch] = useState<BatchOptions>(DEFAULT_BATCH);
  useEffect(() => { setPicked(new Set()); }, [scope]);
  const pickedSessions = useMemo(() => sessionList.filter(s => picked.has(s.username)), [sessionList, picked]);
  const cards = [
    {
      id: 'batch-export', title: t('批量导出'), summary: batchSummary(pickedSessions.length, batch),
      content: <BatchExport picked={pickedSessions} options={batch} onChange={setBatch} />,
    },
    { id: 'sessions-export', title: t('导出会话列表'), content: <SessionsExport picked={pickedSessions} /> },
  ];
  const view = <BrowserPanel label={t('导出')} detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back={t('返回会话列表')} title={t('已选的会话')} paneLabel={t('已选的会话')}
    list={<ConnectedList backend={shell.backend} offline={t('连接 weflow serve 后，可以在这里选择会话批量导出。')}>
      <ExportPicker state={shell.sessions} picked={picked} onChange={setPicked} onRetry={shell.reloadSessions} />
    </ConnectedList>}>
    {connected && <PickedList picked={pickedSessions} onRemove={id => setPicked(old => { const next = new Set(old); next.delete(id); return next; })} />}
  </BrowserPanel>;
  return { view, cards, shown: ['tasks', ...(connected ? cards.map(card => card.id) : [])] };
}
