---
name: weflow-html-template
description: 修改 HTML 导出的页面模板（crates/weflow-core/src/export_msg/ 下的 template.html、list.html、moments.html、colours.css 和 build.mjs）时，在本地用 CI 的 Bun 版本生成嵌入程序的 <模板>.{zh,en}.html 副本并一起提交，以及换行符和 bot 提交的处理。改 HTML 导出的模板、配色或它们的构建脚本时使用。
---

# 改 HTML 导出模板

模板与副本的关系见 Wiki [消息导出#模板](https://github.com/334456777/WeFlow/wiki/消息导出#模板)。推送后 `html-template.yml` 会生成副本并提交到分支，见 [发布与 CI#工作流](https://github.com/334456777/WeFlow/wiki/发布与-CI#工作流)。

- 程序用 `include_str!` 嵌入副本：新增模板、或改了模板后要在本地编译 Rust 时，先生成副本，并和模板一起提交，免得 bot 再提交一次。
- 在仓库根目录用 `web/package.json` 的 `packageManager` 里的 Bun 版本生成，结果与 CI 逐字节相同：

  ```sh
  bun x bun@<版本> crates/weflow-core/src/export_msg/build.mjs
  ```

  更新的 Bun 生成的 CSS 不同（如 `--buncss-light` 一类的写法）。
- `git status` 里没改过的副本显示为已修改时，多半只是工作区的换行符不同：`build.mjs` 统一写 LF，`.gitattributes` 覆盖全部副本。用 `tr -d '\r'` 比对确认后，`git checkout -- <文件>` 还原。
- 只改模板的 PR 不运行 `ci.yml` 的代码检查；bot 推送副本后要批准运行，见 [weflow-ci](../weflow-ci/SKILL.md)。
