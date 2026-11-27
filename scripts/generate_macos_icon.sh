#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Error: generate_macos_icon.sh requires macOS (Darwin) with iconutil." >&2
  exit 1
fi

if ! command -v iconutil >/dev/null 2>&1; then
  echo "Error: 'iconutil' not found. Install Xcode Command Line Tools." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ICON_DIR="$ROOT_DIR/packaging/macos/icon"
SVG_PATH="${SVG_PATH:-$ICON_DIR/rusty-loader-icon.svg}"
MASTER_PNG="${MASTER_PNG:-$ICON_DIR/rusty-loader-icon-1024.png}"
ICNS_OUT="${ICNS_OUT:-$ROOT_DIR/packaging/macos/rusty-loader.icns}"

FROM_PNG=false
if [[ "${1:-}" == "--from-png" ]]; then
  FROM_PNG=true
elif [[ -n "${1:-}" ]]; then
  echo "Usage: $(basename "$0") [--from-png]" >&2
  exit 1
fi

mkdir -p "$ICON_DIR" "$(dirname "$ICNS_OUT")"

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

rasterize_svg_to_png() {
  local svg_file="$1"
  local png_out="$2"

  if command -v rsvg-convert >/dev/null 2>&1; then
    echo "Rasterizing SVG via rsvg-convert..."
    rsvg-convert -w 1024 -h 1024 "$svg_file" -o "$png_out"
    return 0
  fi

  local chrome_bin="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
  if [[ -x "$chrome_bin" ]]; then
    echo "Rasterizing SVG via headless Google Chrome..."
    local wrapper_html="$TMP_DIR/render_icon.html"
    local abs_svg
    abs_svg="$(cd "$(dirname "$svg_file")" && pwd)/$(basename "$svg_file")"
    cat > "$wrapper_html" <<EOF
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <style>
    html, body {
      margin: 0;
      padding: 0;
      width: 1024px;
      height: 1024px;
      background: transparent;
      overflow: hidden;
    }
    img {
      display: block;
      width: 1024px;
      height: 1024px;
    }
  </style>
</head>
<body>
  <img src="file://${abs_svg}" width="1024" height="1024" alt="">
</body>
</html>
EOF
    "$chrome_bin" \
      --headless=new \
      --disable-gpu \
      --default-background-color=00000000 \
      --window-size=1024,1024 \
      --force-device-scale-factor=1 \
      --hide-scrollbars \
      --no-first-run \
      --no-default-browser-check \
      --user-data-dir="$TMP_DIR/chrome-profile" \
      --screenshot="$png_out" \
      "file://${wrapper_html}" >/dev/null 2>&1
    return 0
  fi

  if command -v qlmanage >/dev/null 2>&1; then
    echo "Rasterizing SVG via qlmanage fallback..."
    qlmanage -t -s 1024 -o "$TMP_DIR" "$svg_file" >/dev/null 2>&1
    local ql_out="$TMP_DIR/$(basename "$svg_file").png"
    if [[ -f "$ql_out" ]]; then
      cp "$ql_out" "$png_out"
      return 0
    fi
  fi

  echo "Error: No SVG rasterizer available (checked rsvg-convert, Google Chrome, qlmanage)." >&2
  echo "Provide a 1024x1024 transparent RGBA PNG at '${MASTER_PNG}' and re-run with --from-png." >&2
  exit 1
}

if [[ "$FROM_PNG" == "false" ]]; then
  if [[ ! -f "$SVG_PATH" ]]; then
    echo "Error: Master SVG not found at '${SVG_PATH}'." >&2
    exit 1
  fi
  rasterize_svg_to_png "$SVG_PATH" "$MASTER_PNG"
else
  if [[ ! -f "$MASTER_PNG" ]]; then
    echo "Error: Master PNG not found at '${MASTER_PNG}'." >&2
    exit 1
  fi
fi

# Validate dimensions and transparent corner alpha
python3 - "$MASTER_PNG" <<'PY'
import sys
from PIL import Image

path = sys.argv[1]
img = Image.open(path).convert("RGBA")
w, h = img.size
if (w, h) != (1024, 1024):
    raise SystemExit(f"Error: Expected 1024x1024 PNG at {path}, got {w}x{h}")

corner = img.getpixel((0, 0))
if corner[3] != 0:
    raise SystemExit(f"Error: Expected transparent outer corner at (0,0), got RGBA {corner}")

# Normalize to clean RGBA PNG
img.save(path, format="PNG")
print(f"Verified master PNG: {path} ({w}x{h} RGBA, transparent corners)")
PY

ICONSET_DIR="$TMP_DIR/rusty-loader.iconset"
mkdir -p "$ICONSET_DIR"

# Generate all 10 required macOS iconset PNG resolutions
if python3 -c "import PIL" >/dev/null 2>&1; then
  python3 - "$MASTER_PNG" "$ICONSET_DIR" <<'PY'
import os
import sys
from PIL import Image

master_path, out_dir = sys.argv[1], sys.argv[2]
master = Image.open(master_path).convert("RGBA")

entries = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
]

for filename, size in entries:
    out_path = os.path.join(out_dir, filename)
    if size == 1024:
        resized = master.copy()
    else:
        resized = master.resize((size, size), Image.Resampling.LANCZOS)
    resized.save(out_path, format="PNG")
PY
else
  sips -z 16 16 "$MASTER_PNG" --out "$ICONSET_DIR/icon_16x16.png" >/dev/null
  sips -z 32 32 "$MASTER_PNG" --out "$ICONSET_DIR/icon_16x16@2x.png" >/dev/null
  cp "$ICONSET_DIR/icon_16x16@2x.png" "$ICONSET_DIR/icon_32x32.png"
  sips -z 64 64 "$MASTER_PNG" --out "$ICONSET_DIR/icon_32x32@2x.png" >/dev/null
  sips -z 128 128 "$MASTER_PNG" --out "$ICONSET_DIR/icon_128x128.png" >/dev/null
  sips -z 256 256 "$MASTER_PNG" --out "$ICONSET_DIR/icon_128x128@2x.png" >/dev/null
  cp "$ICONSET_DIR/icon_128x128@2x.png" "$ICONSET_DIR/icon_256x256.png"
  sips -z 512 512 "$MASTER_PNG" --out "$ICONSET_DIR/icon_256x256@2x.png" >/dev/null
  cp "$ICONSET_DIR/icon_256x256@2x.png" "$ICONSET_DIR/icon_512x512.png"
  cp "$MASTER_PNG" "$ICONSET_DIR/icon_512x512@2x.png"
fi

iconutil -c icns "$ICONSET_DIR" -o "$ICNS_OUT"

# Round-trip check with iconutil
VERIFY_ICONSET="$TMP_DIR/verify.iconset"
iconutil -c iconset "$ICNS_OUT" -o "$VERIFY_ICONSET"
if [[ ! -s "$ICNS_OUT" || ! -d "$VERIFY_ICONSET" ]]; then
  echo "Error: Failed to verify generated .icns at '${ICNS_OUT}'." >&2
  exit 1
fi

echo "Generated and verified macOS icon bundle: ${ICNS_OUT}"
