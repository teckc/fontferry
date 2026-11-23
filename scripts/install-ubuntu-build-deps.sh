#!/usr/bin/env bash
set -euo pipefail

# Use the signed Ubuntu archive directly; the runner's regional mirror can
# time out for each package before falling back to this same archive.
sources="$RUNNER_TEMP/fontferry-ubuntu.sources"
sed -E 's@mirror\+file:/etc/apt/apt-mirrors.txt|https?://[^ ]*archive\.ubuntu\.com/ubuntu/?@https://archive.ubuntu.com/ubuntu@g; s@http://security\.ubuntu\.com/ubuntu@https://security.ubuntu.com/ubuntu@g' \
  /etc/apt/sources.list.d/ubuntu.sources > "$sources"
apt=(sudo apt-get -o "Dir::Etc::sourcelist=$sources" -o Dir::Etc::sourceparts=- \
  -o Acquire::Languages=none -o Acquire::Retries=2 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)
"${apt[@]}" -o APT::Update::Error-Mode=any update
"${apt[@]}" install --yes libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf "$@"
