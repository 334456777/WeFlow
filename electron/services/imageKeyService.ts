import { app } from 'electron'
import { existsSync } from 'fs'
import { join } from 'path'

type ImageKeyResult = { success: boolean; xorKey?: number; aesKey?: string; verified?: boolean; error?: string }

let binding: { koffi: any; acquire: any; free: any } | undefined

function loadBinding() {
  if (binding) return binding
  const koffi = require('koffi')
  const relative = join('resources', 'native-db', 'win32', process.arch, 'weflow_wcdb.dll')
  const candidates = app.isPackaged
    ? [join(process.resourcesPath, relative)]
    : [join(process.cwd(), relative), join(app.getAppPath(), relative)]
  const path = candidates.find(existsSync)
  if (!path) throw new Error('Rust 图片取钥库未构建，请先运行 npm run native-db:build')
  const lib = koffi.load(path)
  binding = {
    koffi,
    acquire: lib.func('int weflow_get_image_keys(const char *accountDir, const char *wxid, const char *directoriesJson, _Out_ void **result)'),
    free: lib.func('void wcdb_free_string(void *result)')
  }
  return binding
}

export async function acquireImageKeys(accountDir?: string, wxid?: string, onProgress?: (message: string) => void): Promise<ImageKeyResult> {
  if (process.platform !== 'win32') return { success: false, error: '仅支持 Windows' }
  if (!accountDir?.trim()) return { success: false, error: '请先选择当前账号目录' }
  onProgress?.('正在从缓存目录采集并校验当前账号的图片密钥...')
  let result: any = null
  try {
    const { koffi, acquire, free } = loadBinding()
    const out: any[] = [null]
    // Koffi's asynchronous invocation keeps sample scanning and JPEG verification off
    // Electron's main thread; no user account data is logged here.
    const code = await new Promise<number>((resolve, reject) => {
      acquire.async(accountDir, wxid || null, null, out, (error: Error | null, status: number) => {
        if (error) reject(error)
        else resolve(status)
      })
    })
    result = out[0]
    try {
      const text = result ? koffi.decode(result, 'char', -1) : ''
      if (code !== 0) return { success: false, error: text || '获取图片密钥失败' }
      const parsed = JSON.parse(text)
      onProgress?.(parsed.verified ? '图片密钥已通过完整验证' : '图片 AES 已验证，XOR 尚未完整验证')
      return { success: true, xorKey: parsed.image_xor_key, aesKey: parsed.image_aes_key, verified: parsed.verified === true }
    } finally {
      if (result) free(result)
    }
  } catch (error) {
    return { success: false, error: error instanceof Error ? error.message : String(error) }
  }
}
