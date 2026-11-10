<#
.SYNOPSIS
  Calibrates the benchmark account's profile on a real account (#605); with -Register, sets itself up to run every
  four weeks. Run as the user whose WeFlow is configured on the real account, never as the runner's account.

.DESCRIPTION
  Builds account_profile_probe from the newest main in a clone of its own (-BenchDir\calibrate), then runs it on the
  WeFlow config with --calibrate: the profile (shares and sizes only) replaces -BenchDir\profile\profile.json when a
  part of it moved past the probe's thresholds, and nothing of the account is printed, only whether it did, how many
  parts moved and the profile's hash. The config is only read; the key in it never leaves it. bench.yml builds the
  benchmark account from that profile from its next run on: a new profile is a new environment, whose results are
  compared among themselves, while the older ones stay as they were.

  -BenchDir\profile is readable by the runner's account (-RunnerAccount), writable by you only. What each run printed
  is appended to -BenchDir\profile\calibrate.log.

.EXAMPLE
  .\calibrate.ps1                 # once, now
  .\calibrate.ps1 -Register       # every four weeks, Sunday 02:40, whether you are logged on or not
#>
param(
  [string] $BenchDir = 'E:\weflow-bench',
  [string] $Config = (Join-Path ($env:WEFLOW_HOME ? $env:WEFLOW_HOME : (Join-Path $env:APPDATA 'weflow')) 'config.json'),
  [string] $RunnerAccount = 'weflow-bench',
  [string] $Repository = 'https://github.com/334456777/WeFlow.git',
  [switch] $Register
)
$ErrorActionPreference = 'Stop'

if ($Register) {
  $action = New-ScheduledTaskAction -Execute (Get-Command pwsh).Source `
    -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -BenchDir `"$BenchDir`" -Config `"$Config`""
  $trigger = New-ScheduledTaskTrigger -Weekly -WeeksInterval 4 -DaysOfWeek Sunday -At 02:40
  # S4U: runs whether you are logged on or not, without keeping your password
  $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType S4U -RunLevel Limited
  $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Hours 3)
  Register-ScheduledTask -TaskName 'WeFlow benchmark calibration' -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force | Out-Null
  Write-Host 'registered: WeFlow benchmark calibration, every four weeks on Sunday at 02:40'
  return
}

if (-not (Test-Path $Config)) { throw "no WeFlow config at $Config" }
$work = Join-Path $BenchDir 'calibrate'
$profileDir = Join-Path $BenchDir 'profile'
New-Item -ItemType Directory -Force $work, $profileDir | Out-Null
# yours to write, the runner's to read
icacls $profileDir /inheritance:r /grant:r "${env:USERNAME}:(OI)(CI)F" 'SYSTEM:(OI)(CI)F' `
  'Administrators:(OI)(CI)F' "${RunnerAccount}:(OI)(CI)RX" /Q | Out-Null
$log = Join-Path $profileDir 'calibrate.log'

function Say([string] $line) {
  $stamped = "$(Get-Date -Format 'yyyy-MM-dd HH:mm') $line"
  Write-Host $stamped
  Add-Content -Path $log -Value $stamped
}

$src = Join-Path $work 'src'
if (-not (Test-Path (Join-Path $src '.git'))) {
  git clone --quiet --depth 1 $Repository $src
} else {
  git -C $src fetch --quiet --depth 1 origin main
  git -C $src checkout --quiet --force FETCH_HEAD
}
if ($LASTEXITCODE) { throw 'could not get main' }
$env:CARGO_TARGET_DIR = Join-Path $work 'target'
cargo build --quiet --release --manifest-path (Join-Path $src 'Cargo.toml') -p weflow-core --example account_profile_probe
if ($LASTEXITCODE) { throw 'could not build account_profile_probe' }
$probe = Join-Path $env:CARGO_TARGET_DIR 'release\examples\account_profile_probe.exe'
$out = & $probe $Config --calibrate (Join-Path $profileDir 'profile.json') 2>&1
$code = $LASTEXITCODE
# what the probe says in calibrate mode carries no number of the account; an error is kept to its first line
foreach ($line in @($out)) { Say "$line".Split("`n")[0] }
if ($code) { Say "the probe failed ($code)"; exit $code }
