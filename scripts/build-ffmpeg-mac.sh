#!/bin/sh
set -eu
arch="${1:-arm64}"
case "$arch" in arm64) ffarch=aarch64; ccarch=arm64;; x64) ffarch=x86_64; ccarch=x86_64;; *) exit 1;; esac
root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
mkdir -p "$root/work/ffmpeg-$arch"
cd "$root/work/ffmpeg-$arch"
"$root/work/ffmpeg-7.1.1/configure" \
  --arch="$ffarch" --target-os=darwin --enable-cross-compile \
  --cc="clang -arch $ccarch" --extra-cflags=-mmacosx-version-min=12.0 --extra-ldflags=-mmacosx-version-min=12.0 \
  --disable-autodetect --disable-doc --disable-debug --disable-ffplay \
  --disable-x86asm --disable-shared --enable-static --enable-small
make -j6
cp ffmpeg ffprobe "$root/vendor/mac-$arch/"
cp "$root/work/ffmpeg-7.1.1/COPYING.LGPLv2.1" "$root/vendor/mac-$arch/FFmpeg-LICENSE.txt"
printf '%s\n' 'FFmpeg 7.1.1. Source: https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz' 'Build: scripts/build-ffmpeg-mac.sh in the accompanying source archive.' > "$root/vendor/mac-$arch/FFmpeg-README.txt"
