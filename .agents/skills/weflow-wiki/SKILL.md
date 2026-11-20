---
name: weflow-wiki
description: 修改 WeFlow 的 GitHub Wiki（wiki/ 目录里的独立 clone）和两份 README：书面技术叙述、只写现状和结论的写法，写完后检查过程性说法的扫描命令（也适用于代码注释），只提交自己的改动、提交信息的格式，以及中英文 README 逐行同步。改 Wiki 页面或 README、代码改动需要同步 Wiki、写注释或文档后自查措辞时使用。
---

# 改 Wiki 和 README

Wiki 写什么、只写现状的规则、链接和页面结构的写法、哪些改动要同步哪一页、Wiki 与 PR 的先后，见 [CLAUDE.md](../../../CLAUDE.md) 的「Wiki」一节；不写维护者的个人情况见「隐私」。

## 准备

- 没有 `wiki/` 时运行 `git clone https://github.com/334456777/WeFlow.wiki.git wiki`；改之前 `git -C wiki pull`。
- 它可能同时被多个会话使用：工作区里可能有别人改了还没提交的内容。

## 写法

- 用书面的技术叙述：写「无法读取」「尚未」「因此」，不写「读不了」「还没」「所以就」「点一下就」「请查看」这类口语和对话式的句子。
- 逆向和调研的结论按「结论 → 依据 → 原因」写，不按尝试的先后叙述。
- 除了「命令-API」#版本 的变更列表，「评估后不采用的方案」表和带微信版本号的实测记录（说明验证的范围）也保留。
- 不写逐个情况的推演，只写系统做什么。
- 写完扫一遍新增的文字，代码注释和其他文档同样适用：

  ```sh
  rg -i "no longer|any more|anymore|used to|previously|the old |instead of|by hand|never" <文件>
  rg "不再|原来|改为|不手动|先.*再" <文件>
  ```

  解释设计取舍的 `instead of`（如「排队而不是返回 429」）可以保留。

## 提交和推送

- 只提交自己的文件。同一页上别人也有未提交的改动时，只暂存自己的那几行：准备一份只含自己改动的文件内容，`git hash-object -w <这份内容>` 写入对象库，再用 `git update-index --cacheinfo 100644,<哈希>,<页面>.md` 登记到索引。
- 提交信息用中文，以改动的页面名开头（如「朋友圈：HTML 页面；消息导出：…」），不用 Conventional Commits。
- 推送前用 `git log origin/master..master` 看清会一起推上去哪些提交，其中可能有别人尚未推送的提交。
- 发版时在 Wiki 的最新提交上打与版本同名的附注 tag，见 [weflow-release](../weflow-release/SKILL.md#发布)。

## README

README 和 Wiki 的分工见 [CLAUDE.md](../../../CLAUDE.md) 的「Wiki」与「其他」：README 是快速介绍，详细的步骤和原因写在 Wiki，README 链接过去。README 在主仓库里，跟 PR 走，不在 `wiki/` 里提交。

- 先改中文版 `README.zh-CN.md`，再把同样的改动译进 `README.md`。两份逐行对应：同一个标题、列表项、代码块和表格行在两份文件的同一行，改完用下面的命令核对，没有输出才算对齐：

  ```sh
  diff <(grep -n '^\(#\|```\|- \|[0-9]\+\. \||\)' README.zh-CN.md | cut -d: -f1) \
       <(grep -n '^\(#\|```\|- \|[0-9]\+\. \||\)' README.md | cut -d: -f1)
  ```

- 命令的示例输出照实际运行的结果写，wxid、路径和密钥换成占位符。
- 图片放在 `docs/images/`。
- 改完运行 `make docs-check`：检查相对链接和锚点，以及汉字旁的逗号是否为中文逗号。
