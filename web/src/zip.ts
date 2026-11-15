// A .zip archive, for the .xlsx the page saves (#783). The browser's CompressionStream deflates each file, so the
// first screen carries no deflate of its own; this writes the headers around them.

let crcTable: Int32Array | undefined;

/** CRC-32 of `bytes`, as zip headers record it. */
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

/** A .zip of `files`, by path, each deflated and stamped with the current local time. */
export async function zip(files: Record<string, Uint8Array<ArrayBuffer>>): Promise<Uint8Array<ArrayBuffer>> {
  const now = new Date();
  // MS-DOS time and date: seconds halved, years from 1980
  const stamp = (now.getFullYear() - 1980) << 25 | (now.getMonth() + 1) << 21 | now.getDate() << 16
    | now.getHours() << 11 | now.getMinutes() << 5 | now.getSeconds() >> 1;
  const encoder = new TextEncoder();
  const entries = await Promise.all(Object.entries(files).map(async ([path, data]) =>
    ({ name: encoder.encode(path), data, packed: await deflate(data), crc: crc32(data) })));
  const out = new Uint8Array(entries.reduce((n, e) => n + 76 + 2 * e.name.length + e.packed.length, 22));
  const view = new DataView(out.buffer);
  const u16 = (at: number, value: number) => view.setUint16(at, value, true);
  const u32 = (at: number, value: number) => view.setUint32(at, value, true);
  // The fields the local and the central header share, from "version needed to extract" to the name's length
  const shared = (at: number, e: typeof entries[number]) => {
    u16(at, 20);
    u16(at + 4, 8); // deflate
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
    u16(central + 4, 20); // made by: version 2.0
    shared(central + 6, e);
    u32(central + 42, local);
    out.set(e.name, central + 46);
    local += 30 + e.name.length + e.packed.length;
    central += 46 + e.name.length;
  }
  // end of central directory
  u32(central, 0x06054b50);
  u16(central + 8, entries.length);
  u16(central + 10, entries.length);
  u32(central + 12, central - start);
  u32(central + 16, start);
  return out;
}
