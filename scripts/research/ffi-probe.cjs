// Node/koffi ABI check. Reads a PRIVATE CLI config; prints only aggregate results.
// Usage: node scripts/research/ffi-probe.cjs <native library> <private config>
// db_path must point directly at an account directory (prefer an offline copy).
const fs = require('fs')
const path = require('path')
const crypto = require('crypto')
const koffi = require('koffi')

const library = path.resolve(process.argv[2])
const config = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'))
const profile = config.profiles[config.current_profile]
const lib = koffi.load(library)
const init = lib.func('int32 wcdb_init()')
const open = lib.func('int32 wcdb_open_account(const char* path, const char* key, _Out_ int64* handle)')
const close = lib.func('int32 wcdb_close_account(int64 handle)')
const sessions = lib.func('int32 wcdb_get_sessions(int64 handle, _Out_ void** outJson)')
const free = lib.func('void wcdb_free_string(void* ptr)')
const update = lib.func('int32 wcdb_update_message(int64 handle, const char* session, int32 id, int32 time, const char* text, _Out_ void** error)')
const handle = [0]
const initRc = init()
const openRc = open(path.join(profile.db_path, 'db_storage', 'session', 'session.db'), profile.decrypt_key, handle)
console.log(JSON.stringify({ librarySha256: crypto.createHash('sha256').update(fs.readFileSync(library)).digest('hex'), initRc, openRc }))
if (openRc !== 0) process.exitCode = 1
else {
  try {
    const out = [null]
    const rc = sessions(handle[0], out)
    try {
      const rows = JSON.parse(koffi.decode(out[0], 'char', -1))
      console.log(JSON.stringify({ sessionsRc: rc, array: Array.isArray(rows), count: Array.isArray(rows) ? rows.length : null }))
    } finally { if (out[0]) free(out[0]) }
    const error = [null]
    const writeRc = update(handle[0], 'wxid_synthetic_missing', 0, 0, 'synthetic', error)
    try { console.log(JSON.stringify({ writeRc, readOnly: writeRc === -4 })) }
    finally { if (error[0]) free(error[0]) }
  } finally { console.log(JSON.stringify({ closeRc: close(handle[0]) })) }
}
