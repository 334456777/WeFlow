#!/usr/bin/env bun
// Makes the HTML export's template small: template.html, one copy for each language, which the program embeds
// (html.rs). CI runs it when template.html or this script changes on a branch, and commits what it writes to that
// branch (.github/workflows/html-template.yml): the copies are made there, not by hand.
//
//   template.zh.html  the English words left out (the page's words are written in Chinese)
//   template.en.html  with them
//
// Each copy: the CSS and the script minified by Bun, the comments and the space between the tags dropped. The three
// placeholders (__WF_LANG__, __WF_TITLE__, __WF_DATA__) stay as they are. Other scripts import `minified` to make the
// same copy of a template.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const HERE = import.meta.dir;
export const SOURCE = path.join(HERE, "template.html");
export const LANGS = ["zh", "en"];

/** The template at `source` made small for one language. */
export async function minified(lang, source = SOURCE) {
  let t = fs.readFileSync(source, "utf8");
  if (lang !== "en") t = t.replace(/\/\*EN\*\/[\s\S]*?\/\*EN\*\//, "{}");
  // Bun minifies from files
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "weflow-template-"));
  try {
    const min = async (ext, code) => {
      const file = path.join(work, "part." + ext);
      fs.writeFileSync(file, code);
      const r = await Bun.build({ entrypoints: [file], minify: true });
      if (!r.success) throw new Error(r.logs.join("\n"));
      return (await r.outputs[0].text()).trim();
    };
    const style = /<style>([\s\S]*?)<\/style>/, script = /<script>([\s\S]*?)<\/script>/;
    const css = await min("css", t.match(style)[1]), js = await min("js", t.match(script)[1]);
    const [head, rest] = t.split(t.match(style)[0]), [body, tail] = rest.split(t.match(script)[0]);
    const tight = s => s.replace(/<!--[\s\S]*?-->/g, "").replace(/>\s+</g, "><").replace(/\n\s*/g, "");
    const out = tight(head) + "<style>" + css + "</style>" + tight(body) + "<script>" + js + "</script>" + tight(tail) + "\n";
    for (const p of ["__WF_LANG__", "__WF_TITLE__", "__WF_DATA__"]) if (out.split(p).length !== 2) throw new Error(`${lang}: ${p} is not there exactly once`);
    return out;
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  for (const lang of LANGS) {
    const t = await minified(lang);
    fs.writeFileSync(path.join(HERE, `template.${lang}.html`), t);
    console.log(`template.${lang}.html  ${(Buffer.byteLength(t) / 1024).toFixed(1)} KB`);
  }
}
