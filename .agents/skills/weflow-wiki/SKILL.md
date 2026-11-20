---
name: weflow-wiki
description: 修改 WeFlow 的 GitHub Wiki（wiki/ 目录里的独立 clone）：书面技术叙述、只写现状和结论的写法，写完后检查过程性说法的扫描命令（也适用于代码注释），只提交自己的改动、提交信息的格式、先推 Wiki 再合并引用它的 PR。改 Wiki 页面、代码改动需要同步 Wiki、写注释或文档后自查措辞时使用。
---

# 改 Wiki

Wiki 写什么、链接怎么写、哪些改动要同步哪一页，见 [CLAUDE.md](../../../CLAUDE.md) 的「Wiki」一节。

## 准备

- `wiki/` 是 `WeFlow.wiki.git`（分支 `master`）的 clone，不属于主仓库。没有时运行 `git clone https://github.com/334456777/WeFlow.wiki.git wiki`；改之前 `git -C wiki pull`。
- 它可能同时被多个会话使用：工作区里可能有别人改了还没提交的内容。

## 写法

- 用书面的技术叙述：写「无法读取」「尚未」「因此」，不写「读不了」「还没」「所以就」「点一下就」「请查看」这类口语和对话式的句子。
- 只写现状和结论：不写「原来」「以前」「不再」「已删除」「改为」「从 X 变为 Y」、尝试的经过和改动前后的测量对比，这些写进 issue 和 PR。逆向和调研的结论按「结论 → 依据 → 原因」写，不按尝试的先后叙述。
- 保留：「命令-API」#版本 的变更列表、「评估后不采用的方案」表、带微信版本号的实测记录（说明验证的范围）。
- 不写维护者的个人情况（作息、设备的使用状况）和逐个情况的推演，只写系统做什么。
- 页面结构变了同步重写 `_Sidebar.md`；小节标题不加序号。
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
- 先推送 Wiki，再合并引用新页面或新锚点的 PR。
- 发版时在 Wiki 的最新提交上打与版本同名的附注 tag，见 [weflow-release](../weflow-release/SKILL.md#发布)。
