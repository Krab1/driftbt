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
& "$PSScriptRoot\bundle-plugins.ps1"
if ($LASTEXITCODE -ne 0) { throw "bundling the plugins failed" }
npm run tauri build -- --bundles nsis
if ($LASTEXITCODE -ne 0) { throw "build failed" }

# The installer would otherwise call itself "Drift" in Task Manager and file properties, the same name as the app.
# tauri renders the whole script before running makensis, so patch the rendered copy and build it again.
$exeName = "Drift_${Version}_x64-setup.exe"
$nsiDir = "target\release\nsis\x64"
$nsi = Join-Path $nsiDir "installer.nsi"
$text = [IO.File]::ReadAllText("$PWD\$nsi")
$patched = $text -replace 'VIAddVersionKey "(ProductName|FileDescription)" "\$\{PRODUCTNAME\}"', 'VIAddVersionKey "$1" "$${PRODUCTNAME} Installer"'
if ($patched -eq $text) { throw "installer.nsi has no ProductName/FileDescription lines to rename" }
[IO.File]::WriteAllText("$PWD\$nsi", $patched, (New-Object Text.UTF8Encoding($true)))

# tauri's own "is the app running" check (Windows Restart Manager) aborts the whole install with "Failed to kill Drift"
# whenever Windows lists anything holding drift.exe and can't shut it down. Drift ends itself and waits for its process
# to be gone in installer-hooks.nsh, so the check is replaced by nothing. If the exe were somehow still locked, the file
# copy gives the normal Retry dialog instead of aborting.
$utils = Join-Path $nsiDir "utils.nsh"
$u = [IO.File]::ReadAllText("$PWD\$utils")
$u2 = [regex]::Replace($u, '(?s)(!macro CheckIfAppIsRunning executablePath productName).*?(!macroend)', "`$1`r`n  ; Replaced by DriftStopApp in installer-hooks.nsh (see scripts\release.ps1).`r`n`$2", 1)
if ($u2 -eq $u -or $u2 -match 'RmShutdown') { throw "utils.nsh: CheckIfAppIsRunning was not replaced" }
[IO.File]::WriteAllText("$PWD\$utils", $u2, (New-Object Text.UTF8Encoding($true)))
Push-Location $nsiDir
& "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe" /V2 installer.nsi
$made = $LASTEXITCODE
Pop-Location
if ($made -ne 0) { throw "makensis failed" }
Copy-Item (Join-Path $nsiDir "nsis-output.exe") "target\release\bundle\nsis\$exeName" -Force
Remove-Item "target\release\bundle\nsis\$exeName.sig" -ErrorAction SilentlyContinue
# The CLI refuses a key path together with the key environment variables, so set them aside for this call.
$savedKey = $env:TAURI_SIGNING_PRIVATE_KEY
Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
npx tauri signer sign -f $KeyPath '--password=' "target\release\bundle\nsis\$exeName"
$signed = $LASTEXITCODE
$env:TAURI_SIGNING_PRIVATE_KEY = $savedKey
if ($signed -ne 0) { throw "signing failed" }

$out = "target\release-out\$Version"
New-Item -ItemType Directory -Force $out | Out-Null
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
