# GGScanner rename rollout

The editions will be `GGScanner.exe` (capture, OCR, and manager) and
`GGScannerOCR.exe` (OCR and manager, with capture dependencies excluded).

## Publish the updater bridge first

Publish a release with the existing `GOODScanner.exe` and `GOODCapture.exe`
build targets before renaming the GitHub repository or build targets. The bridge
updater tries both `Anyrainel/GGScanner` and `Anyrainel/GOODScanner`, follows
repository-rename redirect chains, and uses stable edition roles in `update.json`:

```json
{
  "tag": "v2026.10.07",
  "revision": 125,
  "assets": {
    "scanner": "GOODScanner-125.exe",
    "capture": "GOODCapture-125.exe"
  }
}
```

The example revision is illustrative; CI writes its actual run number. These
filenames are complete uploaded asset names, including their revision suffix.
Old manifests without `assets` remain supported. Existing updaters ignore the
additional fields.

## Rename and retain compatibility downloads

After publishing the bridge, rename the repository in GitHub Settings and update
local remotes to `https://github.com/Anyrainel/GGScanner.git`. Do not reuse the old
repository name: GitHub's redirect is needed by older installed versions.

Rename the build targets and update CI's manifest roles to
`scanner: GGScannerOCR-<revision>.exe` and `capture: GGScanner-<revision>.exe`.
Every release must continue uploading copies under both old names
(`GOODScanner.exe`, `GOODCapture.exe`) and their revision-specific names. Users
can skip the bridge release, so a one-release alias window is insufficient.
Aliases must preserve edition: old GOODScanner receives the OCR build; old
GOODCapture receives the capture build.

The update replaces the executable at its existing local path and restarts that
path. It deliberately preserves the installed filename and shortcuts; a fresh
download uses the new names. Existing settings filenames and wire-format IDs
are independent of the branding and should remain compatible.

Old versions whose API access fails may not handle an extra repository-rename
redirect through a mirror. The bridge fixes this for updated installations;
legacy asset aliases cannot fix code already installed. If an old version cannot
discover the renamed repository, a manual download is required once.

## Validation

- Test legacy manifests and renamed manifest roles, including edition mapping.
- Test redirect chains and bounded redirect loops with a local HTTP server.
- Check both editions build and keep capture dependencies feature-gated.
- Inspect release uploads for canonical and compatibility names before publishing.
