// 设置的高级部分（#217）：命令行自带的工具，每个都在下方的命令控制台里打开。
import { ArrowRight } from '@phosphor-icons/react';
import { SettingRow } from './layout';
import { msg, t } from '../i18n';

/** 工具列表：每项是一个命令，带名称和用途的说明（显示时经过 t()）。 */
export const TOOLS = [
  { command: 'db test', title: msg('打开数据库'), text: msg('用当前档案的目录和密钥打开数据库，显示概要。') },
  { command: 'image auto-download start', title: msg('原图自动下载'), text: msg('微信下载聊天图片时改为下载原图，只支持 Windows。') },
] as const;

/** 每个工具一行，行上的按钮在命令控制台里打开它的命令。 */
export function AdvancedTools({ onOpen }: { onOpen(command: string): void }) {
  return <div className="advanced-tools">
    {TOOLS.map(tool => <SettingRow key={tool.command} name={t(tool.title)} text={<>{t(tool.text)}<code>weflow {tool.command}</code></>}>
      <button type="button" className="secondary-button" onClick={() => onOpen(tool.command)}>{t('在控制台打开')}<ArrowRight size={12} /></button>
    </SettingRow>)}
  </div>;
}
