# Changelog

Notable changes to poclimon. Every PR into `develop` adds a line under
**Unreleased**; merging `develop` into `main` turns that section into the next
version's heading and its GitHub Release notes.

Sections: `Added`, `Changed`, `Fixed`, `Removed`. Write entries for players, not
for the diff.

## [Unreleased]

### Changed
- New Game Boy Color look: white dialog-style text boxes, a white pen with grass and flowers, and battle-HUD nameplates with a blue EXP bar.
- Prefer your terminal's own colours? Set `theme = "terminal"` under `[display]` in `poclimon.toml`.

### Fixed
- Hopping creatures stay whole mid-jump instead of vanishing out of their sprite box.

## Earlier releases

Notes for v0.6.3 and before are on the
[GitHub Releases page](https://github.com/JohnSBarden/poclimon/releases).
