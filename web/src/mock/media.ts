// 示例数据的图片（按名称），以及语音消息播放的声音。
import benchChart from './images/bench-chart.svg';
import calligraphy from './images/calligraphy.svg';
import dinner from './images/dinner.svg';
import exportHtml from './images/export-html.svg';
import flowers from './images/flowers.svg';
import logo from './images/logo.svg';
import meetup from './images/meetup.svg';
import morning from './images/morning.svg';
import pingpong from './images/pingpong.svg';
import release from './images/release.svg';
import skinSketch from './images/skin-sketch.svg';
import sticker from './images/sticker.svg';
import street from './images/street.svg';
import webuiDark from './images/webui-dark.svg';
import webuiLight from './images/webui-light.svg';

/** 示例数据的图片，键是消息和动态里使用的名称。 */
export const pictures = {
  'bench-chart': benchChart, calligraphy, dinner, 'export-html': exportHtml, flowers, logo, meetup, morning, pingpong, release,
  'skin-sketch': skinSketch, sticker, street, 'webui-dark': webuiDark, 'webui-light': webuiLight,
};

/** 图片的名称。 */
export type Picture = keyof typeof pictures;

/** 名为 `name` 的图片的地址；没有这个名称的图片时为 logo 的地址。 */
export const pictureUrl = (name: string): string => pictures[name as Picture] ?? logo;

/**
 * 朋友圈示例媒体声称所在的地址：微信服务器的地址（网页只把这类地址当作图片），以图片的名称结尾，
 * 由示例服务端提供这张图片。
 */
export const snsAddress = (name: Picture) => 'https://sample.qpic.cn/mmsns/' + name;

let sound: string | null = null;

/** 一段短而柔和的提示音，WAV 格式，只生成一次：所有示例语音消息都播放它。 */
export function voiceSound(): string {
  if (sound) return sound;
  const rate = 8000;
  const length = Math.round(rate * 1.2);
  const bytes = new Uint8Array(44 + length * 2);
  const view = new DataView(bytes.buffer);
  const text = (at: number, value: string) => { for (let i = 0; i < value.length; i++) bytes[at + i] = value.charCodeAt(i); };
  text(0, 'RIFF'); view.setUint32(4, 36 + length * 2, true); text(8, 'WAVE');
  text(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, rate, true); view.setUint32(28, rate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, 'data'); view.setUint32(40, length * 2, true);
  for (let i = 0; i < length; i++) {
    const time = i / rate;
    const tone = time < 0.6 ? 523.25 : 659.25;
    const fade = Math.min(1, time * 20) * Math.exp(-3 * (time % 0.6));
    view.setInt16(44 + i * 2, Math.round(Math.sin(2 * Math.PI * tone * time) * fade * 6000), true);
  }
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  sound = 'data:audio/wav;base64,' + btoa(binary);
  return sound;
}
