// 「下载」 in the preview's corner (#341): saves the file the preview shows, as it shows it: the image opened (`.dat`)
// or its original (`_h.dat`) once shown instead, or the video. Named after the message's time.
import { useState } from 'react';
import { CircleNotch, DownloadSimple } from '@phosphor-icons/react';

const EXTENSIONS: Record<string, string> = {
  'image/jpeg': 'jpg', 'image/png': 'png', 'image/gif': 'gif', 'image/webp': 'webp', 'image/heic': 'heic', 'video/mp4': 'mp4',
};

/** `微信图片_20261020_085900_原图.jpg`: the kind, the time from the address (`create-time`), the original marked. */
export function downloadName(src: string, kind: 'image' | 'video', type: string, original: boolean): string {
  const seconds = Number(new URL(src, 'http://page').searchParams.get('create-time'));
  const pad = (n: number) => String(n).padStart(2, '0');
  const at = seconds > 0 ? new Date(seconds * 1000) : null;
  const stamp = at ? '_' + at.getFullYear() + pad(at.getMonth() + 1) + pad(at.getDate()) + '_' + pad(at.getHours()) + pad(at.getMinutes()) + pad(at.getSeconds()) : '';
  const extension = EXTENSIONS[type.split(';')[0].trim()] ?? (kind === 'video' ? 'mp4' : 'jpg');
  return (kind === 'video' ? '微信视频' : '微信图片') + stamp + (original ? '_原图' : '') + '.' + extension;
}

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
  const label = state === 'failed' ? '下载失败，点此重试' : kind === 'video' ? '下载视频' : original ? '下载高清原图' : '下载图片';
  return <button type="button" className={'icon-button preview-download' + (state === 'failed' ? ' failed' : '')} aria-label={label} title={label}
    disabled={state === 'busy'} onClick={save}>
    {state === 'busy' ? <CircleNotch size={18} className="spin" /> : <DownloadSimple size={18} />}
  </button>;
}
