# npm engine package disposition

Owner: Val, OpenCoven release maintainer. Reviewed against `origin/main` at
`f38210d` on 2026-10-17.

The supported installation path is the managed `@opencoven/cli` engine. The
direct `@opencoven/coven-code` package is deprecated but retained for versions
that were published. The [release workflow](../.github/workflows/release.yml)
defaults `publish_npm` to false; the separate
[npm publish workflow](../.github/workflows/npm-publish.yml) is manual and
requires an existing GitHub Release. This keeps direct npm publication an
explicit release-maintainer action rather than a requirement for every engine
release.

The [package manifest](../npm/package.json) has no `optionalDependencies` or
optional installation branch. Its `postinstall` runs the
[native installer](../npm/install.js), which verifies the downloaded archive
against `checksums.json` before extraction. An already-present native binary
skips the download and checksum check. That shortcut allows an offline rerun
of `postinstall` in an existing package; it is not an integrity check
for that existing binary. The managed CLI or a verified release archive is
the recommended path when the existing binary's provenance is uncertain.

Val owns any future decision to republish the deprecated package, change its
offline shortcut, or remove the direct npm path. The manual Windows archive
path and download deadline fixes are tracked in [#347](https://github.com/OpenCoven/coven-code/pull/347)
and [#354](https://github.com/OpenCoven/coven-code/pull/354). Native
Windows installer behavior still needs Windows runtime verification.
