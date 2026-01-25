#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$SCRIPT_DIR/build"
APPDIR="$BUILD_DIR/Oxitailr.AppDir"

echo "=== Building Oxitailr AppImage ==="

# Clean previous build
rm -rf "$BUILD_DIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$APPDIR/usr/share/applications"

# Build release binary if needed
echo "Building release binary..."
cd "$PROJECT_DIR"
cargo build --release

# Copy binary
echo "Copying files..."
cp "$PROJECT_DIR/target/release/oxitailr" "$APPDIR/usr/bin/"
chmod +x "$APPDIR/usr/bin/oxitailr"

# Copy icon
cp "$PROJECT_DIR/release/icon_256x256.png" "$APPDIR/oxitailr.png"
cp "$PROJECT_DIR/release/icon_256x256.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/oxitailr.png"

# Copy desktop file
cp "$SCRIPT_DIR/oxitailr.desktop" "$APPDIR/"
cp "$SCRIPT_DIR/oxitailr.desktop" "$APPDIR/usr/share/applications/"

# Create AppRun
cat > "$APPDIR/AppRun" << 'APPRUN'
#!/bin/bash
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/oxitailr" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

# Download and extract appimagetool if not present
APPIMAGETOOL_DIR="$SCRIPT_DIR/appimagetool"
if [ ! -f "$APPIMAGETOOL_DIR/AppRun" ]; then
    echo "Downloading appimagetool..."
    APPIMAGETOOL_APPIMAGE="$SCRIPT_DIR/appimagetool-x86_64.AppImage"
    wget -q "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage" -O "$APPIMAGETOOL_APPIMAGE"
    chmod +x "$APPIMAGETOOL_APPIMAGE"
    cd "$SCRIPT_DIR"
    "$APPIMAGETOOL_APPIMAGE" --appimage-extract
    mv squashfs-root "$APPIMAGETOOL_DIR"
    rm "$APPIMAGETOOL_APPIMAGE"
fi

# Build AppImage
echo "Creating AppImage..."
cd "$BUILD_DIR"
ARCH=x86_64 "$APPIMAGETOOL_DIR/AppRun" --no-appstream "$APPDIR"

# Move to release folder
VERSION=$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
OUTPUT_NAME="Oxitailr-${VERSION}-x86_64.AppImage"
mv Oxitailr-x86_64.AppImage "$PROJECT_DIR/release/$OUTPUT_NAME"

echo ""
echo "=== Done! ==="
echo "AppImage created: $PROJECT_DIR/release/$OUTPUT_NAME"
echo ""
echo "To run: ./release/$OUTPUT_NAME"
echo "To integrate with desktop: Install AppImageLauncher or copy to ~/Applications/"
