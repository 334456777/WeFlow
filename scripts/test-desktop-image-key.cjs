// Windows adapter smoke test: build the Rust DLL, then run this file with Node.
// All account data and cache files are synthetic and stay in the ignored temp/ folder.
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const Module = require('node:module')
const esbuild = require('esbuild')
const sharp = require('sharp')

const root = path.resolve(__dirname, '..')
const fixture = path.join(root, 'temp', `desktop-image-key-${process.pid}`)
assert.ok(fixture.startsWith(path.resolve(root, 'temp') + path.sep))
const originalAppdata = process.env.APPDATA

function encrypt(plain, code, identity, xorLength) {
  const aes = crypto.createHash('md5').update(`${code}${identity}`).digest('hex').slice(0, 16)
  const cipher = crypto.createCipheriv('aes-128-ecb', Buffer.from(aes), null)
  const encrypted = Buffer.concat([cipher.update(plain.subarray(0, 16)), cipher.final()])
  const header = Buffer.alloc(15)
  Buffer.from([7, 8, 0x56, 0x32, 8, 7]).copy(header)
  header.writeInt32LE(16, 6)
  header.writeInt32LE(xorLength, 10)
  header[14] = 1
  const tail = Buffer.from(plain.subarray(plain.length - xorLength))
  for (let i = 0; i < tail.length; i++) tail[i] ^= code & 255
  return { aes, data: Buffer.concat([header, encrypted, plain.subarray(16, plain.length - xorLength), tail]) }
}

function loadService() {
  const source = path.join(root, 'electron', 'services', 'imageKeyService.ts')
  const bundle = esbuild.buildSync({ entryPoints: [source], bundle: true, platform: 'node', packages: 'external', write: false }).outputFiles[0].text
  const module = new Module(source, moduleParent)
  module.filename = source
  module.paths = Module._nodeModulePaths(path.dirname(source))
  const original = Module._load
  Module._load = function (id, ...args) {
    return id === 'electron' ? { app: { isPackaged: false, getAppPath: () => root } } : original.call(this, id, ...args)
  }
  try { module._compile(bundle, source) } finally { Module._load = original }
  return module.exports.acquireImageKeys
}
const moduleParent = module

async function main() {
  assert.equal(process.platform, 'win32', 'This adapter uses the Windows DLL')
  fs.mkdirSync(fixture, { recursive: true })
  process.env.APPDATA = path.join(fixture, 'appdata')
  const kvcomm = path.join(process.env.APPDATA, 'Tencent', 'xwechat', 'net', 'kvcomm')
  fs.mkdirSync(kvcomm, { recursive: true })
  fs.writeFileSync(path.join(kvcomm, 'key_123_456.statistic'), '')
  const account = path.join(fixture, 'wxid_synthetic_ab12')
  fs.mkdirSync(account)
  const pixels = Buffer.from(Array.from({ length: 32 * 32 * 3 }, (_, n) => n * 17 % 256))
  const jpeg = await sharp(pixels, { raw: { width: 32, height: 32, channels: 3 } }).jpeg().toBuffer()
  const encrypted = encrypt(jpeg, 123, 'wxid_synthetic', 64)
  fs.writeFileSync(path.join(account, 'sample_t.dat'), encrypted.data)
  const acquire = loadService()
  const messages = []
  const result = await acquire(account, 'wxid_synthetic', message => messages.push(message))
  assert.deepEqual(result, { success: true, xorKey: 123, aesKey: encrypted.aes, verified: true })
  assert.equal(messages.length, 2)
  assert.equal((await acquire()).success, false)
  const other = path.join(fixture, 'wxid_unrelated_ab12')
  fs.mkdirSync(other)
  assert.equal((await acquire(other, 'wxid_unrelated')).success, false)
  fs.writeFileSync(path.join(account, 'sample_t.dat'), encrypt(jpeg, 123, 'wxid_synthetic', 0).data)
  assert.equal((await acquire(account, 'wxid_synthetic')).verified, false)
  fs.rmSync(kvcomm, { recursive: true })
  assert.equal((await acquire(account, 'wxid_synthetic')).success, false)
  console.log('Desktop image-key adapter: async Rust DLL, verification and failure cases passed')
}

main().catch(error => { console.error(error); process.exitCode = 1 }).finally(() => {
  if (originalAppdata === undefined) delete process.env.APPDATA
  else process.env.APPDATA = originalAppdata
  if (fs.existsSync(fixture)) fs.rmSync(fixture, { recursive: true })
})
