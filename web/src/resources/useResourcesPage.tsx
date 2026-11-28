// 资源浏览页（#206）：在页面上选类型，在操作控制台的卡片里选会话和日期。
import { useEffect, useRef, useState } from 'react';
import { Images } from '@phosphor-icons/react';
import type { OpenAt, Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import { ALL_RESOURCES, ResourceFilterCard, resourceFilterSummary, ResourcesPage } from './ResourcesPage';
import { t } from '../i18n';
import './resources.css';

/** 资源浏览页：页面视图和操作控制台的卡片。 */
export function useResourcesPage(shell: Shell, openAt: OpenAt): Page {
  const { scope, sessionList } = shell;
  const [filter, setFilter] = useState(ALL_RESOURCES);
  useEffect(() => { setFilter(ALL_RESOURCES); }, [scope]);
  const scroller = useRef<HTMLElement>(null);
  const cards = [{
    id: 'resources-filter', title: t('筛选资源'), summary: resourceFilterSummary(filter, sessionList),
    content: <ResourceFilterCard sessions={sessionList} filter={filter} onChange={setFilter} />,
  }];
  const view = <ScrollPage label={t('资源浏览')} icon={Images} scroller={scroller}>
    <Connected backend={shell.backend} offline={t('连接 weflow serve 后，可以在这里浏览所有会话的图片、视频、语音和文件。')}>
      <ResourcesPage key={scope} filter={filter} scroller={scroller} onPreview={shell.openPreview} onOpen={openAt} />
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: shell.records ? ['resources-filter'] : [] };
}
