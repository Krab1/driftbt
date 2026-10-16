<#
  Builds the tracker-sim plugin and copies it into a Drift data folder as <data>\plugins\tracker-sim (plugin.json
  plus tracker-sim.exe). Quit Drift first, or the copy of a running program fails. Then turn it on in
  Settings -> Plugins.

  Usage: scripts\install-tracker-sim.ps1 -Data "$env:APPDATA\<Drift data folder>"
#>
param([Parameter(Mandatory)][string]$Data)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
cargo build --release -p drift-tracker-sim-plugin
if ($LASTEXITCODE -ne 0) { throw "build failed" }
$dest = Join-Path $Data "plugins\tracker-sim"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item "target\release\tracker-sim.exe" $dest -Force
Copy-Item "crates\drift-tracker-sim-plugin\plugin.json" $dest -Force
Write-Host "Installed to $dest"
