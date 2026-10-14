<#
  Builds a signed installer and the update manifest for Drift.

    scripts\release.ps1 -Version 0.1.0                 # build only; files land in target\release-out\0.1.0
    scripts\release.ps1 -Version 0.1.0 -Publish        # also create the GitHub release (Krab1/driftbt)
    scripts\release.ps1 -Version 0.0.2 -BaseUrl http://127.0.0.1:8899/   # manifest for a local test server

  The signing key stays outside the repo (default %USERPROFILE%\.tauri\drift.key). Losing it means installed copies can
  never update again; the matching public key is in src-tauri\tauri.conf.json.
#>
param(
  [Parameter(Mandatory)] [string]$Version,
  [switch]$Publish,
  [string]$Notes = "",
  [string]$BaseUrl = "",
  [string]$KeyPath = "$env:USERPROFILE\.tauri\drift.key",
  [string]$Repo = "Krab1/driftbt"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3" }
if (-not (Test-Path $KeyPath)) { throw "Signing key not found at $KeyPath" }

# One version everywhere: the app, the crates and the installer.
$conf = Get-Content src-tauri\tauri.conf.json -Raw
$conf = [regex]::Replace($conf, '("version":\s*")[^"]+(")', "`${1}$Version`${2}", 1)
[IO.File]::WriteAllText("$PWD\src-tauri\tauri.conf.json", $conf)
$cargo = Get-Content Cargo.toml -Raw
$cargo = [regex]::Replace($cargo, '(?m)^(version\s*=\s*")[^"]+(")', "`${1}$Version`${2}", 1)
[IO.File]::WriteAllText("$PWD\Cargo.toml", $cargo)

$env:TAURI_SIGNING_PRIVATE_KEY = $KeyPath
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
npm run tauri build -- --bundles nsis
if ($LASTEXITCODE -ne 0) { throw "build failed" }

$out = "target\release-out\$Version"
New-Item -ItemType Directory -Force $out | Out-Null
$exeName = "Drift_${Version}_x64-setup.exe"
Copy-Item "target\release\bundle\nsis\$exeName" $out -Force
Copy-Item "target\release\bundle\nsis\$exeName.sig" $out -Force
$sig = (Get-Content "$out\$exeName.sig" -Raw).Trim()
$base = if ($BaseUrl) { $BaseUrl.TrimEnd('/') } else { "https://github.com/$Repo/releases/download/v$Version" }
$manifest = [ordered]@{
  version  = $Version
  notes    = $Notes
  pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = @{ "windows-x86_64" = [ordered]@{ signature = $sig; url = "$base/$exeName" } }
}
[IO.File]::WriteAllText("$PWD\$out\latest.json", ($manifest | ConvertTo-Json -Depth 5))
Write-Host "Built $out ($exeName, latest.json)"

if ($Publish) {
  if ($BaseUrl) { throw "-Publish and -BaseUrl don't go together" }
  gh release create "v$Version" "$out\$exeName" "$out\$exeName.sig" "$out\latest.json" --repo $Repo --title "Drift $Version" --notes $Notes
  if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
  Write-Host "Published v$Version"
}
