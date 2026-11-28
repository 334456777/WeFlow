// A message's pictures and videos outside the chat window too: what the preview dialog opens (the chat window, the
// resources and the Moments open it), the play badge on a video, and why a thumbnail cannot be shown.
import { Play } from '@phosphor-icons/react';
import { mediaUrl, type ParsedMessage } from './api/messages';
import { t } from './i18n';

/** What the preview dialog shows: an image or a video, and how to go through the others of its gallery. */
export type Preview =
  | { type: 'image'; src: string; title: string; gallery?: Gallery; locate?: () => void; thumb?: PreviewThumb; hd?: PreviewHd }
  | { type: 'video'; src: string; poster?: string; title: string; gallery?: Gallery; locate?: () => void; thumb?: PreviewThumb };

/** The original WeChat downloads on View original (`_h.dat`) of a picture the preview shows as opened (`.dat`), to
 * show instead once it is on this computer (#194); `shown` once asked for, `loading` until it has come, `opened` the
 * image it took the place of, to go back to (#338). */
export interface PreviewHd { src: string; sessionId: string; localId: number; createTime: number; shown?: boolean; loading?: boolean; opened?: string }

/** A picture message as the preview opens it: the image WeChat shows when it is opened, with its original beside. */
export function imagePreview(sessionId: string, message: ParsedMessage, title: string): Extract<Preview, { type: 'image' }> {
  return {
    type: 'image', src: mediaUrl('image-data', sessionId, message, { variant: 'normal' }), title,
    hd: { src: mediaUrl('image-data', sessionId, message, { variant: 'hd' }), sessionId, localId: message.localId, createTime: message.createTime },
  };
}

/** The thumbnail a preview opened from (#194): the preview takes its shape at once and shows it until the picture
 * comes, so the picture that flies in from it has a place to land and does not move again when it loads. */
export interface PreviewThumb { src: string; aspect: number }

/** The pictures and videos of the conversation read so far, the one shown at `index` (#194). */
export interface Gallery { items: Preview[]; index: number }

/** The first byte of a media address, which says whether its file is there. */
const firstByte = (src: string) => fetch(src, { headers: { Range: 'bytes=0-0' } });

/** What a failed answer says: the server answers its error as JSON. */
async function failureIn(response: Response): Promise<string> {
  try {
    const doc = await response.json();
    return doc?.error?.message || t('无法读取（HTTP {status}）', { status: response.status });
  } catch {
    return t('无法读取');
  }
}

/** Why a media address failed: the server answers its error as JSON (a video not downloaded yet, an image in a format not supported yet…). */
export async function reasonOf(src: string): Promise<string> {
  try {
    return await failureIn(await firstByte(src));
  } catch {
    return t('无法读取');
  }
}

/** Whether the video at `src` is there to play, asked for its first byte; when not, why. */
export async function videoPlays(src: string): Promise<{ plays: boolean; reason: string }> {
  try {
    const answer = await firstByte(src);
    if (!answer.ok) return { plays: false, reason: await failureIn(answer) };
    await answer.body?.cancel();
    return { plays: true, reason: '' };
  } catch {
    return { plays: false, reason: t('无法读取') };
  }
}

/** Why the thumbnail at `src` failed, and with `video`, the address of the video it stands for, whether that still
 * plays. A video not on this computer answers why for itself and its thumbnail alike, so it is asked first. */
export async function thumbFailure(src: string, video?: string): Promise<{ reason: string; plays: boolean }> {
  if (video) {
    const answer = await videoPlays(video);
    if (!answer.plays) return answer;
  }
  return { plays: Boolean(video), reason: await reasonOf(src) };
}

/** The play button drawn on a video's thumbnail; on a thumbnail that failed, in place of the warning. */
export function PlayBadge({ size }: { size: number }) {
  return <span className="play-badge" aria-hidden="true"><Play size={size} weight="fill" /></span>;
}
