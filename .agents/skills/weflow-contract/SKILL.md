---
name: weflow-contract
description: 修改命令 API 的契约之后要做的事：重新生成 web/contract 的快照和 WebUI 的类型、跑完整的契约测试，以及 schemars 与生成器的坑、总被拒绝的命令、serve 集成测试的写法。改 weflow-core 的 contract/ 类型或它们的文档注释、加响应字段或路由、改 serve_api 时使用。
---

# 改命令 API 的契约

流程和各项测试见 Wiki [服务端实现#类型化契约](https://github.com/334456777/WeFlow/wiki/服务端实现#类型化契约)；要不要提升 `API_VERSION` 见 [CLAUDE.md](../../../CLAUDE.md) 的「版本与 tag」。

## 步骤

1. 改了类型之后（哪怕只加一个可选的响应字段，或只改文档注释）重写快照和样本，再生成网页的类型：

   ```sh
   WEFLOW_UPDATE_CONTRACT=1 cargo test -p weflow-cli --bin weflow contract_
   cd web && bun run contract && bun run typecheck
   ```

   本机的 Bun 比 `web/package.json` 的 `packageManager` 新时，用那个版本生成：`bun x bun@<版本> scripts/contract.ts`。
2. 跑整个 `cargo test -p weflow-cli --bin weflow`：`every_answer_is_described` 等检查不以 `contract_` 开头，只跑子集会漏掉 `contract_snapshot_is_current` 之类的失败。
3. 重新生成后 `git status` 显示文件改了而 `git diff` 为空，或 `samples.json` 只有换行符的变化：用 `git checkout -- <文件>` 还原。

## 坑

- 展开（`#[serde(flatten)]`）的 `Option` 或被跳过的字段：`left_out_not_null` 会把 `null` 从 schema 里去掉，回复里真有 `null` 时校验失败。拆成明确的类型，例如 untagged enum `MessagesBundle { Folder, Zip }`。
- `web/scripts/contract.ts` 不认 `allOf`：直接用 `of(generator.subschema_for::<T>())` 登记。
- 给契约枚举的成员写 `///` 后，schemars 把 `"enum": [...]` 改写成 `oneOf`，每项带 `const` 和 `description`；生成的 TS 联合类型不变（按声明顺序重排），`contract-compat.ts` 不报。`#[serde(flatten)]` 字段和 `$ref` 字段上的文档会作为 `description` 与 `$ref` 并列。
- 总被只读层拒绝的命令加进 `crates/weflow-cli/src/serve_api/rules.rs` 的 `REFUSED`，不要硬给它描述 `data`；`every_answer_is_described` 的例外只有 `chat contact` 和 `config get`。`left_out_not_null`、`every_answer_is_described`、`contract_snapshot_is_current` 都在 `serve_api/openapi.rs`。
- 原生层的回复读进契约类型时用 `deny_unknown_fields`，从 `Value` 里 `take` 出来，不 clone。
- serve 的集成测试：GET 路由不读请求体，参数写进查询串；用 `crates/weflow-cli/tests/serve_api/common.rs` 的 `misfits(&contract, &json!({"$ref": …}), &data)` 对照契约。
