#!/bin/sh

get_os_arch() {
  local os_name
  local arch_name

  os_name=$(uname -s | tr '[:upper:]' '[:lower:]')
  arch_name=$(uname -m)

  if [ "$os_name" != "linux" ]; then
    echo "Komandan currently only supports Linux. Your operating system: $os_name"
    exit 1
  fi

  if [ "$arch_name" != "x86_64" ]; then
    echo "Komandan currently only supports x86_64 architecture. Your architecture: $arch_name"
    exit 1
  fi

  echo "${os_name}_${arch_name}"
}

install_komandan() {
  local os_arch
  local download_url
  local temp_dir
  local release_tag
  local install_dir

  os_arch=$(get_os_arch)

  release_tag=$(curl -s "https://api.github.com/repos/hahnavi/komandan/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

  if [ -z "$release_tag" ]; then
    echo "Could not find the latest release tag."
    exit 1
  fi

  file_name="komandan_$release_tag-$os_arch.zip"
  download_url="https://github.com/hahnavi/komandan/releases/download/$release_tag/$file_name"
  temp_dir=$(mktemp -d)
  install_dir="$HOME/.local/bin"

  mkdir -p "$install_dir"

  echo "Downloading Komandan from $download_url"
  curl -sSL "$download_url" -o "$temp_dir/$file_name"

  if [ $? -ne 0 ]; then
    echo "Failed to download Komandan."
    rm -rf "$temp_dir"
    exit 1
  fi

  unzip -q -o "$temp_dir/$file_name" -d "$install_dir"
  chmod +x "$install_dir/komandan"

  rm -rf "$temp_dir"

  echo "Komandan installed successfully to $install_dir"

  # Check if $install_dir is already in PATH
  if ! echo "$PATH" | grep -q "$install_dir"; then
    echo "Please add $install_dir to your PATH environment variable."
    echo "You can do this by adding the following line to your shell's configuration file:"
    echo ""
    echo "  export PATH=\"$install_dir:\$PATH\""
    echo ""
    echo "For bash, add it to ~/.bashrc"
    echo "For zsh, add it to ~/.zshrc"
    echo "For fish, add it to ~/.config/fish/config.fish"
    echo ""
    echo "After updating the configuration file, run 'source' on it or restart your shell."
  fi
}

install_komandan
