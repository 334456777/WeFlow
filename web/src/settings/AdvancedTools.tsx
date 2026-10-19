// The advanced part of the settings (#217): the CLI's own tools, each opened in the command console below them.
import { ArrowRight } from '@phosphor-icons/react';

export const TOOLS = [
  { command: 'db open', title: '打开数据库', text: '用当前档案的目录和密钥打开数据库，显示概要。' },
  { command: 'config import', title: '从桌面端导入配置', text: '读取 WeFlow 桌面端的 config.json；加密保存的值要重新设置。' },
  { command: 'ffmpeg install', title: '安装 ffmpeg', text: '给内置解码器读不了的 WXGF 图片使用。' },
  { command: 'image auto-download start', title: '图片自动下载', text: '在微信里自动下载聊天图片的原图，只支持 Windows。' },
  { command: 'runtime info', title: '运行时', text: '内置运行时解压到哪里、版本和目标平台。' },
] as const;

export function AdvancedTools({ onOpen }: { onOpen(command: string): void }) {
  return <ul className="advanced-tools">
    {TOOLS.map(t => <li key={t.command}>
      <div><h3>{t.title}</h3><p>{t.text}</p><code>weflow {t.command}</code></div>
      <button type="button" className="secondary-button" onClick={() => onOpen(t.command)}>在控制台打开<ArrowRight size={12} /></button>
    </li>)}
  </ul>;
}
