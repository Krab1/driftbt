<#
  Builds the first-party plugin program and lays the plugin folders out in src-tauri\bundled-plugins, which the
  installer then carries. Run by release.ps1; run it by hand before `tauri dev` or a local `tauri build`.

  Each folder under crates\drift-search-plugin\plugins holds a plugin.json; every one gets the same drift-search.exe,
  which picks its source from the argument in that plugin.json.
#>
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
$profileDir = if ($args -contains "-Debug") { "debug" } else { "release" }
if ($profileDir -eq "release") { cargo build --release -p drift-search-plugin } else { cargo build -p drift-search-plugin }
if ($LASTEXITCODE -ne 0) { throw "building the plugin program failed" }

$appVersion = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $appVersion) { throw "couldn't read the app version from Cargo.toml" }
$exe = "target\$profileDir\drift-search.exe"
$out = "src-tauri\bundled-plugins"
Get-ChildItem $out -Directory -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
foreach ($dir in Get-ChildItem "crates\drift-search-plugin\plugins" -Directory) {
  $dest = Join-Path $out $dir.Name
  New-Item -ItemType Directory -Force $dest | Out-Null
  # The plugin's version follows the app's, so every app release that changes the program also upgrades installed copies.
  $manifest = (Get-Content (Join-Path $dir.FullName "plugin.json") -Raw) -replace '("version":\s*")[^"]+(")', "`${1}$appVersion`${2}"
  [IO.File]::WriteAllText("$PWD\$dest\plugin.json", $manifest)
  Copy-Item $exe $dest
}
Write-Host ("Bundled plugins: " + ((Get-ChildItem $out -Directory | ForEach-Object Name) -join ", "))
