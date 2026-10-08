// Builds the Rust database layer (crates/weflow-wcdb-ffi) for this platform and copies it to
// resources/native-db/<platform>/<arch>/, where electron/services/wcdbCore.ts loads it from.
// Usage: node scripts/build-native-db.cjs [--target <rust target triple>]
const { execFileSync } = require('child_process')
const fs = require('fs')
const path = require('path')

const root = path.resolve(__dirname, '..')
const argTarget = (() => {
  const i = process.argv.indexOf('--target')
  return i >= 0 ? process.argv[i + 1] : process.env.WEFLOW_NATIVE_DB_TARGET || ''
})()

const platform = argTarget
  ? (argTarget.includes('windows') ? 'win32' : argTarget.includes('apple') ? 'darwin' : 'linux')
  : process.platform
const arch = argTarget ? (argTarget.startsWith('aarch64') ? 'arm64' : 'x64') : (process.arch === 'arm64' ? 'arm64' : 'x64')
const libName = platform === 'win32' ? 'weflow_wcdb.dll' : platform === 'darwin' ? 'libweflow_wcdb.dylib' : 'libweflow_wcdb.so'
const platformDir = platform === 'darwin' ? 'macos' : platform === 'win32' ? 'win32' : 'linux'

const args = ['build', '--release', '-p', 'weflow-wcdb-ffi']
if (platform === 'win32' && arch === 'x64') args.push('-p', 'weflow-wxkey')
if (argTarget) args.push('--target', argTarget)
console.log(`[native-db] cargo ${args.join(' ')}`)
execFileSync('cargo', args, { cwd: root, stdio: 'inherit' })

const built = path.join(root, 'target', ...(argTarget ? [argTarget] : []), 'release', libName)
if (!fs.existsSync(built)) {
  console.error(`[native-db] build output not found: ${built}`)
  process.exit(1)
}
const destDir = path.join(root, 'resources', 'native-db', platformDir, arch)
fs.mkdirSync(destDir, { recursive: true })
fs.copyFileSync(built, path.join(destDir, libName))
console.log(`[native-db] ${path.relative(root, path.join(destDir, libName))}`)

if (platform === 'win32' && arch === 'x64') {
  const keyDir = path.join(root, 'resources', 'native-key', 'win32', 'x64')
  fs.mkdirSync(keyDir, { recursive: true })
  fs.copyFileSync(path.join(root, 'target', ...(argTarget ? [argTarget] : []), 'release', 'wx_key.dll'), path.join(keyDir, 'wx_key.dll'))
  console.log('[native-key] resources/native-key/win32/x64/wx_key.dll')
}
