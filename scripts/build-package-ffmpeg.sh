#!/usr/bin/env bash
set -euo pipefail

# Inputs are verified by package-ffmpeg.mts before this build starts.
source_dir=$1
output_dir=$2
platform=$3
architecture=$4
command -v pkg-config >/dev/null
mkdir -p "$source_dir/toolchain-licenses"
build_dir=$(mktemp -d "${TMPDIR:-/tmp}/orbit-ffmpeg.XXXXXXXX")
cleanup() {
  status=$?
  if [[ $status != 0 ]]; then
    [[ ! -f "$build_dir/ffmpeg/ffbuild/config.log" ]] || cp "$build_dir/ffmpeg/ffbuild/config.log" "$source_dir/build-failure.log"
  fi
  rm -rf -- "$build_dir"
  exit "$status"
}
trap cleanup EXIT
mkdir "$build_dir/ffmpeg" "$build_dir/x264" "$build_dir/zlib" "$build_dir/install"
tar -xf "$source_dir/ffmpeg-8.1.1.tar.xz" -C "$build_dir/ffmpeg" --strip-components=1
tar -xf "$source_dir/x264.tar" -C "$build_dir/x264"
tar -xf "$source_dir/zlib-1.3.1.tar.gz" -C "$build_dir/zlib" --strip-components=1
prefix="$build_dir/install"
jobs=${PRESENTER_ORBIT_FFMPEG_JOBS:-4}
ffmpeg_flags=(--disable-autodetect --enable-gpl --enable-libx264 --enable-zlib --enable-static --disable-shared --disable-debug --disable-doc --disable-ffplay --disable-nonfree --disable-version3)
x264_flags=(--enable-static --disable-cli --disable-opencl)
if [[ "$platform" == windows ]]; then
  if [[ "$architecture" == x64 ]]; then
    cross_prefix=${PRESENTER_ORBIT_FFMPEG_MINGW_PREFIX:-x86_64-w64-mingw32-}
    export CC="${cross_prefix}gcc"
    cpu=x86_64
  else
    cross_prefix=${PRESENTER_ORBIT_FFMPEG_MINGW_PREFIX:-aarch64-w64-mingw32-}
    export CC="${cross_prefix}clang"
    cpu=aarch64
    x264_flags+=(--disable-asm)
  fi
  export AR="${cross_prefix}ar" RANLIB="${cross_prefix}ranlib"
  command -v "$CC" >/dev/null
  if [[ -n ${PRESENTER_ORBIT_FFMPEG_TOOLCHAIN_LICENSES:-} ]]; then
    cp -R "$PRESENTER_ORBIT_FFMPEG_TOOLCHAIN_LICENSES/." "$source_dir/toolchain-licenses/"
  elif [[ "$architecture" == x64 && -f /usr/share/doc/gcc-mingw-w64-x86-64-posix/copyright ]]; then
    cp /usr/share/doc/gcc-mingw-w64-x86-64-posix/copyright "$source_dir/toolchain-licenses/GCC-copyright.txt"
    cp /usr/share/doc/mingw-w64-common/copyright "$source_dir/toolchain-licenses/MinGW-w64-copyright.txt"
    for license in GPL-2 GPL-3 LGPL-2 LGPL-2.1 LGPL-3 GFDL-1.2 Artistic; do
      cp "/usr/share/common-licenses/$license" "$source_dir/toolchain-licenses/$license.txt"
    done
  else
    echo 'Set PRESENTER_ORBIT_FFMPEG_TOOLCHAIN_LICENSES to the complete runtime license directory for the installed MinGW toolchain.' >&2
    exit 1
  fi
  "$CC" --version > "$source_dir/compiler-version.txt"
  x264_flags+=(--host="${cross_prefix%-}" --cross-prefix="$cross_prefix" --extra-ldflags=-static)
  ffmpeg_flags+=(--target-os=mingw32 --arch="$cpu" --cc="$CC" --enable-cross-compile --cross-prefix="$cross_prefix" --disable-pthreads --enable-w32threads --extra-ldflags=-static)
elif [[ "$platform" == macos ]]; then
  [[ $(uname -s) == Darwin ]] || { echo 'macOS FFmpeg must be built on macOS.' >&2; exit 1; }
  export MACOSX_DEPLOYMENT_TARGET=10.15
  clang --version > "$source_dir/compiler-version.txt"
  if [[ "$architecture" == arm64 ]]; then
    export MACOSX_DEPLOYMENT_TARGET=11.0
    ffmpeg_flags+=(--arch=aarch64 --cc=clang --extra-cflags=-arch\ arm64 --extra-ldflags=-arch\ arm64)
    x264_flags+=(--host=aarch64-apple-darwin --extra-cflags=-arch\ arm64 --extra-ldflags=-arch\ arm64)
  else
    ffmpeg_flags+=(--arch=x86_64 --cc=clang --extra-cflags=-arch\ x86_64 --extra-ldflags=-arch\ x86_64)
    x264_flags+=(--host=x86_64-apple-darwin --extra-cflags=-arch\ x86_64 --extra-ldflags=-arch\ x86_64)
  fi
else
  echo 'Only Windows and macOS use this bundled FFmpeg build.' >&2; exit 1
fi
cd "$build_dir/zlib"
./configure --static --prefix="$prefix"
make -j "$jobs"
make install
cd "$build_dir/x264"
./configure --prefix="$prefix" "${x264_flags[@]}"
make -j "$jobs"
make install-lib-static install-lib-dev
cd "$build_dir/ffmpeg"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
./configure --prefix="$prefix" --pkg-config=pkg-config --pkg-config-flags=--static "${ffmpeg_flags[@]}"
extension=''
[[ "$platform" != windows ]] || extension=.exe
make -j "$jobs" "ffmpeg$extension" "ffprobe$extension"
mkdir -p "$output_dir"
cp "ffmpeg$extension" "ffprobe$extension" "$output_dir/"
cp "$build_dir/ffmpeg/COPYING.GPLv2" "$source_dir/FFmpeg-COPYING.GPLv2"
cp "$build_dir/ffmpeg/LICENSE.md" "$source_dir/FFmpeg-LICENSE.md"
cp "$build_dir/x264/COPYING" "$source_dir/x264-COPYING"
cp "$build_dir/zlib/LICENSE" "$source_dir/zlib-LICENSE"
cp "$build_dir/ffmpeg/ffbuild/config.log" "$source_dir/ffmpeg-config.log"
cp "$build_dir/x264/config.log" "$source_dir/x264-config.log"
