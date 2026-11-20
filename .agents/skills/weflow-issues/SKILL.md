---
name: weflow-issues
description: 在 WeFlow 仓库建 issue、写 issue 与 PR 的正文和评论、加标签、建立依赖和 Development 关联、关闭 issue，以及在 GitHub Project「WeFlow」看板上添加条目、修改 Status 和 Priority 的做法和坑。建或关 issue、给 issue 或 PR 加标签与关联、更新看板、在 GitHub 上发评论时使用。
---

# Issue、标签与看板

什么情况要记 issue、看板各个 Status 和 Priority 的含义写在 [CLAUDE.md](../../../CLAUDE.md) 的「Issue 与看板」。标签、关联和看板需要维护者权限。

## 写正文和评论

- 用中文。裸链接两侧留空格，或写成 `[文字](url)`：GitHub 会把紧挨着的中文算进链接。
- 公开的内容只写汇总和结论，隐私的规则见 CLAUDE.md 的「隐私」。
- 工作过程的材料（审计结果、基线、调研、基准数据）写进 issue 评论或 PR 正文，长内容放在 `<details>` 里，不在仓库里新增文档文件。
- 优化、调研类的工作合并后，把试过但没采用的做法（每项的数字和原因）、测出的构成和测量方法评论到对应的 issue，下次接着做时不用重测。
- 在 Windows 上用 Python 生成标题或正文时，输出的行尾是 CRLF：读进 shell 变量之前先 `tr -d '\r'`，或让 Python 直接写文件，否则回车符会进到标题里。

## 标签

- 先 `gh label list` 确认标签名：`gh api repos/334456777/WeFlow/issues/<n>/labels -f "labels[]=<名>"` 遇到不存在的名字会静默新建一个标签。
- `gh pr edit --add-label` 会因 classic projects 停用而报错，PR 同样用上面的 `gh api`。
- 分类只用标签，不设 GitHub 的 Issue Types。

## 依赖与关联

- 依赖（blocked by）只能建在 issue 之间。PR 属于某个 issue（`Refs #N`、「#N 第二步」）时，用 GraphQL 的 `addCloseIssueReferences(input: {issueId, pullRequestIds})` 关联到 issue 的 Development：对已合并的 PR 也有效，不改变 issue 的状态，撤销用 `removeCloseIssueReferences`。每个 issue 最多 10 条手动关联。
- 叠层 PR 只关联第一层，上层从第一层往下看。判断 PR 是不是叠层看 gh stack 的数据（GraphQL `PullRequest.stack { number size }`、`stackEntry { position }`），不看 base 分支：合并后 base 会改回 `main`。
- 查依赖时，除了「依赖」「阻塞」「叠在」，还要查「第 N 步」「第 N 批」「第 N 层」「第 N 部分」这类写法。
- 长期任务 issue 不写进 PR 的关闭关键词或 Development；合并后用 `gh pr view <n> --json closingIssuesReferences` 核对没有误关。

## 关闭

- 关之前读代码确认：评论里写的「已实现」要指到真正起作用的那段代码。
- 父 issue 按它自己正文的要求判断，做完就关（`gh issue close <n> --reason completed`），评论写做完了什么、哪些子 issue 继续单独跟踪；不因下面还开着长期的子 issue 而留着它。
- 以 not planned 关闭的 issue，看板设为 `Not Planned`，不设 `Done`。

## 看板（Project 3）

- 加入看板：`gh project item-add 3 --owner 334456777 --url <issue 或 PR 的链接> --format json -q .id`，输出条目 id。
- 改一个条目：`gh project item-edit 3 --owner 334456777 --url <链接> --field Status --value "In Review"`，`Priority` 同样写法。
- 批量修改时不用 `--url`：它每次都在背后查一次条目，十几个条目后就触发次级限流（`gh api rate_limit` 仍显示有余量）。用 `item-add` 输出的条目 id 和下面的字段 id 修改，条目之间停几秒：

  ```sh
  gh project item-edit --project-id PVT_kwHOCbIY6c4Bl33L --id <条目 id> \
    --field-id PVTSSF_lAHOCbIY6c4Bl33Lzhki_ws --single-select-option-id <选项 id>
  ```

  | 字段 | id | 选项 |
  |---|---|---|
  | Status | `PVTSSF_lAHOCbIY6c4Bl33Lzhki_ws` | Backlog `d2a1912c`、Todo `c764d01b`、In Progress `5c390812`、In Review `15922e75`、Blocked `55065b91`、Done `0ab8fcb6`、Not Planned `8bd982a2` |
  | Priority | `PVTSSF_lAHOCbIY6c4Bl33LzhkjBqw` | P0 `4a76cddd`、P1 `298728dd`、P2 `85f548b7`、P3 `ba910511` |

- `gh project item-list` 的开销很大，需要时只调用一次，结果重复使用。阶段性收尾时用一次 `gh project item-list 3 --owner 334456777 --limit 600 --format json` 对照 issue 和 PR 的开关状态，以 GitHub 上的状态为准修正看板。
- 开始做某个 issue 时，把它和它的父 issue 一起设为 `In Progress`。
- 重写 Status 字段的选项会清空所有条目的 Status，选项的 id 也会变：改选项后重新查 id，并给每个条目重新设置。
