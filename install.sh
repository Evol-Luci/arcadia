#!/usr/bin/env bash
#
# Arcadia AppImage desktop integration (per-user, no sudo).
#
# AppImages are portable and self-contained, so by design they do NOT register
# themselves in the application list. This script integrates an AppImage you
# downloaded: it installs the AppImage to a stable location, writes an XDG
# desktop entry pointing at it, extracts the bundled icon, and refreshes the
# launcher caches. Every compliant launcher (GNOME, KDE, walker, rofi, wofi,
# ...) then shows Arcadia -- no desktop-environment detection needed.
#
#   ./install.sh path/to/Arcadia.AppImage   integrate the AppImage
#   ./install.sh --uninstall                remove everything this installed
#
# NOTE: this is only for the AppImage. If you have the .deb, just install it
# with the package manager -- it registers the app and icons by itself:
#   sudo apt install ./arcadia_0.1.0_amd64.deb     (remove: sudo apt remove arcadia)

set -euo pipefail

# --- identity (from desktop/src-tauri/tauri.conf.json) ---
APP_NAME="Arcadia"
APP_ID="arcadia"                       # .desktop filename + icon name
COMMENT="Linux-native emulation ecosystem."
CATEGORIES="Game;"
WM_CLASS="Arcadia"                     # for Hyprland window<->launcher mapping
KEYWORDS="emulation;emulator;game;launcher;retro;dreamshell;dreamvault;"

# --- XDG locations (honor overrides, fall back to spec defaults) ---
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS_DIR="$DATA_HOME/applications"
ICON_BASE="$DATA_HOME/icons/hicolor"
INSTALL_DIR="$DATA_HOME/$APP_ID"                 # where the AppImage is kept
INSTALLED_APPIMAGE="$INSTALL_DIR/$APP_NAME.AppImage"
DESKTOP_FILE="$APPS_DIR/$APP_ID.desktop"

refresh_caches() {
  command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -f -t "$ICON_BASE" >/dev/null 2>&1 || true
}

uninstall() {
  echo "Removing Arcadia desktop integration..."
  rm -fv "$DESKTOP_FILE"
  rm -fv "$ICON_BASE"/*/apps/"$APP_ID".png
  rm -rfv "$INSTALL_DIR"
  refresh_caches
  echo "Done. Arcadia removed from the application list."
}

# Extract the icon from inside the AppImage into the user's hicolor theme.
# Uses --appimage-extract, which works without FUSE. Falls back to the
# AppDir-root icon if the hicolor tree is not present.
install_icons_from_appimage() {
  local appimage="$1"
  local tmp; tmp="$(mktemp -d)"

  ( cd "$tmp" && "$appimage" --appimage-extract \
      "usr/share/icons/hicolor/*/apps/$APP_ID.png" >/dev/null 2>&1 ) || true

  local found=0
  local src
  while IFS= read -r -d '' src; do
    # .../hicolor/<size>/apps/arcadia.png  ->  capture <size>
    local size; size="$(basename "$(dirname "$(dirname "$src")")")"
    local dest_dir="$ICON_BASE/$size/apps"
    mkdir -p "$dest_dir"
    cp -f "$src" "$dest_dir/$APP_ID.png"
    echo "  icon:  $dest_dir/$APP_ID.png"
    found=1
  done < <(find "$tmp/squashfs-root/usr/share/icons/hicolor" \
             -name "$APP_ID.png" -print0 2>/dev/null)

  if [[ $found -eq 0 ]]; then
    # fallback: single root icon (.DirIcon / <id>.png) -> a 256x256 slot
    ( cd "$tmp" && "$appimage" --appimage-extract "$APP_ID.png" >/dev/null 2>&1 ) || true
    local root_icon="$tmp/squashfs-root/$APP_ID.png"
    [[ -f "$root_icon" ]] || root_icon="$tmp/squashfs-root/.DirIcon"
    if [[ -f "$root_icon" ]]; then
      mkdir -p "$ICON_BASE/256x256/apps"
      cp -f "$root_icon" "$ICON_BASE/256x256/apps/$APP_ID.png"
      echo "  icon:  $ICON_BASE/256x256/apps/$APP_ID.png (fallback)"
    else
      echo "  warning: no icon found inside the AppImage" >&2
    fi
  fi

  rm -rf "$tmp"
}

install() {
  local src="${1:-}"
  if [[ -z "$src" ]]; then
    echo "error: pass the AppImage path: ./install.sh path/to/Arcadia.AppImage" >&2
    exit 1
  fi
  if [[ ! -f "$src" ]]; then
    echo "error: AppImage not found: $src" >&2
    exit 1
  fi
  chmod +x "$src" 2>/dev/null || true

  echo "Integrating Arcadia AppImage..."

  # 1. install the AppImage to a stable location (entry stays valid even if you
  #    delete the download)
  mkdir -p "$INSTALL_DIR"
  if [[ "$(readlink -f "$src")" != "$(readlink -f "$INSTALLED_APPIMAGE" 2>/dev/null)" ]]; then
    cp -f "$src" "$INSTALLED_APPIMAGE"
  fi
  chmod +x "$INSTALLED_APPIMAGE"
  echo "  app:   $INSTALLED_APPIMAGE"

  # 2. desktop entry (absolute Exec -> the installed AppImage)
  mkdir -p "$APPS_DIR"
  cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=$APP_NAME
Comment=$COMMENT
Exec=$INSTALLED_APPIMAGE %U
Icon=$APP_ID
Terminal=false
Categories=$CATEGORIES
Keywords=$KEYWORDS
StartupNotify=true
StartupWMClass=$WM_CLASS
EOF
  chmod 644 "$DESKTOP_FILE"
  echo "  entry: $DESKTOP_FILE"

  # 3. icon (pulled from inside the AppImage)
  install_icons_from_appimage "$INSTALLED_APPIMAGE"

  # 4. refresh launcher caches
  refresh_caches
  echo "Done. Arcadia should now appear in your application launcher."
}

case "${1:-}" in
  --uninstall|-u|uninstall) uninstall ;;
  -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//' ;;
  *) install "$@" ;;
esac
