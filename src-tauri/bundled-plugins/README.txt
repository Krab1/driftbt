Drift's first-party plugins (the search sources) are copied here by scripts\bundle-plugins.ps1 before a release build
and end up inside the installer. The folders it creates are generated and not committed; this file only keeps the
folder in git so the installer's resource list always has something to match.
