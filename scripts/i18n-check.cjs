#!/usr/bin/env node
/**
 * Verifies the English dictionaries cover every translated string.
 *
 *   renderer:     every t('…') / tr('…') key under src/ must exist in src/i18n/locales/en
 *   main process: every mt('…') key under electron/ must exist in electron/i18n/en.ts
 *
 * It also checks that {placeholders} in a key and its English value match.
 * Exit code 1 when anything is missing or inconsistent.
 */
const fs = require('fs')
const path = require('path')
const ts = require('typescript')

const root = path.resolve(__dirname, '..')

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      if (entry.name !== 'node_modules' && entry.name !== 'i18n') walk(file, out)
    } else if (/\.tsx?$/.test(entry.name) && !/\.d\.ts$/.test(entry.name)) {
      out.push(file)
    }
  }
  return out
}

function collectKeys(dir, names) {
  const keys = new Map()
  for (const file of walk(dir)) {
    const source = fs.readFileSync(file, 'utf8')
    if (!/i18n/.test(source)) continue
    const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
    const visit = (node) => {
      if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && names.includes(node.expression.text) && node.arguments.length) {
        const arg = node.arguments[0]
        if (ts.isStringLiteral(arg) || ts.isNoSubstitutionTemplateLiteral(arg)) {
          const list = keys.get(arg.text) || []
          list.push(path.relative(root, file))
          keys.set(arg.text, list)
        }
      }
      ts.forEachChild(node, visit)
    }
    visit(sf)
  }
  return keys
}

// Load a dictionary by evaluating the compiled-on-the-fly TypeScript modules.
function loadDictionary(entry) {
  const cache = new Map()
  const load = (file) => {
    if (cache.has(file)) return cache.get(file).exports
    const code = ts.transpileModule(fs.readFileSync(file, 'utf8'), { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText
    const mod = { exports: {} }
    cache.set(file, mod)
    const dir = path.dirname(file)
    const localRequire = (spec) => {
      if (!spec.startsWith('.')) return require(spec)
      const base = path.resolve(dir, spec)
      const resolved = [base + '.ts', path.join(base, 'index.ts')].find((candidate) => fs.existsSync(candidate))
      return load(resolved)
    }
    new Function('exports', 'require', 'module', code)(mod.exports, localRequire, mod)
    return mod.exports
  }
  return load(entry).en
}

const placeholders = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',')

let failed = false
function check(label, keys, dictionary) {
  const missing = []
  const mismatched = []
  for (const [key, files] of keys) {
    if (!(key in dictionary)) missing.push([key, files[0]])
    else if (placeholders(key) !== placeholders(dictionary[key])) mismatched.push([key, dictionary[key]])
  }
  console.log(`${label}: ${keys.size} keys, ${missing.length} missing, ${mismatched.length} placeholder mismatches`)
  for (const [key, file] of missing.slice(0, 40)) console.log(`  missing  ${JSON.stringify(key)}  (${file})`)
  for (const [key, value] of mismatched.slice(0, 40)) console.log(`  mismatch ${JSON.stringify(key)} => ${JSON.stringify(value)}`)
  if (missing.length || mismatched.length) failed = true
}

check('renderer', collectKeys(path.join(root, 'src'), ['t', 'tr']), loadDictionary(path.join(root, 'src/i18n/locales/en.ts')))
check('main', collectKeys(path.join(root, 'electron'), ['mt']), loadDictionary(path.join(root, 'electron/i18n/en.ts')))
process.exit(failed ? 1 : 0)
