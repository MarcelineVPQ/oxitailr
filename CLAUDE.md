# Claude Code Instructions for Oxitailr

## Release Process

When creating a GitHub release, **ALWAYS upload ALL THREE binaries**:

1. **Linux binary**: `target/release/oxitailr`
2. **Windows binary**: `target/x86_64-pc-windows-gnu/release/oxitailr.exe`
3. **AppImage**: `release/Oxitailr-{VERSION}-x86_64.AppImage`

### Build Commands

```bash
# Build all three binaries
cargo build --release
cargo build --release --target x86_64-pc-windows-gnu
./appimage/build-appimage.sh
```

### Release Command

```bash
gh release create v{VERSION} \
  --title "v{VERSION}" \
  --notes "Release notes here" \
  target/release/oxitailr \
  target/x86_64-pc-windows-gnu/release/oxitailr.exe \
  release/Oxitailr-{VERSION}-x86_64.AppImage
```

### Checklist Before Release

- [ ] Update version in `Cargo.toml`
- [ ] Update `CHANGELOG.md`
- [ ] Update version in `README.md` download instructions
- [ ] Build all three binaries (Linux, Windows, AppImage)
- [ ] Commit and push changes
- [ ] Create GitHub release with **ALL THREE** binaries attached
