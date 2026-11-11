#!/usr/bin/env bun
// Makes the HTML export's templates small, one copy of each for each language, which the program embeds. CI runs it
// when a template or this script changes on a branch, and commits what it writes to that branch
// (.github/workflows/html-template.yml).
//
//   template.html  the chat page (html.rs)                     -> template.zh.html, template.en.html
//   list.html      the list of sessions or contacts (list_html.rs) -> list.zh.html, list.en.html
//
// The .zh copy leaves the English words out (the pages' words are written in Chinese); the .en copy keeps them. Each
// copy: the CSS and the script minified by Bun, the comments and the space between the tags dropped. The three
// placeholders (__WF_LANG__, __WF_TITLE__, __WF_DATA__) stay as they are.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const HERE = import.meta.dir;
const TEMPLATES = ["template", "list"];

/** The template `source` made small for one language. */
async function minified(source, lang) {
  const t = lang === "en" ? source : source.replace(/\/\*EN\*\/[\s\S]*?\/\*EN\*\//, "{}");
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

for (const name of TEMPLATES) {
  const source = fs.readFileSync(path.join(HERE, name + ".html"), "utf8");
  for (const lang of ["zh", "en"]) {
    const t = await minified(source, lang);
    fs.writeFileSync(path.join(HERE, `${name}.${lang}.html`), t);
    console.log(`${name}.${lang}.html  ${(Buffer.byteLength(t) / 1024).toFixed(1)} KB`);
  }
}
