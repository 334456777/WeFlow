// My footprint (#209): the range of days (today until another is picked), words to narrow the timeline to, and what
// was read, for the AI card of the console.
import { useRef, useState } from 'react';
import { Footprints } from '@phosphor-icons/react';
import { rangeOf, type Footprint, type FootprintChoice } from '../api/footprint';
import type { OpenAt, Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import { FootprintAi, FootprintExport, FootprintPage } from './Footprint';
import { t } from '../i18n';
import './footprint.css';

export function useFootprintPage(shell: Shell, openAt: OpenAt): Page {
  const [choice, setChoice] = useState<FootprintChoice>({ preset: 'today' });
  const range = rangeOf(choice);
  const [keyword, setKeyword] = useState('');
  const [data, setData] = useState<Footprint | null>(null);
  const scroller = useRef<HTMLElement>(null);
  const cards = [
    { id: 'footprint-ai', title: t('AI 足迹总结'), summary: range.label, content: <FootprintAi range={range} data={data} aiReady={shell.profile?.aiKey} /> },
    { id: 'footprint-export', title: t('导出足迹'), summary: range.label, content: <FootprintExport range={range} /> },
  ];
  const view = <ScrollPage label={t('我的足迹')} icon={Footprints} scroller={scroller}>
    <Connected backend={shell.backend} offline={t('连接 weflow serve 后，可以在这里查看你和谁聊过天、谁 @过你。')}>
      <FootprintPage key={shell.scope} choice={choice} onChoice={next => { setChoice(next); scroller.current?.scrollTo({ top: 0 }); }}
        keyword={keyword} onKeyword={setKeyword} onOpen={openAt} onData={setData} />
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: shell.records ? ['footprint-ai', 'footprint-export', 'tasks'] : [] };
}
