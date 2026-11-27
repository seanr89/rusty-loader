#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Error: macos_bundle.sh requires macOS (Darwin)." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

APP_NAME="${APP_NAME:-rusty-loader}"
APP_DIR="${APP_DIR:-/Applications}"
APP_DIR="${APP_DIR/#\~/$HOME}"
APP_DIR="${APP_DIR%/}"
[[ -z "$APP_DIR" ]] && APP_DIR="/"

BUNDLE_ID="${BUNDLE_ID:-com.seanr89.rusty-loader}"
CODESIGN_IDENTITY="${CODESIGN_IDENTITY:--}"
BINARY_PATH="${BINARY_PATH:-$ROOT_DIR/target/release/rusty-loader}"
BUNDLE_DIR="${BUNDLE_DIR:-$ROOT_DIR/target/release/bundle}"

if [[ -z "${VERSION:-}" ]]; then
  if [[ -f "$ROOT_DIR/Cargo.toml" ]]; then
    VERSION="$(awk -F'"' '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^version[[:space:]]*=/{print $2; exit}' "$ROOT_DIR/Cargo.toml")"
  fi
fi

if [[ -z "${VERSION:-}" ]]; then
  echo "Error: VERSION could not be determined from environment or Cargo.toml." >&2
  exit 1
fi

PLIST_TEMPLATE="$ROOT_DIR/packaging/macos/Info.plist.template"
LAUNCHER_TEMPLATE="$ROOT_DIR/packaging/macos/launcher.sh.template"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

BUNDLE_APP_PATH="${BUNDLE_DIR}/${APP_NAME}.app"
INSTALLED_APP_PATH="${APP_DIR}/${APP_NAME}.app"

escape_sed_repl() {
  printf '%s' "$1" | sed -e 's/[\\/&]/\\&/g'
}

read_bundle_version() {
  local app_path="$1"
  local plist_path="${app_path}/Contents/Info.plist"
  if [[ ! -f "$plist_path" ]]; then
    echo "unknown"
    return 0
  fi
  local ver
  ver="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$plist_path" 2>/dev/null || true)"
  if [[ -z "$ver" ]]; then
    echo "unknown"
  else
    echo "$ver"
  fi
}

stop_running_app() {
  local installed_app_path="$1"
  local pattern="${installed_app_path}/Contents/MacOS/"

  if ! pgrep -f "$pattern" >/dev/null 2>&1; then
    return 0
  fi

  echo "Stopping running ${APP_NAME} instance in ${installed_app_path}..."
  pkill -TERM -f "$pattern" >/dev/null 2>&1 || true

  local i=0
  while [[ $i -lt 20 ]]; do
    if ! pgrep -f "$pattern" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
    i=$((i + 1))
  done

  if pgrep -f "$pattern" >/dev/null 2>&1; then
    echo "Process did not exit gracefully within 5s; sending SIGKILL..."
    pkill -KILL -f "$pattern" >/dev/null 2>&1 || true
    sleep 0.25
  fi
}

sign_bundle() {
  local app_path="$1"
  local bin_in_bundle="${app_path}/Contents/MacOS/${APP_NAME}"

  codesign --force --sign "$CODESIGN_IDENTITY" "$bin_in_bundle"
  codesign --force --deep --sign "$CODESIGN_IDENTITY" "$app_path"
  codesign --verify --deep --strict "$app_path"
}

refresh_launch_services() {
  local app_path="$1"
  if [[ -x "$LSREGISTER" ]]; then
    "$LSREGISTER" -f "$app_path"
  fi
}

