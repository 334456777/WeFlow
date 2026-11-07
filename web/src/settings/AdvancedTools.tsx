// The advanced part of the settings (#217): the CLI's own tools, each opened in the command console below them.
import { ArrowRight } from '@phosphor-icons/react';
import { msg, t } from '../i18n';

// shown through t()
export const TOOLS = [
  { command: 'db open', title: msg('打开数据库'), text: msg('用当前档案的目录和密钥打开数据库，显示概要。') },
  { command: 'image auto-download start', title: msg('图片自动下载'), text: msg('在微信里自动下载聊天图片的原图，只支持 Windows。') },
  { command: 'runtime info', title: msg('运行时'), text: msg('内置运行时解压到哪里、版本和目标平台。') },
] as const;

export function AdvancedTools({ onOpen }: { onOpen(command: string): void }) {
  return <ul className="advanced-tools">
    {TOOLS.map(tool => <li key={tool.command}>
      <div><h3>{t(tool.title)}</h3><p>{t(tool.text)}</p><code>weflow {tool.command}</code></div>
      <button type="button" className="secondary-button" onClick={() => onOpen(tool.command)}>{t('在控制台打开')}<ArrowRight size={12} /></button>
    </li>)}
  </ul>;
}
