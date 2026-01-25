# Claude Code Instructions for Oxitailr

## Release Process

When creating a GitHub release, **ALWAYS upload ALL THREE binaries** with consistent naming:

1. **Linux binary**: `oxitailr-{VERSION}-x86_64`
2. **Windows binary**: `oxitailr-{VERSION}-x86_64.exe`
3. **AppImage**: `Oxitailr-{VERSION}-x86_64.AppImage`

**IMPORTANT**:
- Always include the VERSION in all binary names
- Always include `-x86_64` suffix on all binaries (we only build 64-bit)

### Build Commands

```bash
# Build all three binaries
cargo build --release
cargo build --release --target x86_64-pc-windows-gnu
./appimage/build-appimage.sh
```

### Release Command

```bash
# Set version variable
VERSION="0.2.13"  # Update this!

# Copy binaries with correct names
cp target/release/oxitailr /tmp/oxitailr-${VERSION}-x86_64
cp target/x86_64-pc-windows-gnu/release/oxitailr.exe /tmp/oxitailr-${VERSION}-x86_64.exe

# Create release with all three binaries
gh release create v${VERSION} \
  --title "v${VERSION}" \
  --notes "Release notes here" \
  /tmp/oxitailr-${VERSION}-x86_64 \
  /tmp/oxitailr-${VERSION}-x86_64.exe \
  release/Oxitailr-${VERSION}-x86_64.AppImage
```

### Checklist Before Release

- [ ] Update version in `Cargo.toml`
- [ ] Update `CHANGELOG.md`
- [ ] Update version in `README.md` download instructions
- [ ] Build all three binaries (Linux, Windows, AppImage)
- [ ] Commit and push changes
- [ ] Create GitHub release with **ALL THREE** binaries:
  - [ ] `oxitailr-{VERSION}-x86_64` (Linux)
  - [ ] `oxitailr-{VERSION}-x86_64.exe` (Windows)
  - [ ] `Oxitailr-{VERSION}-x86_64.AppImage`
