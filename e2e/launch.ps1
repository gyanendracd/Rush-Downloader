# Starts a separate test copy of the app and prints its process id.
# It uses its own list, its own download folder and its own browser data, so it
# can run beside a copy of the app that is already open, and it is parked
# off-screen. Needs the interface server on port 1420 (`npm run dev`, or a
# running `npm run tauri dev`).
param(
  [Parameter(Mandatory = $true)][string]$TestFolder,
  [int]$DebugPort = 9333
)
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\debug\rushdownload.exe"
if (-not (Test-Path $exe)) { "build the app first: cargo build -p rushdownload"; exit 1 }
if (-not (Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)) {
  "the interface server is not running on port 1420: start it with 'npm run dev' first"; exit 1
}

$state = Join-Path $TestFolder "state"
$out = Join-Path $TestFolder "out"
New-Item -ItemType Directory -Force $state, $out, (Join-Path $TestFolder "webview") | Out-Null
$saved = Join-Path $state "downloads.json"
if (-not (Test-Path $saved)) {
  $seed = @{ next_id = 1; tasks = @(); settings = @{ download_dir = $out; max_concurrent = 3; connections = 4; speed_limit = 0 } }
  [IO.File]::WriteAllText($saved, ($seed | ConvertTo-Json -Depth 5))
}

$env:RUSHDOWNLOAD_STATE_DIR = $state
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $TestFolder "webview"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$DebugPort"
$p = Start-Process -FilePath $exe -PassThru
Set-Content -Path (Join-Path $TestFolder "pid.txt") -Value $p.Id

$up = $false
foreach ($i in 1..60) {
  try {
    $t = Invoke-RestMethod -Uri "http://127.0.0.1:$DebugPort/json" -TimeoutSec 2
    if ($t | Where-Object { $_.url -like "*localhost:1420*" }) { $up = $true; break }
  } catch {}
  if ($p.HasExited) { break }
  Start-Sleep -Seconds 1
}
if (-not $up) { "test app did not come up (exited: $($p.HasExited))"; exit 1 }
Start-Sleep -Seconds 1
& (Join-Path $PSScriptRoot "win2.ps1") -ProcessId $p.Id -Do hide | Out-Null
"test app running, process id $($p.Id), window parked off-screen"