check_writable_dir() {
  local dir="$1"
  local create_if_missing="${2:-false}"

  if [[ "$create_if_missing" == "true" ]]; then
    if ! mkdir -p "$dir" 2>/dev/null; then
      echo "Error: Unable to create directory '${dir}'." >&2
      echo "Hint: Re-run with 'sudo make ...' or use a user-writable directory such as 'APP_DIR=~/Applications'." >&2
      exit 1
    fi
  fi

  if [[ ! -d "$dir" || ! -w "$dir" ]]; then
    echo "Error: Target directory '${dir}' is not writable." >&2
    echo "Hint: Re-run with 'sudo make ...' or use a user-writable directory such as 'APP_DIR=~/Applications'." >&2
    exit 1
  fi
}

atomic_swap_bundle() {
  local src_bundle="$1"
  local dest_bundle="$2"
  local staging_bundle="${APP_DIR}/.${APP_NAME}.app.staging.$$"
  local backup_bundle="${APP_DIR}/.${APP_NAME}.app.backup.$$"

  rm -rf "$staging_bundle" "$backup_bundle"

  if ! ditto "$src_bundle" "$staging_bundle"; then
    rm -rf "$staging_bundle"
    echo "Error: Failed to stage bundle to '${staging_bundle}'." >&2
    return 1
  fi

  if ! sign_bundle "$staging_bundle"; then
    rm -rf "$staging_bundle"
    echo "Error: Code signing failed for '${staging_bundle}'." >&2
    return 1
  fi

  if [[ -e "$dest_bundle" ]]; then
    if ! mv "$dest_bundle" "$backup_bundle"; then
      rm -rf "$staging_bundle"
      echo "Error: Failed to move existing bundle '${dest_bundle}' aside." >&2
      echo "Hint: Re-run with 'sudo make ...' or use 'APP_DIR=~/Applications'." >&2
      return 1
    fi
  fi

  if ! mv "$staging_bundle" "$dest_bundle"; then
    echo "Error: Failed to install staged bundle into '${dest_bundle}'." >&2
    if [[ -e "$backup_bundle" ]]; then
      mv "$backup_bundle" "$dest_bundle" || true
    fi
    rm -rf "$staging_bundle"
    return 1
  fi

  rm -rf "$backup_bundle"
}

cmd_bundle() {
  if [[ ! -f "$BINARY_PATH" ]]; then
    echo "Error: Release binary not found at '${BINARY_PATH}'. Run 'cargo build --release' first." >&2
    exit 1
  fi
  if [[ ! -f "$PLIST_TEMPLATE" ]]; then
    echo "Error: Info.plist template not found at '${PLIST_TEMPLATE}'." >&2
    exit 1
  fi
  if [[ ! -f "$LAUNCHER_TEMPLATE" ]]; then
    echo "Error: Launcher template not found at '${LAUNCHER_TEMPLATE}'." >&2
    exit 1
  fi

  local contents_dir="${BUNDLE_APP_PATH}/Contents"
  local macos_dir="${contents_dir}/MacOS"
  local executable_name="${APP_NAME}-launcher"

  rm -rf "$BUNDLE_APP_PATH"
  mkdir -p "$macos_dir"

  cp "$BINARY_PATH" "${macos_dir}/${APP_NAME}"
  chmod 755 "${macos_dir}/${APP_NAME}"

  local esc_app_name esc_bundle_id esc_version esc_executable
  esc_app_name="$(escape_sed_repl "$APP_NAME")"
  esc_bundle_id="$(escape_sed_repl "$BUNDLE_ID")"
  esc_version="$(escape_sed_repl "$VERSION")"
  esc_executable="$(escape_sed_repl "$executable_name")"

  sed -e "s/{{APP_NAME}}/${esc_app_name}/g" \
    "$LAUNCHER_TEMPLATE" > "${macos_dir}/${executable_name}"
  chmod 755 "${macos_dir}/${executable_name}"

  sed -e "s/{{APP_NAME}}/${esc_app_name}/g" \
      -e "s/{{BUNDLE_ID}}/${esc_bundle_id}/g" \
      -e "s/{{VERSION}}/${esc_version}/g" \
      -e "s/{{EXECUTABLE}}/${esc_executable}/g" \
    "$PLIST_TEMPLATE" > "${contents_dir}/Info.plist"

  local icon_src="${ROOT_DIR}/packaging/macos/${APP_NAME}.icns"
  if [[ -f "$icon_src" ]]; then
    mkdir -p "${contents_dir}/Resources"
    cp "$icon_src" "${contents_dir}/Resources/${APP_NAME}.icns"
    /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string ${APP_NAME}.icns" "${contents_dir}/Info.plist" >/dev/null 2>&1 || \
      /usr/libexec/PlistBuddy -c "Set :CFBundleIconFile ${APP_NAME}.icns" "${contents_dir}/Info.plist" >/dev/null
  fi

  plutil -lint "${contents_dir}/Info.plist" >/dev/null
  sign_bundle "$BUNDLE_APP_PATH"
  echo "Packaged macOS app bundle: ${BUNDLE_APP_PATH} (v${VERSION})"
}

