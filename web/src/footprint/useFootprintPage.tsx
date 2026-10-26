// My footprint (#209): the range of days (today until another is picked), words to narrow the timeline to, and what
// was read, for the AI card of the console.
import { useRef, useState } from 'react';
import { Footprints } from '@phosphor-icons/react';
import { presetRanges, type Footprint, type FootprintRange } from '../api/footprint';
import type { OpenAt, Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import { FootprintAi, FootprintExport, FootprintPage } from './Footprint';

export function useFootprintPage(shell: Shell, openAt: OpenAt): Page {
  const [range, setRange] = useState<FootprintRange>(() => presetRanges()[0]);
  const [keyword, setKeyword] = useState('');
  const [data, setData] = useState<Footprint | null>(null);
  const scroller = useRef<HTMLElement>(null);
  const cards = [
    { id: 'footprint-ai', title: 'AI 足迹总结', summary: range.label, content: <FootprintAi range={range} data={data} aiReady={shell.profile?.aiKey} /> },
    { id: 'footprint-export', title: '导出足迹', summary: range.label, content: <FootprintExport range={range} /> },
  ];
  const view = <ScrollPage label="我的足迹" icon={Footprints} scroller={scroller}>
    <Connected backend={shell.backend} offline="连接 weflow serve 后，可以在这里查看你和谁聊过天、谁 @过你。">
      <FootprintPage key={shell.scope} range={range} onRange={next => { setRange(next); scroller.current?.scrollTo({ top: 0 }); }}
        keyword={keyword} onKeyword={setKeyword} onOpen={openAt} onData={setData} />
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: shell.connected ? ['footprint-ai', 'footprint-export', 'tasks'] : [] };
}
