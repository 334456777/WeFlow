# Troubleshooting automatic WeChat key retrieval on macOS

**English** | [简体中文](zh-CN/MAC-KEY-FAQ.md)

If WeFlow fails to obtain the WeChat database key automatically on macOS, this guide may help.

### Stop retrying in a row

When you see any of the following errors, pause and do not keep clicking the retrieve button:

- SCAN_FAILED, usually accompanied by "No suitable module found" or "Sink pattern not found"
- HOOK_FAILED or "Native Hook Failed"
- patch_breakpoint_failed
- thread_get_state_failed

Current macOS and WeChat protections are very sensitive. Repeated attempts do not fix the problem; they are more likely to be treated as abnormal behavior and trigger WeChat's safe mode or system-level memory protection.

### Suggested procedure

Based on feedback from many community users, if retrieval has already failed, the following steps in order usually solve it:

1. **Downgrade WeChat.** Use an older version that the community has verified to be more compatible. Currently 4.1.7.57 or 4.1.8.100 are recommended first.
2. **Quit WeChat completely.** Use Command + Q or end the process in Activity Monitor; closing the window is not enough.
3. **Restart your Mac.** This step is essential and must be a real restart. Logging out or sleep/wake does not clear the low-level blocking state.
4. **Reopen WeChat.** Click around a little to keep it in the foreground, and make sure it is **not logged in**.
5. **Go back to WeFlow.** Try "Get key automatically" **once only**.
6. **Enter your password and log in.** First enter your system password in the dialog; once the page confirms that login is allowed, log in to WeChat.
7. **Return to normal use.** As soon as you have the key, you can update WeChat back to the latest version you normally use.

### Common errors and what to do

**SCAN_FAILED: No suitable module found**
WeChat's memory layout is non-standard, or the target module was not hit. Make sure WeChat has fully started and stays in the foreground. If that does not help, go through the full "downgrade, restart the computer, retrieve, upgrade again" procedure above.

**SCAN_FAILED: Sink pattern not found**
WeFlow does not yet support the signature of the WeChat version you are using. The quickest fix is to downgrade to WeChat 4.1.7 or 4.1.8.100 and try again.

**patch_breakpoint_failed or thread_get_state_failed**
These errors are mostly caused by macOS security mechanisms blocking breakpoint injection or thread state reads. Further attempts are pointless: quit WeChat completely, restart your computer and try again.

**task_for_pid:5**
The system denied permission to attach to the process. Make sure you are using the packaged WeFlow.app, and check that code signing and debugging permissions are configured correctly.

### About the recommended versions

As of April 2026, community feedback shows that WeChat 4.1.7 and 4.1.8.100 are the most stable and have the highest success rate for key retrieval.

This does not mean newer versions can never work; it is only a reference for troubleshooting. WeFlow will gradually support the signatures of newer WeChat versions in future updates, so keep an eye on the project's releases.

### Final advice

After the first failure, find the cause first and never click "Get automatically" blindly several times in a row. If you already failed several times before reading this document, the best approach is to start from zero: quit WeChat completely, restart the computer, then try again.

Finally, if none of the above works, save the complete error text, especially the English details after SCAN_FAILED or HOOK_FAILED, and submit it to [the issue](https://github.com/hicccc77/WeFlow/issues/745). It greatly speeds up locating and fixing compatibility problems.
