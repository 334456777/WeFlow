// 预览右下角的「下载」按钮（#341）：保存预览正在显示的那个文件，即点开时的图片（`.dat`）、换成原图后的原图
// （`_h.dat`），或者视频。文件名取消息的时间。
import { useState } from 'react';
import { CircleNotch, DownloadSimple } from '@phosphor-icons/react';
import { t } from '../i18n';

const EXTENSIONS: Record<string, string> = {
  'image/jpeg': 'jpg', 'image/png': 'png', 'image/gif': 'gif', 'image/webp': 'webp', 'image/heic': 'heic', 'video/mp4': 'mp4',
};

/** 下载的文件名，如 `微信图片_20261020_085900_原图.jpg`：种类、地址里的时间（`create-time`），原图另加标记。 */
export function downloadName(src: string, kind: 'image' | 'video', type: string, original: boolean): string {
  const seconds = Number(new URL(src, 'http://page').searchParams.get('create-time'));
  const pad = (n: number) => String(n).padStart(2, '0');
  const at = seconds > 0 ? new Date(seconds * 1000) : null;
  const stamp = at ? '_' + at.getFullYear() + pad(at.getMonth() + 1) + pad(at.getDate()) + '_' + pad(at.getHours()) + pad(at.getMinutes()) + pad(at.getSeconds()) : '';
  const extension = EXTENSIONS[type.split(';')[0].trim()] ?? (kind === 'video' ? 'mp4' : 'jpg');
  return (kind === 'video' ? t('微信视频') : t('微信图片')) + stamp + (original ? t('_原图') : '') + '.' + extension;
}

/** 预览的「下载」按钮。 */
export function PreviewDownload({ src, kind, original }: { src: string; kind: 'image' | 'video'; original: boolean }) {
  const [state, setState] = useState<'idle' | 'busy' | 'failed'>('idle');
  const save = async () => {
    setState('busy');
    try {
      const answer = await fetch(src);
      if (!answer.ok) throw new Error(String(answer.status));
      const blob = await answer.blob();
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = downloadName(src, kind, blob.type, original);
      document.body.append(link);
      link.click();
      link.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      setState('idle');
    } catch {
      setState('failed');
    }
  };
  const label = state === 'failed' ? t('下载失败，点此重试') : kind === 'video' ? t('下载视频') : original ? t('下载高清原图') : t('下载图片');
  return <button type="button" className={'icon-button preview-download' + (state === 'failed' ? ' failed' : '')} aria-label={label} title={label}
    disabled={state === 'busy'} onClick={save}>
    {state === 'busy' ? <CircleNotch size={18} className="spin" /> : <DownloadSimple size={18} />}
  </button>;
}
