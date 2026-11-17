// The sample's pictures, by name, and the sound its voice messages play.
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

export const pictures = {
  'bench-chart': benchChart, calligraphy, dinner, 'export-html': exportHtml, flowers, logo, meetup, morning, pingpong, release,
  'skin-sketch': skinSketch, sticker, street, 'webui-dark': webuiDark, 'webui-light': webuiLight,
};

export type Picture = keyof typeof pictures;

export const pictureUrl = (name: string): string => pictures[name as Picture] ?? logo;

/**
 * Where Moments' sample media say they are: a WeChat server's address (as the page takes only those as pictures),
 * ending in the picture's name, which the sample serves.
 */
export const snsAddress = (name: Picture) => 'https://sample.qpic.cn/mmsns/' + name;

let sound: string | null = null;

/** A short soft chime as a WAV, made once: what every sample voice message plays. */
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
