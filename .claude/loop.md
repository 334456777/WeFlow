# Bug 监控 loop

## 启动命令

```
/loop 监控issues出现Bug标签由你来修，收到其他Claude的Bug修复请求也由你来修然后通知对方
```

## 要求

- 监视 `334456777/WeFlow` 上带 `bug` 标签的未关闭 issue，出现就修。
- 其他 Claude 会话发来的 bug 修复请求，也由这个会话修，修完用 SendMessage 通知对方。
- CI 全部通过就自己合并（merge commit），删掉本地和远程分支，看板上的 PR 和 issue 都设为 Done。
- 节奏：
  - 内部每 5 分钟查一次：用后台 Bash 循环，不用 Monitor，因为 Monitor 最长 30 分钟就过期；
  - 外部保底每 50 分钟唤醒一次：ScheduleWakeup 设 3000 秒；
  - 修完一个 bug 后，重新启动后台检查。

## 范围

- 只修 bug。不是 bug 的 issue（增强、WebUI、优化等）留给其他会话。
- 看板只改自己负责的条目。
- 发版（打 tag）必须先问。

## 修复流程

- 在自己的 worktree 和分支（`fix/<scope>-<描述>`）上修，不在共用的工作副本里切换分支。
- 提交：
  - 提交信息和 PR 标题用 `type(scope):` 加中文描述；
  - PR 描述用中文，用 `fixed`、`closed` 关联 issue；
  - 加标签，加入看板并设为 In Review。
- 用户能看到的行为变了，先推送 Wiki，再合并 PR。
- 等 CI 跑完再汇报，不在 CI 还没出结果时结束回合。
- 合并后，主工作副本要 `git pull`。

## 同时有多个 bug

同一时间待修的 bug 不止一个时（多个会话发来请求，或扫描到多个带 `bug` 标签的 issue），每个 bug 开一个子代理去修，几个子代理并行。只有一个 bug 时照常自己修。

- 开子代理用 Agent，`isolation: "worktree"`，每个子代理有自己的 worktree，各用一个 `fix/<scope>-<描述>` 分支。
- 交给子代理的说明要写清楚：
  - issue 链接，或请求方说的原因和建议的修法；
  - 上面「修复流程」和「约束」的要求；
  - 做完的标准：补测试，推送分支，开 PR，加标签，加入看板设为 In Review，回报 PR 链接、改了哪些文件、是否改变了用户可见的行为。
- 子代理不合并 PR，也不改 Wiki。下面这些由本会话做：
  - 等 CI；
  - 需要时推送 Wiki；
  - 合并 PR，删掉分支和 worktree；
  - 把看板设为 Done；
  - 通知请求方。
- 两个 bug 要改同一批文件时不并行：先修一个，合并以后再开下一个子代理，避免冲突。
- 一次合并一个 PR：每合并一个就在主工作副本里 `git pull`。后面的 PR 和 main 冲突时，让对应的子代理 rebase 后重新推送，用 SendMessage 找到它。
- 所有子代理的 PR 都合并之后，再重新启动后台检查。

## 约束

- 回复用简体中文。
- 仓库、Project「WeFlow」和 Wiki 都是公开的：不上传聊天记录、wxid、隐私信息、本地路径、密钥、API Key。
- 真实微信数据只读，只报汇总。

## 后台检查命令

```bash
while true; do
  out=$(gh issue list -R 334456777/WeFlow --label bug --state open --json number,title --jq '.[] | "#\(.number) \(.title)"' 2>/dev/null) || out=""
  if [ -n "$out" ]; then echo "新 bug issue："; echo "$out"; break; fi
  sleep 300
done
```

用 TaskStop 停掉它以后，`sleep.exe 300` 会留下来，要另外结束这个进程。
