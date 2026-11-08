<#
.SYNOPSIS
  Sets up this Windows machine as the benchmark runner of bench.yml (#601, #603). Run once, as an administrator.

.DESCRIPTION
  - a local account of its own (not an administrator) that runs the GitHub Actions runner as a service, so the jobs
    can read neither your user folder (where WeChat keeps its data) nor the folders given in -Protect, which it is
    explicitly denied
  - the runner (the newest stable release at least 3 days old, its download checked against the digest GitHub
    publishes) in -RunnerDir, registered to the repository with the label weflow-bench
  - -BenchDir, the runner's own folder (accounts, binaries, build cache, Rust), named in the runner's .env as
    WEFLOW_BENCH_DIR (bench.yml's steps call Git for Windows' bash by its full path and put Git on their own PATH,
    so the runner's PATH needs no Git)
  - Microsoft Defender exclusions for -BenchDir and the runner's work folder: real-time scanning of what a benchmark
    writes is noise in its timings (the code built there is the repository's own)
  The registration token comes from the repository's Settings > Actions > Runners > New self-hosted runner, or from
  `gh api -X POST repos/334456777/WeFlow/actions/runners/registration-token -q .token`; it is valid for an hour.
  Running it again updates the folders and the .env and registers the runner again (--replace).

.EXAMPLE
  .\setup-runner.ps1 -Token <token> -BenchDir D:\weflow-bench -Protect D:\WeFlow-target
#>
param(
  [Parameter(Mandatory)] [string] $Token,
  [string] $BenchDir = 'D:\weflow-bench',
  [string] $RunnerDir = 'C:\actions-runner-weflow',
  [string] $Account = 'weflow-bench',
  [string] $Repository = '334456777/WeFlow',
  # folders the runner's account must not read, besides every user folder (which it cannot read by default)
  [string[]] $Protect = @(),
  # also switch the machine to the High performance power plan
  [switch] $HighPerformance
)
$ErrorActionPreference = 'Stop'

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'Run this script as an administrator.'
}
$bash = 'C:\Program Files\Git\bin\bash.exe'
if (-not (Test-Path $bash)) { throw "Git for Windows is not installed for all users ($bash): install it first." }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere) -or -not (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)) {
  throw 'The MSVC build tools (Visual Studio or Build Tools with "Desktop development with C++") are not installed.'
}

# ---- the account ----
# a random password nobody needs to know: the service keeps it; one of each kind meets any complexity policy
$chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!#%+-=?@^_'
$password = 'Aa1!' + -join (1..32 | ForEach-Object { $chars[[Security.Cryptography.RandomNumberGenerator]::GetInt32($chars.Length)] })
$secure = ConvertTo-SecureString $password -AsPlainText -Force
if (Get-LocalUser -Name $Account -ErrorAction SilentlyContinue) {
  Set-LocalUser -Name $Account -Password $secure -PasswordNeverExpires $true
} else {
  New-LocalUser -Name $Account -Password $secure -PasswordNeverExpires -UserMayNotChangePassword `
    -Description 'Runs the WeFlow benchmark runner (bench.yml)' | Out-Null
}
Write-Host "account $Account ready"

# ---- the folders ----
foreach ($dir in @($BenchDir, $RunnerDir)) {
  New-Item -ItemType Directory -Force $dir | Out-Null
  icacls $dir /grant "${Account}:(OI)(CI)F" /T /Q | Out-Null
}
foreach ($dir in $Protect) {
  if (Test-Path $dir) {
    icacls $dir /deny "${Account}:(OI)(CI)F" /Q | Out-Null
    Write-Host "denied $Account on $dir"
  }
}
# Git's folder may have lost the Users group to another tool's sandbox: the account reads and runs it by name
icacls 'C:\Program Files\Git' /grant "${Account}:(OI)(CI)RX" /T /Q | Out-Null
Add-MpPreference -ExclusionPath $BenchDir, (Join-Path $RunnerDir '_work')
Write-Host "Defender exclusions: $BenchDir, $(Join-Path $RunnerDir '_work')"
if ($HighPerformance) {
  powercfg /setactive SCHEME_MIN
  Write-Host 'power plan: High performance'
}

# ---- the runner ----
$cutoff = (Get-Date).ToUniversalTime().AddDays(-3)
# kept in a variable first: piped straight on, the JSON array would go down the pipeline as one object
$releases = Invoke-RestMethod 'https://api.github.com/repos/actions/runner/releases?per_page=20'
$release = $releases | Where-Object { -not $_.prerelease -and -not $_.draft -and ([datetime]$_.published_at).ToUniversalTime() -lt $cutoff } |
  Select-Object -First 1
$asset = $release.assets | Where-Object { $_.name -like 'actions-runner-win-x64-*.zip' } | Select-Object -First 1
if (-not $asset) { throw 'no Windows x64 runner found in the releases' }
if (-not (Test-Path (Join-Path $RunnerDir 'config.cmd'))) {
  $zip = Join-Path $env:TEMP $asset.name
  Invoke-WebRequest $asset.browser_download_url -OutFile $zip
  $want = ($asset.digest -replace '^sha256:', '').ToUpper()
  $got = (Get-FileHash $zip -Algorithm SHA256).Hash
  if (-not $want -or $want -ne $got) { Remove-Item $zip; throw "the runner download does not match its digest ($got)" }
  Expand-Archive $zip -DestinationPath $RunnerDir -Force
  Remove-Item $zip
  Write-Host "runner $($release.tag_name) unpacked in $RunnerDir"
}

Push-Location $RunnerDir
try {
  & .\config.cmd --unattended --replace --url "https://github.com/$Repository" --token $Token `
    --name "weflow-bench-$env:COMPUTERNAME" --labels weflow-bench --work _work `
    --runasservice --windowslogonaccount ".\$Account" --windowslogonpassword $password
  if ($LASTEXITCODE) { throw "config.cmd failed ($LASTEXITCODE)" }
  # what the jobs see: the bench folder
  Set-Content -Encoding ascii .env "WEFLOW_BENCH_DIR=$BenchDir"
  icacls $RunnerDir /grant "${Account}:(OI)(CI)F" /T /Q | Out-Null
  Get-Service -Name 'actions.runner.*' | Where-Object { $_.BinaryPathName -like "*$RunnerDir*" } | Restart-Service
} finally {
  Pop-Location
}
$password = $null
Write-Host "done: the runner weflow-bench-$env:COMPUTERNAME runs as $Account; check it at https://github.com/$Repository/settings/actions/runners"
