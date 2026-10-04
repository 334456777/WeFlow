#!/usr/bin/env node
/**
 * Shuffles the <img> order inside the image gallery of the READMEs.
 *
 * GitHub strips <script>, <style> and `class` from Markdown, so the browser cannot
 * reshuffle the gallery on its own; run this script (or a scheduled workflow) to get
 * a new order. The gallery is a plain <table> of <img> cells for that reason.
 *
 *   node scripts/shuffle-gallery.cjs                    # random order
 *   node scripts/shuffle-gallery.cjs --seed 42          # reproducible order
 *   node scripts/shuffle-gallery.cjs --dry-run          # print the new order, write nothing
 *   node scripts/shuffle-gallery.cjs docs/zh-CN/README.md
 */
const fs = require('fs')
const path = require('path')

const root = path.resolve(__dirname, '..')
const READMES = ['README.md', 'docs/README.md', 'docs/zh-CN/README.md']
/** Gallery containers, tried in order; the first one holding >= 2 images wins. */
const SECTIONS = [
  ['<table>', '</table>'],
  ['<div class="image-grid">', '</div>'],
]
// A line owning a whole cell, e.g. `    <td><img src="…" alt="…" width="150"></td>`,
// so shuffling the line keeps the surrounding layout intact.
const IMG_LINE = /^[ \t]*.*<img\b[^\n]*$/gm
const USAGE = [
  'Usage: node scripts/shuffle-gallery.cjs [options] [files...]',
  '',
  'Shuffles the <img> order of the README image gallery (a <table> of images).',
  'Without files it scans README.md and docs/**/README.md.',
  '',
  '  --seed <number>  use a reproducible order',
  '  --dry-run        print the new order without writing',
  '  -h, --help       show this help',
].join('\n')

/** Locate the gallery body, i.e. the container that actually holds the images. */
function findGallery(source) {
  let fallback = null
  for (const [open, close] of SECTIONS) {
    const start = source.indexOf(open)
    if (start === -1) continue
    const end = source.indexOf(close, start + open.length)
    if (end === -1) continue
    const body = source.slice(start, end)
    const found = body.match(IMG_LINE)
    if (fallback === null) fallback = { start, end, body }
    if (found && found.length >= 2) return { start, end, body }
  }
  return fallback
}

/** Mulberry32: tiny deterministic PRNG, used only when --seed is given. */
function seededRandom(seed) {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function shuffle(list, random) {
  for (let i = list.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1))
    const tmp = list[i]
    list[i] = list[j]
    list[j] = tmp
  }
  return list
}

/** Human readable label for a dry run, e.g. `GPU` from alt="GPU". */
function label(line) {
  const alt = /alt="([^"]*)"/.exec(line)
  if (alt && alt[1]) return alt[1]
  const src = /src="([^"]*)"/.exec(line)
  return src ? path.basename(src[1]) : line.trim()
}

function shuffleFile(file, random, dryRun, quiet) {
  const shown = path.relative(root, file) || file
  const source = fs.readFileSync(file, 'utf8')
  const gallery = findGallery(source)
  if (!gallery) {
    if (!quiet) console.warn(`skip ${shown}: no gallery container found`)
    return 0
  }
  const { start, end, body } = gallery

  const images = body.match(IMG_LINE)
  if (!images || images.length < 2) {
    if (!quiet) console.warn(`skip ${shown}: gallery has fewer than 2 images`)
    return 0
  }

  const order = shuffle(images.slice(), random)
  if (dryRun) {
    console.log(`${shown}: ${order.map(label).join(' ')}`)
    return order.length
  }

  let next = 0
  const rebuilt = body.replace(IMG_LINE, () => order[next++])
  fs.writeFileSync(file, source.slice(0, start) + rebuilt + source.slice(end))
  console.log(`${shown}: shuffled ${order.length} images -> ${order.map(label).join(' ')}`)
  return order.length
}

function main() {
  const args = process.argv.slice(2)
  let dryRun = false
  let seed = null
  const files = []

  for (let i = 0; i < args.length; i++) {
    const arg = args[i]
    if (arg === '--dry-run') dryRun = true
    else if (arg === '--seed') seed = Number(args[++i])
    else if (arg.startsWith('--seed=')) seed = Number(arg.slice('--seed='.length))
    else if (arg === '-h' || arg === '--help') {
      console.log(USAGE)
      return
    } else if (arg.startsWith('-')) {
      console.error(`unknown option: ${arg}`)
      process.exitCode = 2
      return
    } else files.push(arg)
  }

  if (seed !== null && !Number.isFinite(seed)) {
    console.error('--seed expects a number')
    process.exitCode = 2
    return
  }

  const random = seed === null ? Math.random : seededRandom(seed)
  const explicit = files.length > 0
  const targets = explicit ? files : READMES.filter((file) => fs.existsSync(path.resolve(root, file)))
  let total = 0
  for (const file of targets) {
    const abs = path.resolve(root, file)
    if (!fs.existsSync(abs)) {
      console.warn(`skip ${file}: not found`)
      continue
    }
    total += shuffleFile(abs, random, dryRun, !explicit)
  }

  if (total === 0) {
    console.error('no gallery images were shuffled')
    process.exitCode = 1
  }
}

main()
