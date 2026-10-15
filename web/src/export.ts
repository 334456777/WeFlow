// Export of the sample records, made in the browser. Real exports will run `export messages` on the server and
// fetch the file with `?return=file` (docs/serve-api.md).
import { strToU8, zipSync } from 'fflate';
import { categoryLabels, getMessagesInRange } from './mock/data';
import type { Conversation, ExportOptions, ExportResult, Message } from './types';

const xmlEscape = (value: string) => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll("'", '&apos;');

function download(bytes: Uint8Array, type: string, fileName: string): string {
  const copy = new Uint8Array(bytes.length);
  copy.set(bytes);
  const url = URL.createObjectURL(new Blob([copy], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = fileName;
  document.body.append(link);
  link.click();
  link.remove();
  return url;
}

export function makeSpreadsheet(messages: Message[]): Uint8Array {
  const rows = [
    ['日期', '时间', '发送者', '类型', '内容', '媒体文件'],
    ...messages.map(message => [message.date, message.time, message.sender, ({ text: '文本', image: '图片', file: '文件' })[message.kind], message.text, message.fileName || '']),
  ];
  const sheetRows = rows.map((row, rowIndex) => '<row r="' + String(rowIndex + 1) + '">' + row.map((value, columnIndex) => '<c r="' + String.fromCharCode(65 + columnIndex) + String(rowIndex + 1) + '" t="inlineStr"><is><t xml:space="preserve">' + xmlEscape(value) + '</t></is></c>').join('') + '</row>').join('');
  const files: Record<string, Uint8Array> = {
    '[Content_Types].xml': strToU8('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>'),
    '_rels/.rels': strToU8('<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>'),
    'xl/workbook.xml': strToU8('<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="记录" sheetId="1" r:id="rId1"/></sheets></workbook>'),
    'xl/_rels/workbook.xml.rels': strToU8('<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>'),
    'xl/worksheets/sheet1.xml': strToU8('<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="3" width="20" customWidth="1"/><col min="4" max="4" width="10" customWidth="1"/><col min="5" max="5" width="64" customWidth="1"/><col min="6" max="6" width="28" customWidth="1"/></cols><sheetData>' + sheetRows + '</sheetData></worksheet>'),
  };
  return zipSync(files, { level: 6 });
}

async function fetchAsset(path: string): Promise<Blob> {
  const response = await fetch(path);
  if (!response.ok) throw new Error('媒体文件读取失败，请重试。');
  return response.blob();
}

async function imageAsDataUrl(path: string): Promise<string> {
  const blob = await fetchAsset(path);
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error('媒体文件读取失败，请重试。'));
    reader.readAsDataURL(blob);
  });
}

export async function exportConversation(conversation: Conversation, options: ExportOptions): Promise<ExportResult> {
  if (!options.from || !options.to || options.from > options.to) throw new Error('请选择有效的时间范围。');
  const messages = getMessagesInRange(conversation, options.from, options.to);
  if (!messages.length) throw new Error('当前时间范围内没有可导出的记录。');
  const name = (conversation.name + '_' + options.from + '_' + options.to).replace(/[<>:"/\\|?*]/g, '_');
  const mediaName = (message: Message) => 'media/' + (message.fileName || message.id);
  let bytes: Uint8Array;
  let fileName: string;
  let type: string;
  if (options.format === 'JSON') {
    bytes = strToU8(JSON.stringify({ source: 'WeFlow 示例数据', conversation: conversation.name, category: categoryLabels[conversation.category], from: options.from, to: options.to, messages: messages.map(({ asset, ...message }) => ({ ...message, ...(options.includeMedia && asset ? { media: mediaName(message) } : {}) })) }, null, 2));
    fileName = name + '.json'; type = 'application/json;charset=utf-8';
  } else if (options.format === 'Excel') {
    bytes = makeSpreadsheet(messages);
    fileName = name + '.xlsx'; type = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
  } else {
    const items = await Promise.all(messages.map(async message => {
      const image = message.asset && options.includeMedia ? '<img src="' + await imageAsDataUrl(message.asset) + '" alt="' + xmlEscape(message.text) + '" />' : '';
      return '<article class="' + message.direction + '"><small>' + xmlEscape(message.sender + ' · ' + message.date + ' ' + message.time) + '</small><p>' + xmlEscape(message.text) + '</p>' + image + '</article>';
    }));
    bytes = strToU8('<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>' + xmlEscape(conversation.name) + '的记录</title><style>body{font:16px/1.6 system-ui,sans-serif;max-width:850px;margin:36px auto;padding:0 24px;color:#202326;background:#fafbfc}header{border-bottom:1px solid #e1e5e9;margin-bottom:24px}article{max-width:80%;padding:12px 18px;border:1px solid #e1e5e9;border-radius:16px;margin:12px 0;background:white}.outgoing{margin-left:auto;background:#eceef0}small{color:#747c91}p{white-space:pre-wrap;margin:5px 0}img{max-width:100%;border-radius:8px}</style><header><h1>' + xmlEscape(conversation.name) + '</h1><p>' + options.from + ' ↔ ' + options.to + ' · 示例数据</p></header>' + items.join('') + '</html>');
    fileName = name + '.html'; type = 'text/html;charset=utf-8';
  }
  const assets = messages.filter(message => message.asset);
  if (options.includeMedia && assets.length && options.format !== 'HTML') {
    const archive: Record<string, Uint8Array> = { [fileName]: bytes };
    for (const message of assets) archive[mediaName(message)] = new Uint8Array(await (await fetchAsset(message.asset!)).arrayBuffer());
    archive['README.txt'] = strToU8('本文件由 WeFlow 导出，使用示例数据。\n媒体文件保存在 media 文件夹中。\n');
    bytes = zipSync(archive, { level: 6 });
    fileName = name + '.zip'; type = 'application/zip';
  }
  const downloadUrl = download(bytes, type, fileName);
  return { fileName, count: messages.length, downloadUrl };
}
