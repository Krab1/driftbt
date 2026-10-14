# Releasing Drift

Releases live on GitHub Releases of `Krab1/driftbt`. Installed copies check
`https://github.com/Krab1/driftbt/releases/latest/download/latest.json` once a day (and from Settings → General → Check now),
show a banner, and install only after the user clicks **Install and restart**.

## Cut a release

```powershell
scripts\release.ps1 -Version 0.1.0 -Notes "What changed"            # build + sign, files in target\release-out\0.1.0
scripts\release.ps1 -Version 0.1.0 -Notes "What changed" -Publish   # same, then `gh release create v0.1.0`
```

The script sets the version in `Cargo.toml` and `src-tauri/tauri.conf.json`, builds the NSIS installer, signs it, writes
`latest.json`, and uploads the installer, its `.sig` and `latest.json`. Commit the version bump afterwards.
`npm run tauri build` needs `cargo` on `PATH`.

## The signing key

`%USERPROFILE%\.tauri\drift.key` (no password) signs every update; the public half is `plugins.updater.pubkey` in
`tauri.conf.json`. **Back it up and never commit it.** If it is lost, installed copies can never update again and users must
reinstall by hand. Changing the key has the same effect.

## Testing an update without publishing

1. `scripts\release.ps1 -Version 0.0.2 -BaseUrl http://127.0.0.1:8899` (builds and writes a manifest pointing at a local server),
   then put the version back to the current one.
2. Serve `target\release-out\0.0.2` on that port.
3. Run a **debug** build with `DRIFT_UPDATE_URL=http://127.0.0.1:8899/latest.json` (the override is compiled out of release
   builds) and click Check now → Install and restart.

This was done on 2026-10-14: the signature verified, the installer ran, the app came back as 0.0.2.

## Unsigned installer

The installer isn't code-signed, so Windows SmartScreen warns on first run. The update signature is a different thing: it
protects the update channel, not SmartScreen.
