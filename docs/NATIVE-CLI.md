# Native CLI (`weflow`)

A Rust command-line build of WeFlow's backend. Every command prints one JSON document on stdout
(`{"success": true, "data": ...}` or `{"success": false, "error": {...}}`); progress goes to stderr with `--progress`.

## Language

English is the default. Chinese is selected only from the environment, in this order
(the first variable that is set and non-empty decides):

`WEFLOW_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `LANGUAGE`

A value starting with `zh` (`zh_CN.UTF-8`, `zh-TW`, `zh`) gives Chinese; anything else, including `C` and `POSIX`, gives English.
`--lang en|zh` overrides the environment for a single run.

The language affects generated text: TXT/Excel export labels (`[Image]` / `[图片]`), the default official-account payment
merchant name, and the default AI insight prompt. JSON keys, error codes and `--help` text are always English.

## Commands

```
weflow config   list | get | set | unset | clear | import
weflow db       detect | scan <root> | test | open
weflow key      db | image | scan-image <user-dir>
weflow chat     sessions | messages | latest | search | contacts | contact | update-message | delete-message
                anti-revoke check|install|uninstall | voice | emoji
weflow export   sessions | contacts | footprint | media | messages
weflow analytics overall | rankings | time | excluded
weflow group    list | members | ranking | hours | media | member | export-members
weflow report   annual years|generate | dual generate
weflow sns      timeline | users | stats | export | download-image | block-delete | delete
weflow biz      accounts | messages | pay-records
weflow insight  test | records | get | mark-read | clear | trigger | footprint
weflow backup   create | inspect | restore
weflow serve    --http --message-push --insight --image-auto-download
weflow runtime  info | manifest
```

Exit codes: `0` ok, `1` runtime error, `2` bad arguments, `3` config/key error, `4` database/native library error.
