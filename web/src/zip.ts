// 生成 .zip 压缩包，用于网页保存的 .xlsx（#783）。每个文件由浏览器的 CompressionStream 做 deflate 压缩，
// 首屏的代码因此不必自带 deflate 的实现；这里只写各个文件的头和末尾的中央目录。

let crcTable: Int32Array | undefined;

/** `bytes` 的 CRC-32，即 zip 文件头里记录的校验值。 */
function crc32(bytes: Uint8Array): number {
  crcTable ??= Int32Array.from({ length: 256 }, (_, n) => {
    for (let k = 0; k < 8; k++) n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
    return n;
  });
  let crc = -1;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 255] ^ (crc >>> 8);
  return ~crc >>> 0;
}

async function deflate(bytes: Uint8Array<ArrayBuffer>): Promise<Uint8Array> {
  const stream = new Blob([bytes]).stream().pipeThrough(new CompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** 把 `files`（键为路径）打包成 .zip：每个文件都做 deflate 压缩，修改时间记为当前的本地时间。 */
export async function zip(files: Record<string, Uint8Array<ArrayBuffer>>): Promise<Uint8Array<ArrayBuffer>> {
  const now = new Date();
  // MS-DOS 格式的时间和日期：秒数除以 2，年份从 1980 年算起
  const stamp = (now.getFullYear() - 1980) << 25 | (now.getMonth() + 1) << 21 | now.getDate() << 16
    | now.getHours() << 11 | now.getMinutes() << 5 | now.getSeconds() >> 1;
  const encoder = new TextEncoder();
  const entries = await Promise.all(Object.entries(files).map(async ([path, data]) =>
    ({ name: encoder.encode(path), data, packed: await deflate(data), crc: crc32(data) })));
  const out = new Uint8Array(entries.reduce((n, e) => n + 76 + 2 * e.name.length + e.packed.length, 22));
  const view = new DataView(out.buffer);
  const u16 = (at: number, value: number) => view.setUint16(at, value, true);
  const u32 = (at: number, value: number) => view.setUint32(at, value, true);
  // 本地文件头和中央目录里的文件头共有的字段：从 `version needed to extract` 到文件名的长度
  const shared = (at: number, e: typeof entries[number]) => {
    u16(at, 20);
    u16(at + 4, 8); // 压缩方式为 deflate
    u32(at + 6, stamp);
    u32(at + 10, e.crc);
    u32(at + 14, e.packed.length);
    u32(at + 18, e.data.length);
    u16(at + 22, e.name.length);
  };
  let local = 0;
  const start = entries.reduce((n, e) => n + 30 + e.name.length + e.packed.length, 0);
  let central = start;
  for (const e of entries) {
    u32(local, 0x04034b50);
    shared(local + 4, e);
    out.set(e.name, local + 30);
    out.set(e.packed, local + 30 + e.name.length);
    u32(central, 0x02014b50);
    u16(central + 4, 20); // `version made by` 为 2.0
    shared(central + 6, e);
    u32(central + 42, local);
    out.set(e.name, central + 46);
    local += 30 + e.name.length + e.packed.length;
    central += 46 + e.name.length;
  }
  // 中央目录结束记录（end of central directory）
  u32(central, 0x06054b50);
  u16(central + 8, entries.length);
  u16(central + 10, entries.length);
  u32(central + 12, central - start);
  u32(central + 16, start);
  return out;
}
