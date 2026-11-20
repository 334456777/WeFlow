#!/usr/bin/env bash
# Stop hook: a desktop notification each time Claude Code finishes a turn in this repository.
#
# Claude Code passes the hook's input as JSON on stdin. The title names the project folder; the text is the first line
# of Claude's last reply when jq can read it (from the input, or from the end of the transcript), or a fixed line
# otherwise. Windows (Git Bash) and WSL show a toast through Windows PowerShell, macOS uses osascript and Linux
# notify-send; where none of them is available (a cloud container, a server) nothing happens. It prints nothing and
# always exits 0, so it never holds up or stops a turn. WEFLOW_NO_STOP_NOTIFY=1 turns it off.
set -u

[ -n "${WEFLOW_NO_STOP_NOTIFY:-}" ] && exit 0
input=$(cat)

title="Claude Code · $(basename "${CLAUDE_PROJECT_DIR:-$PWD}")"
body=
if command -v jq > /dev/null 2>&1; then
  # The first non-empty line without Markdown emphasis or leading heading, quote and list marks, at most 120 characters.
  first_line='split("\n") | map(gsub("[*`]"; "") | sub("^\\s*(#{1,6}\\s+|>\\s*|-\\s+|\\|\\s*)*"; ""))
    | map(select(length > 0)) | (.[0] // "") | .[0:120]'
  body=$(jq -r "(.last_assistant_message // \"\") | $first_line" <<< "$input" 2> /dev/null)
  if [ -z "$body" ]; then
    transcript=$(jq -r '.transcript_path // empty' <<< "$input" 2> /dev/null)
    if [ -n "$transcript" ] && [ -f "$transcript" ]; then
      # The transcript has one JSON object per line; the last reply is near its end.
      body=$(tail -n 400 "$transcript" | jq -rs "[.[] | select(.type == \"assistant\") | .message.content[]?
        | select(.type == \"text\") | .text] | (last // \"\") | $first_line" 2> /dev/null)
    fi
  fi
fi
[ -n "$body" ] || body="回合结束"

# A Windows toast through Windows PowerShell, which every Windows 10 and 11 has. The text goes in environment variables
# and is escaped for XML there, so no quoting reaches the command line.
windows_toast() {
  WEFLOW_NOTIFY_TITLE=$title WEFLOW_NOTIFY_BODY=$body powershell.exe -NoProfile -NonInteractive -Command '
    $null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
    $null = [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
    $t = [Security.SecurityElement]::Escape($env:WEFLOW_NOTIFY_TITLE)
    $b = [Security.SecurityElement]::Escape($env:WEFLOW_NOTIFY_BODY)
    $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
    $xml.LoadXml("<toast><visual><binding template=""ToastGeneric""><text>$t</text><text>$b</text></binding></visual></toast>")
    $app = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe"
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show(
      [Windows.UI.Notifications.ToastNotification]::new($xml))
  ' > /dev/null 2>&1
}

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    windows_toast
    ;;
  Darwin)
    osascript -e 'on run argv' -e 'display notification (item 2 of argv) with title (item 1 of argv)' -e 'end run' \
      "$title" "$body" > /dev/null 2>&1
    ;;
  *)
    if command -v notify-send > /dev/null 2>&1; then
      notify-send "$title" "$body" > /dev/null 2>&1
    elif command -v powershell.exe > /dev/null 2>&1; then
      windows_toast
    fi
    ;;
esac
exit 0
