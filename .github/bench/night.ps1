<#
.SYNOPSIS
  Starts the night's run of bench.yml from the benchmark runner's computer (#1001); with -Register, sets itself up to
  run every day at 23:25 in Japan.

.DESCRIPTION
  Starts a run of bench.yml by hand on the default branch with no commits, which is the night's run: its queue job
  waits until 23:30 in Japan, then queues the PRs labeled bench and measures the queue (.github/scripts/bench_queue.py).
  A night's run that comes later that night leaves the runner alone.

  It uses the GitHub CLI as you are logged in to it (gh auth login), whose token is in your Windows credentials, so the
  task runs only while you are logged on to Windows: Task Scheduler does not start it otherwise. A night it does not
  run (you are not logged on, or the computer is off) is not made up for when you log on or the computer comes back;
  the queue waits for the next night. It starts nothing when gh is not there, holds no login, or when GitHub refuses
  gh's login. What each run did, an error that stopped it included, is appended to -Log, the only record of a run of
  the task: conhost --headless hands Task Scheduler 0 whatever the script exits with, so the task's last result tells
  only whether pwsh started at all.

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
  # wakes the computer from sleep; no -StartWhenAvailable, so a night missed is not run once the computer is back
  $settings = New-ScheduledTaskSettingsSet -WakeToRun -ExecutionTimeLimit (New-TimeSpan -Minutes 30)
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

try {
  # the task passes the gh it was registered with, which an update or another install may have moved
  if (-not $Gh -or -not (Test-Path -LiteralPath $Gh -PathType Leaf)) {
    Say "nothing started: no gh$(if ($Gh) { " at $Gh" })"
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
    $out = (& $Gh workflow run bench.yml --repo $Repository -f commits= 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -eq 0) {
      Say "started the night's run of bench.yml"
      exit 0
    }
    # a login GitHub refuses (expired or revoked) is refused again
    if ($out -match 'HTTP 401|Bad credentials') {
      Say "nothing started: GitHub refused gh's login ($out)"
      exit 1
    }
    Say "attempt $attempt failed: $out"
    if ($attempt -lt 6) { Start-Sleep -Seconds 30 }
  }
  exit 1
} catch {
  Say "nothing started: $_"
  exit 1
}
