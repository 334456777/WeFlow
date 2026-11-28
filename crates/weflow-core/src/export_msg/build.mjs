#!/usr/bin/env bun
// 把 HTML 导出的模板压缩成程序嵌入的副本，每个模板每种语言一份。分支上的模板或这个脚本有改动时，CI 运行它，
// 并把写出的副本提交到该分支（.github/workflows/html-template.yml）。
//
//   模板           嵌入的模块       页面                   副本
//   template.html  html.rs          聊天记录页          -> template.zh.html、template.en.html
//   list.html      list_html.rs     会话或联系人的列表  -> list.zh.html、list.en.html
//   moments.html   moments_html.rs  朋友圈              -> moments.zh.html、moments.en.html
//
// colours.css 是所有页面共用的配色，替换每个模板里的 `@colours` 注释。.zh 副本去掉两个 `/*EN*/` 之间的英文译文
// （页面上的文字本来就用中文写），.en 副本保留。每份副本的 CSS 和脚本由 Bun 压缩，注释和标签之间的空白都去掉；
// 三个占位符（__WF_LANG__、__WF_TITLE__、__WF_DATA__）原样保留。

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const HERE = import.meta.dir;
const TEMPLATES = ["template", "list", "moments"];
// 不论检出时写成哪种换行符（core.autocrlf），都按 LF 读取，各处生成的副本才相同
const read = file => fs.readFileSync(path.join(HERE, file), "utf8").replace(/\r\n?/g, "\n");
const COLOURS = read("colours.css").trim();

/** 填入配色后的模板 `name`。 */
function source(name) {
  const t = read(name + ".html");
  const parts = t.split("/* @colours */");
  if (parts.length !== 2) throw new Error(`${name}.html: the @colours comment is not there exactly once`);
  return parts[0] + COLOURS + parts[1];
}

/** 把填入配色后的模板 `source` 压缩成语言 `lang` 的副本。 */
async function minified(source, lang) {
  const t = lang === "en" ? source : source.replace(/\/\*EN\*\/[\s\S]*?\/\*EN\*\//, "{}");
  // Bun.build 只从文件压缩，先把 CSS 和脚本写进临时目录
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
  const page = source(name);
  for (const lang of ["zh", "en"]) {
    const t = await minified(page, lang);
    fs.writeFileSync(path.join(HERE, `${name}.${lang}.html`), t);
    console.log(`${name}.${lang}.html  ${(Buffer.byteLength(t) / 1024).toFixed(1)} KB`);
  }
}
