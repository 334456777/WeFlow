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

  When the profile was written, it starts bench.yml so that the new environment holds results to compare at once: the
  three newest releases (no release candidate), then the four PRs labeled `bench` merged last, each before and after
  it (`#N前`, `#N后`), the oldest first, so the oldest release becomes the environment's anchor. bench.yml measures them
  during the night in Japan, over more than one night when they do not fit. That takes a fine-grained token in
  -TokenFile, for this repository only, with Actions read and write (to start the run) and Contents, Issues and Pull
  requests read-only (to list the releases and the PRs); without the file the run is not started, which the log says.
  The file must stay in your own folders: -BenchDir is the runner's to read.

.EXAMPLE
  .\calibrate.ps1                 # once, now
  .\calibrate.ps1 -Register       # every four weeks, Friday 02:40, whether you are logged on or not, waking the computer
#>
param(
  [string] $BenchDir = 'E:\weflow-bench',
  [string] $Config = (Join-Path ($env:WEFLOW_HOME ? $env:WEFLOW_HOME : (Join-Path $env:APPDATA 'weflow')) 'config.json'),
  [string] $RunnerAccount = 'weflow-bench',
  [string] $Repository = 'https://github.com/334456777/WeFlow.git',
  [string] $TokenFile = (Join-Path $env:LOCALAPPDATA 'weflow-bench\dispatch-token'),
  [switch] $Register
)
$ErrorActionPreference = 'Stop'

if ($Register) {
  $action = New-ScheduledTaskAction -Execute (Get-Command pwsh).Source `
    -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -BenchDir `"$BenchDir`" -Config `"$Config`""
  $trigger = New-ScheduledTaskTrigger -Weekly -WeeksInterval 4 -DaysOfWeek Friday -At 02:40
  # S4U: runs whether you are logged on or not, without keeping your password
  $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType S4U -RunLevel Limited
  # wakes the computer from sleep; a run missed while it was off starts once it is back
  $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -WakeToRun -ExecutionTimeLimit (New-TimeSpan -Hours 3)
  Register-ScheduledTask -TaskName 'WeFlow benchmark calibration' -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force | Out-Null
  Write-Host 'registered: WeFlow benchmark calibration, every four weeks on Friday at 02:40'
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
if (-not (@($out) | Where-Object { "$_" -like 'written:*' })) { exit 0 }

# ---- a new profile: bench.yml on the releases and the PRs labeled bench ----

if (-not (Test-Path $TokenFile)) { Say "no token at ${TokenFile}: bench.yml not started"; exit 0 }
$repo = ([uri] $Repository).AbsolutePath.Trim('/') -replace '\.git$', ''
$headers = @{
  Authorization = "Bearer $((Get-Content -Raw $TokenFile).Trim())"
  Accept = 'application/vnd.github+json'
  'X-GitHub-Api-Version' = '2022-11-28'
}
function Api([string] $Method, [string] $Path, $Body) {
  $call = @{ Method = $Method; Uri = "https://api.github.com/repos/$repo$Path"; Headers = $headers }
  if ($Body) { $call.Body = $Body | ConvertTo-Json -Compress; $call.ContentType = 'application/json' }
  # a JSON array comes back as one object: written from a variable, its items go down the pipeline one by one
  $reply = Invoke-RestMethod @call
  $reply
}
try {
  # the three newest releases, the oldest first: it becomes the new environment's anchor
  $releases = Api GET '/releases?per_page=100' |
    Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -match '^v\d+\.\d+\.\d+$' } |
    Sort-Object { [datetime] $_.published_at } -Descending | Select-Object -First 3
  $commits = @($releases | Sort-Object { [datetime] $_.published_at } | ForEach-Object { $_.tag_name })
  # the four PRs labeled bench merged last, each before and after it, the oldest first
  $prs = Api GET '/issues?labels=bench&state=closed&per_page=100' | Where-Object { $_.pull_request } |
    ForEach-Object { Api GET "/pulls/$($_.number)" } | Where-Object { $_.merged_at } |
    Sort-Object { [datetime] $_.merged_at } -Descending | Select-Object -First 4
  foreach ($pr in @($prs | Sort-Object { [datetime] $_.merged_at })) {
    $commits += "#$($pr.number)前=$($pr.merge_commit_sha)^", "#$($pr.number)后=$($pr.merge_commit_sha)"
  }
  Api POST '/actions/workflows/bench.yml/dispatches' @{ ref = 'main'; inputs = @{ commits = $commits -join ',' } } |
    Out-Null
  Say "bench.yml started on $($commits.Count) commits: $(($commits | ForEach-Object { ($_ -split '=')[0] }) -join ', ')"
} catch {
  Say "bench.yml not started: $($_.Exception.Message.Split("`n")[0])"
  exit 1
}
