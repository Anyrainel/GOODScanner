# GGScanner rename rollout

The editions will be `GGScanner.exe` (capture, OCR, and manager) and
`GGScannerOCR.exe` (OCR and manager, with capture dependencies excluded).

## Publish the updater bridge first

The current workflow publishes the bridge under the existing `GOODScanner.exe`
and `GOODCapture.exe` download names. The internal Cargo targets already use
the GG names; CI copies their output to the old public names before uploading.
Keep the repository name and published executable names unchanged during this
stage. Website labels and application branding can already use GGScanner.

Publish this bridge before renaming the GitHub repository or public downloads.
Allow roughly a month for adoption, but do not assume every user will update.
The bridge updater tries both `Anyrainel/GGScanner` and `Anyrainel/GOODScanner`, follows
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

After the adoption period, rename the repository in GitHub Settings and update
local remotes to `https://github.com/Anyrainel/GGScanner.git`. Do not reuse the old
repository name: GitHub's redirect is needed by older installed versions.

At that cutoff, change CI's manifest roles to
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
This includes the restart environment variable, `goodscanner.hsr.manager-instructions`,
GOOD's `yas-GOODScanner` source, and Star Rail's historical `generator.name`.

The companion websites display GGScanner now, while linking to the existing
GOODScanner repository and GOODCapture/GOODScanner assets. Change those URLs
only after the new repository and executable names are available. The old
release URL used for manual updates can remain: GitHub redirects it.

The updater checks the API's asset inventory for the matching edition. If a
newer release has no matching executable, it shows a manual-download notice
instead of offering a broken automatic download. Download and installation
failures also offer a GitHub release-page link. With API access unavailable,
the manifest or legacy filename convention is used, and a missing download
is handled by the same manual fallback. A newer manifest can supersede a
cached API inventory; in that case download failure provides the fallback.

Old versions whose API access fails may not handle an extra repository-rename
redirect through a mirror. The bridge fixes this for updated installations;
legacy asset aliases cannot fix code already installed. If an old version cannot
discover the renamed repository, a manual download is required once.

## Validation

- Test legacy manifests and renamed manifest roles, including edition mapping.
- Test missing executables without switching an OCR installation to capture.
- Test redirect chains and bounded redirect loops with a local HTTP server.
- Check both editions build and keep capture dependencies feature-gated.
- Inspect release uploads for canonical and compatibility names before publishing.
