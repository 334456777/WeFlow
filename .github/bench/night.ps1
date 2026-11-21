<#
.SYNOPSIS
  Starts the night's run of bench.yml from the benchmark runner's computer (#1001); with -Register, sets itself up to
  run every day at 23:25 in Japan.

.DESCRIPTION
  GitHub starts a run of bench.yml's schedule hours late at times, or drops it (#957). This starts a run of bench.yml
  by hand on the default branch with no commits, which is a night's run as one of the schedule is: its queue job waits
  until 23:30 in Japan, then queues the PRs labeled bench and measures the queue (.github/scripts/bench_queue.py). The
  first night's run to reach the night takes the queue; one that comes later that night, of the schedule or not, leaves
  the runner alone.

  It uses the GitHub CLI as you are logged in to it (gh auth login), whose token is in your Windows credentials, so it
  runs only while you are logged on to Windows. It starts nothing when gh holds no login, or when it starts outside
  23:00 to 05:00 in Japan: a run missed while the computer was off starts once it is back, and only during the night
  is there something left to start. What each run did is appended to -Log.

.EXAMPLE
  .\night.ps1             # once, now
  .\night.ps1 -Register   # every day at 23:25 in Japan, while you are logged on, waking the computer
#>
param(
  [string] $Repository = '334456777/WeFlow',
  [string] $Log = (Join-Path $env:LOCALAPPDATA 'weflow-bench\night.log'),
  [string] $Gh = (Get-Command gh -ErrorAction SilentlyContinue).Source,
  [switch] $Register
)
$ErrorActionPreference = 'Stop'

if ($Register) {
  if (-not $Gh) { throw 'no gh on PATH: install the GitHub CLI and run gh auth login first' }
  # 23:25 in Japan, which keeps no summer time, in this computer's own time zone
  $at = [DateTime]::SpecifyKind([DateTime]::UtcNow.Date.AddHours(14).AddMinutes(25), 'Utc').ToLocalTime()
  # conhost --headless: no console window comes up over what you are doing at the time
  $run = "`"$((Get-Command pwsh).Source)`" -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`"" +
    " -Repository $Repository -Log `"$Log`" -Gh `"$Gh`""
  $action = New-ScheduledTaskAction -Execute (Join-Path $env:WINDIR 'System32\conhost.exe') -Argument "--headless $run"
  $trigger = New-ScheduledTaskTrigger -Daily -At $at
  # Interactive: only while you are logged on, the only time your Windows credentials, and gh's login, can be read
  $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive `
    -RunLevel Limited
  # wakes the computer from sleep; a run missed while it was off starts once it is back
  $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -WakeToRun -ExecutionTimeLimit (New-TimeSpan -Minutes 30)
  Register-ScheduledTask -TaskName 'WeFlow benchmark night run' -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force | Out-Null
  Write-Host "registered: WeFlow benchmark night run, every day at $($at.ToString('HH:mm')) (23:25 in Japan)"
  return
}

New-Item -ItemType Directory -Force (Split-Path $Log) | Out-Null
function Say([string] $line) {
  $stamped = "$(Get-Date -Format 'yyyy-MM-dd HH:mm') $line"
  Write-Host $stamped
  Add-Content -Path $Log -Value $stamped
}

# 23:00 to 05:00 in Japan: a night's run started then waits for 23:30 (bench.yml) or has the rest of the night
$now = [DateTime]::UtcNow.AddHours(9).TimeOfDay
if ($now -ge [TimeSpan]'05:00' -and $now -lt [TimeSpan]'23:00') {
  Say "nothing started: $($now.ToString('hh\:mm')) in Japan is outside 23:00-05:00"
  exit 0
}
if (-not $Gh) {
  Say 'nothing started: no gh'
  exit 1
}
# whether gh holds a login, without the network, which may not be up yet; the token itself goes nowhere
& $Gh auth token --hostname github.com *> $null
if ($LASTEXITCODE -ne 0) {
  Say 'nothing started: gh holds no login to github.com'
  exit 1
}

# no commits: the night's run; no prompt for the inputs
$env:GH_PROMPT_DISABLED = '1'
# the network may take a while to come back after the computer wakes
foreach ($attempt in 1..6) {
  $out = & $Gh workflow run bench.yml --repo $Repository -f commits= 2>&1 | Out-String
  if ($LASTEXITCODE -eq 0) {
    Say "started the night's run of bench.yml"
    exit 0
  }
  Say "attempt $attempt failed: $($out.Trim())"
  if ($attempt -lt 6) { Start-Sleep -Seconds 30 }
}
exit 1
