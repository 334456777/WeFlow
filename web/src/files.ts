// Files the page makes and hands to the browser to save: a spreadsheet of rows, or any bytes.
import { strToU8, zipSync } from 'fflate';

export const xmlEscape = (value: string) => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll("'", '&apos;');

/** Starts the download of `bytes` as `fileName`; returns the object URL, to revoke once it is no longer offered. */
export function saveFile(bytes: Uint8Array, type: string, fileName: string): string {
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

export const XLSX_TYPE = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';

/** An .xlsx of one sheet: `rows` as text, the first as the header, columns `widths` characters wide. */
export function makeSheet(name: string, rows: string[][], widths: number[]): Uint8Array {
  const column = (index: number) => (index >= 26 ? String.fromCharCode(64 + Math.floor(index / 26)) : '') + String.fromCharCode(65 + (index % 26));
  const sheetRows = rows.map((row, rowIndex) => '<row r="' + String(rowIndex + 1) + '">' + row.map((value, columnIndex) => '<c r="' + column(columnIndex) + String(rowIndex + 1) + '" t="inlineStr"><is><t xml:space="preserve">' + xmlEscape(value) + '</t></is></c>').join('') + '</row>').join('');
  const cols = widths.map((width, index) => '<col min="' + String(index + 1) + '" max="' + String(index + 1) + '" width="' + String(width) + '" customWidth="1"/>').join('');
  const files: Record<string, Uint8Array> = {
    '[Content_Types].xml': strToU8('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>'),
    '_rels/.rels': strToU8('<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>'),
    'xl/workbook.xml': strToU8('<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="' + xmlEscape(name) + '" sheetId="1" r:id="rId1"/></sheets></workbook>'),
    'xl/_rels/workbook.xml.rels': strToU8('<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>'),
    'xl/worksheets/sheet1.xml': strToU8('<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols>' + cols + '</cols><sheetData>' + sheetRows + '</sheetData></worksheet>'),
  };
  return zipSync(files, { level: 6 });
}

/** A file name without the characters Windows does not allow. */
export const safeName = (name: string) => name.replace(/[<>:"/\\|?*]/g, '_');