cmd_check_installed() {
  if [[ ! -d "$INSTALLED_APP_PATH" ]]; then
    echo "Error: ${INSTALLED_APP_PATH} is not installed." >&2
    echo "Run 'make macos-install APP_DIR=${APP_DIR}' first." >&2
    exit 1
  fi
}

cmd_install() {
  if [[ ! -d "$BUNDLE_APP_PATH" ]]; then
    echo "Error: Bundled app not found at '${BUNDLE_APP_PATH}'. Run 'make macos-bundle' first." >&2
    exit 1
  fi

  check_writable_dir "$APP_DIR" "true"

  if [[ -d "$INSTALLED_APP_PATH" ]]; then
    stop_running_app "$INSTALLED_APP_PATH"
  fi

  atomic_swap_bundle "$BUNDLE_APP_PATH" "$INSTALLED_APP_PATH"
  refresh_launch_services "$INSTALLED_APP_PATH"

  local installed_version
  installed_version="$(read_bundle_version "$INSTALLED_APP_PATH")"
  echo "Installed ${INSTALLED_APP_PATH} (v${installed_version}, signed with identity '${CODESIGN_IDENTITY}')"
}

cmd_update() {
  cmd_check_installed

  if [[ ! -d "$BUNDLE_APP_PATH" ]]; then
    echo "Error: Bundled app not found at '${BUNDLE_APP_PATH}'. Run 'make macos-bundle' first." >&2
    exit 1
  fi

  local old_version
  old_version="$(read_bundle_version "$INSTALLED_APP_PATH")"

  check_writable_dir "$APP_DIR" "false"
  stop_running_app "$INSTALLED_APP_PATH"
  atomic_swap_bundle "$BUNDLE_APP_PATH" "$INSTALLED_APP_PATH"
  refresh_launch_services "$INSTALLED_APP_PATH"

  local new_version
  new_version="$(read_bundle_version "$INSTALLED_APP_PATH")"
  echo "Updated ${INSTALLED_APP_PATH}: v${old_version} -> v${new_version} (signed with identity '${CODESIGN_IDENTITY}')"
}

cmd_uninstall() {
  if [[ ! -e "$INSTALLED_APP_PATH" ]]; then
    echo "${INSTALLED_APP_PATH} is not installed; nothing to remove."
    return 0
  fi

  check_writable_dir "$APP_DIR" "false"
  stop_running_app "$INSTALLED_APP_PATH"
  rm -rf "$INSTALLED_APP_PATH"
  echo "Removed ${INSTALLED_APP_PATH}"
}

usage() {
  cat <<EOF
Usage: $(basename "$0") <bundle|check-installed|install|update|uninstall>
EOF
}

SUBCOMMAND="${1:-}"
case "$SUBCOMMAND" in
  bundle)
    cmd_bundle
    ;;
  check-installed)
    cmd_check_installed
    ;;
  install)
    cmd_install
    ;;
  update)
    cmd_update
    ;;
  uninstall)
    cmd_uninstall
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
