# Refactoring baseline and regression matrix

**English** | [简体中文](zh-CN/refactor-baseline.md)

This page serves the structural refactoring of #391 (#397, #398, #402 and the rest). It gives the behaviour and resource baseline before the changes, and the regression matrix every implementing PR runs: which scenarios, how to run them, what they expect, and where to look when one fails. The audit of the call paths and responsibilities is in [serve-architecture.md](serve-architecture.md). The method follows #16 (what to measure and how) and #257 (what a before-and-after comparison needs).

## What an implementing PR does

1. Runs the [regression matrix](#regression-matrix): the correctness tests on synthetic accounts run in CI by themselves and must all pass before merging.
2. A PR that touches performance or the requests made measures again under the conditions of the [baseline](#baseline-results) ([measuring performance](#measuring-performance)), compares with `--compare`, and puts the [comparison table](#comparison-template) in its description.
3. A PR that changes requests or answers on purpose says so and updates the expectations of the tests concerned (such as the request lists of `web/src/requests.test.tsx`).

## Synthetic data

Every correctness test runs on synthetic encrypted accounts, without real WeChat data:

| Used for | Source | Content |
|---|---|---|
| Rust end-to-end tests | `weflow_native::fixture` (`crates/weflow-native/src/fixture.rs`), built as the tests run | `Fixture::standard`: a private chat and a group over two message databases (cursor pages across them), an image, a zstd-compressed message; tests add Moments, media or a second account as they need |
| Measuring performance | `serve_bench`, in `<work dir>/data`, reused while its size options stay the same | a group of 200,000 messages over 4 message databases; 500 private chats of 30 messages; 3,000 contacts; 2,000 Moments posts; `.dat` files for the latest 1,000 images; the newest 50 messages missing from the search index |
| Looking at the page by hand | `crates/weflow-cli/examples/dev_account.rs`, built in `target/dev-account/` by `bun run dev` in `web/` | a few friends, a group with pictures, Moments and the 「图片测试」 chat |
| WebUI tests | fake backends in the tests (stand-ins for `fetch` and `EventSource`) | each test's own conversations, messages and accounts |

## Regression matrix

The Rust scenarios are in `crates/weflow-cli/tests/serve_api.rs`: a real `weflow serve` process, real HTTP requests, and where it helps the same command run directly to compare. The WebUI scenarios run with `bun test`. In CI, `ci.yml` runs `make ci` on Linux and macOS and every test on Windows; `web.yml` runs `bun test` when `web/` changes.

| Scenario | Input | Expected | Command |
|---|---|---|---|
| CLI and HTTP agree | `config`, `chat` and other commands through the terminal and through HTTP | the body is the `--json` output byte for byte; the status follows the error code and the exit code | `cargo test -p weflow-cli --test serve_api commands_answer_like_the_terminal account_commands_and_read_only_refusals` |
| `/api/` and the child process agree | 26 reads, with cursor pages across message databases | the same status, body and `x-weflow-*` headers from both routes | `… reads_run_in_process_with_the_same_answers` |
| … also when they fail | a required parameter missing, a wrong type, parameters that exclude each other, an unknown parameter, a refusal of the command itself, nothing found, a wrong method, no token or a wrong one | the same 400 / 200 / 405 / 401 from both routes; `/api/` refuses `std`, `return=file` and `Prefer: respond-async` (400) and answers 404 for commands it does not run | `… reads_fail_the_same_in_process_and_in_a_child` |
| Authentication and host | no token, a token in the header or the query, a `Host` that is not this machine | the `/health` routes need no token, the rest answer 401; another `Host` is 403 | `… health_needs_no_token_and_everything_else_does only_requests_to_this_machine_are_answered`; unit test `tokens_are_checked_from_the_header_or_the_query` |
| Refusing writes to WeChat's databases | every command `/openapi.json` marks | all 501 `unsupported`, the set of marked commands fixed | `… refused_commands_are_marked_and_refused` |
| Switching accounts | two profiles on two synthetic accounts, switched 4 times through `PUT /config/set`, the profile names as long as each other | read right after the switch, both routes answer only with the current account's conversations and messages | `… reads_follow_the_profile_right_after_it_changes`; `health_data_changes_with_the_configuration_and_each_run` |
| Long tasks and cancelling | an AI request that hangs, as a job; cancelled queued and running, cancelled twice; a client that goes away | a cancel kills the child at once; a second cancel only forgets the job, its result unchanged; a synchronous request that goes away ends its command too | `… jobs_are_cancelled_and_limited a_job_is_cancelled_once_and_then_forgotten` |
| Progress and SSE | `backup create` as a job; reconnecting with `Last-Event-ID`; the server stopping | `status`, `progress`, then `result`; a reconnect gets only the later events; `bye` when the server stops | `… jobs_run_in_the_background event_streams_say_bye_when_the_server_stops health_events_say_hello_then_beat_without_a_token` |
| Message push | a new message written while listened to | one `message.new` | `… new_messages_are_pushed_as_events` |
| Media and files | images, voices, videos, Moments media; `Range`, `ETag`; `return=file` | the file itself, the right parts, `304`; a job keeps the file it wrote | `… media_come_back_as_files_from_the_process videos_are_sent_in_the_parts_asked_for moments_media_come_from_the_cache_and_only_from_wechat files_come_back_with_the_result jobs_keep_the_file_they_wrote_for_the_browser` |
| Limits | beyond `--max-tasks` and `--max-reads` | a task gets 429 with `Retry-After: 5`; `/api/` reads wait in line | `… jobs_are_cancelled_and_limited`; unit test `requests_wait_in_line_until_it_is_full_or_too_long` |
| Requests of the WebUI's chat page | start, another conversation, back, a search, another account | each step's list of requests is the baseline's; after another account only its records show | `cd web && bun test src/requests.test.tsx` |
| A late answer of the old account in the WebUI | a request made before the account changed comes back after | the old answer does not reach the page | same |
| WebUI flows | navigation, conversations and messages, search, export jobs (progress, cancel, save), contacts, settings, token, account setup, Moments, WeChat Pay records, theme, reduced motion | see each test | `cd web && bun test` (`src/App.test.tsx` and the rest) |

**When one fails**: the name of the test is the scenario of the table; the assertion messages carry the request (such as `GET /chat/messages?…`) and the round. A Rust test keeps the server's stderr in `Server::stderr`, which an assertion can print; to see each request on the server's side, start it with `--log-requests`. When a WebUI list of requests differs, the assertion shows both lists in full.

## Measuring performance

### Tools

| Tool | Measures | Command |
|---|---|---|
| `serve_bench` (`crates/weflow-cli/examples/serve_bench.rs`) | end to end: each read through both routes (median, p95, min, max, n), answer sizes, concurrency, images and videos, the push engine's CPU, and the CPU and peak memory of the server process over the whole run. Always on both a synthetic account (answers checked) and a real one (speed) | `cargo build --release -p weflow-cli --bin weflow --example serve_bench`, then `target/release/examples/serve_bench <work dir> --home <WEFLOW_HOME> --save base.json`; after a change, add `--compare base.json` |
| `clap_round_trip` (an ignored test in `crates/weflow-cli/src/serve_api/in_process.rs`) | what an `/api/` read spends writing its command line, cloning the command tree and parsing it with clap | `cargo test --release -p weflow-cli --bin weflow clap_round_trip -- --ignored --nocapture` |
| `export_pipeline_probe`, `cursor_bench` | the export pipeline and cursors (the message and export paths of #257) | see [native-cli.md](native-cli.md#tests) |
| `web/src/requests.test.tsx` | the number and order of the WebUI's requests per step | `cd web && bun test src/requests.test.tsx` |

### Conditions

- Release builds of two named commits, before and after, on the same machine, toolchain (`rust-toolchain.toml`), data and options.
- At least 10 runs of each read (`--runs`, 10 by default), reported as median, p95, min, max and n; rows measured once (the start, the first read after it) say `once`.
- **Cold and warm**: `serve_bench` starts a new server each time, so "first read after start" and "first conversation opened after start" are cold-process numbers; the rest are warm, steady ones. The operating system's page cache is not emptied: cold means a new process, not a cold disk.
- **Resources**: the CPU time and peak memory of the server process (peak working set on Windows, `VmHWM` on Linux; not reported on macOS). The child processes of the command routes are not counted. On Windows the peak working set includes the pages of mimalloc's reserved arena that were used.
- **Noise**: `--compare` calls a read slower only when its median is more than 2 times (`--tolerance`) **and** more than 20 ms slower; smaller changes are not conclusive. CI runs no performance measurement and has no performance threshold.
- Results of a real account are published as aggregate numbers only: no conversation ids, names, messages or paths.

## Baseline results

Baseline commit `a85f867` (`main` after #407; this PR adds tests and measurements only and does not change the server). Windows 11 Pro (build 26300), Intel Core i7-10700F (16 logical CPUs), 64 GB of memory, NVMe SSD, Rust 1.99.1, release build (thin LTO, `codegen-units = 1`, mimalloc), `serve` with its defaults (`--max-tasks 1`, `--max-reads` and `--max-media` 16). The full results of `serve_bench` are in [`docs/baselines/serve-bench-a85f867-windows.json`](baselines/serve-bench-a85f867-windows.json), ready to be passed to `--compare`.

### Correctness

- Synthetic account: all 22 checks of `serve_bench` passed; a new message reached the listener after 1,440 ms.
- Regression matrix: all passed on Windows and on Linux (WSL); the 228 WebUI tests pass in order and shuffled (`--randomize`).

### Latency (real account, milliseconds, as the client sees them)

The biggest conversation of the real account holds 234,513 messages. Medians, with min–max in brackets; n = 10 each.

| Read | Child process route | `/api/` |
|---|---:|---:|
| Conversations, 50 | 48.5 (47.9–123.1) | 10.7 (10.5–14.1) |
| Messages, first page | 38.8 (38.1–46.5) | 3.8 (3.6–4.3) |
| Messages, a page in the middle, by `offset` | 2,390 (2,378–2,491) | 2,230 (2,127–2,401) |
| Messages, a page in the middle, from a cursor | 39.3 (38.1–40.4) | 3.0 (3.0–3.4) |
| Latest 20 (polling) | 39.2 (38.6–118.0) | 3.2 (2.9–3.5) |
| One contact | 31.8 (31.2–38.2) | 1.3 (1.3–3.1) |
| All contacts | 41.3 (40.7–59.5) | 7.6 (7.0–18.4) |
| Search in the conversation | 44.1 (43.0–46.3) | 4.2 (4.1–4.6) |
| Search in the conversation, a word found nowhere | 177.4 (168.2–245.7) | 66.7 (66.2–139.5) |
| Group members | 41.9 (39.9–44.0) | 1.5 (1.4–10.9) |
| Messages per day | 657.4 (654.5–680.8) | 68.7 (67.7–71.1) |
| Moments, 20 posts | 245.3 (241.3–264.9) | 6.4 (6.1–213.1) |
| 16 pages of messages at once (all of them) | 177.6 (163.8–188.7) | 99.1 (94.9–105.2) |
| An image, each the first time (n = 20) | 53.5 (52.7–117.8) | 6.5 (4.3–31.3) |
| 20 images at once | 52.9, only 10 of 200 answered (the rest 429) | 12.5, all 200 answered |

Reads the WebUI still makes through a child process (they have only that route): conversation statuses (60) 30.7, tab counts 30.4, group list 35.0, `analytics overall` (cached) 29.6, `analytics rankings` 34.8, `analytics time` 30.0. Their time is almost all the fixed cost of starting a process and opening the databases: the part that [serve-architecture.md](serve-architecture.md#appendix-cost-of-the-clap-round-trip) says gains most from running in process.

**Cold start** (once each): 36.6 ms from starting the server until it listens; 391.9 ms for the first read of the conversations after it (the database keys are derived first); then 6.9 ms to open the first conversation.

### Resources

- Over the whole run on the real account (every read and concurrent request of the table above and of the full results), the server process used 46.8 s of CPU, with a peak working set of 1,170 MiB; child processes not counted.
- The push engine uses 7.8 ms of CPU a second while listened to, none otherwise.

### The clap round trip

`clap_round_trip`: about 96 µs per `/api/` read to write the command line and parse it, about 60 µs of it cloning the command tree (the mean of 2,000 runs, the same in three rounds).

### WebUI requests

`web/src/requests.test.tsx` pins the requests of each step on the chat page:

| Step | Requests |
|---|---|
| Start | 1 event stream (`/api/push/messages`) + 5: `/jobs` (the token check), `/config/profiles`, the conversations, their statuses, the first conversation's messages |
| Another conversation | 1: its messages |
| Back to a conversation opened before | 0 (kept) |
| A search in the conversation | 1 |
| Another account (the `data` of `/health/events` changed) | 6: first the messages of the conversation open before, from the **old** account, asked of the new one (needless, for #402), then `/jobs`, `/config/profiles`, the conversations, their statuses, the messages of the new account's first conversation |

An answer of the old account asked for before the switch and coming back after it does not reach the page (the scopes of `api/scope.ts` at work).

## Comparison template

An implementing PR copies this table and fills it in under the same conditions:

```markdown
| Scenario | Route | Baseline median (min–max, n) | After median (min–max, n) | Ratio | Note |
|---|---|---:|---:|---:|---|
| sessions, 50 | api | | | | |
| messages, first page | api | | | | |
| … | | | | | |
| Server process CPU / peak memory | - | | | | |
| WebUI requests (start / another conversation / another account) | - | 6 / 1 / 6 | | | |
```

**Acceptance**:

- The regression matrix passes; intended changes of interfaces or requests are explained in the PR, with the current consumers and the test expectations updated.
- No slowdown flagged by `--compare`; one that is flagged must be explained (correctness bought with it, for instance), or the PR is not merged.
- A clear rise in CPU or peak memory against the baseline must be explained.
- A structural change (such as removing the clap round trip) is not claimed as a speed-up unless its numbers are beyond the noise.
