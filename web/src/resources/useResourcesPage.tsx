// The resource browser (#206): the page picks the type, its console card the conversation and days.
import { useEffect, useState } from 'react';
import { Images } from '@phosphor-icons/react';
import type { OpenAt, Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import { ALL_RESOURCES, ResourceFilterCard, resourceFilterSummary, ResourcesPage } from './ResourcesPage';

export function useResourcesPage(shell: Shell, openAt: OpenAt): Page {
  const { scope, sessionList } = shell;
  const [filter, setFilter] = useState(ALL_RESOURCES);
  useEffect(() => { setFilter(ALL_RESOURCES); }, [scope]);
  const cards = [{
    id: 'resources-filter', title: '筛选资源', summary: resourceFilterSummary(filter, sessionList),
    content: <ResourceFilterCard sessions={sessionList} filter={filter} onChange={setFilter} />,
  }];
  const view = <ScrollPage label="资源浏览" icon={Images}>
    <Connected backend={shell.backend} offline="连接 weflow serve 后，可以在这里浏览所有会话的图片、视频、语音和文件。">
      <ResourcesPage key={scope} filter={filter} onPreview={shell.openPreview} onOpen={openAt} />
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: shell.connected ? ['resources-filter'] : [] };
}
