# Bundled media executables

Windows and macOS packaging builds FFmpeg and ffprobe from pinned source before the package is assembled by `scripts/package-gpui.mts`. The application launches these as separate executables; it does not link FFmpeg or x264 into the Rust host. No decoder is downloaded at application runtime.

`scripts/package-ffmpeg.mts` pins FFmpeg 8.1.1 from [FFmpeg's source releases](https://ffmpeg.org/releases/), x264 commit `b35605ace3ddf7c1a5d67a2eb553f034aef41d55` from the [x264 mirror](https://github.com/mirror/x264) of [VideoLAN's repository](https://code.videolan.org/videolan/x264), and zlib 1.3.1 from [zlib's source archive](https://zlib.net/fossils/). It checks SHA-256 for all three complete source archives. Git archive is explicitly configured without line-ending conversion so the source hash is identical on Windows and Unix.

The build retains FFmpeg's built-in codecs, demuxers, filters and protocols, enables static x264 for the existing H.264 recording contract, and enables static zlib for PNG/deflate support. External-library autodetection is disabled. Nonfree and GPLv3-only options are disabled. These separate FFmpeg executables are GPL-2.0-or-later components; the npm dependency license allowlist is unchanged. See the included FFmpeg, x264 and zlib license texts and [FFmpeg's licensing information](https://ffmpeg.org/legal.html).

Every NSIS installation and Windows ZIP contains `ffmpeg.exe`, `ffprobe.exe` and `ffmpeg-source/`. A macOS application contains the executables in `Contents/MacOS/` and corresponding sources in `Contents/Resources/ffmpeg-source/`. The source directory contains the complete unmodified archives, license texts, build recipe, configure logs and a version/architecture/source-checksum manifest. Recipients can rebuild and redistribute these separate components under their licenses. Retain this directory when redistributing the executables. A bare Windows application EXE also needs these sibling files; use the ZIP or installer for distribution.

The source materials also retain compiler/runtime license notices and the actual compiler version. GNU MinGW builds include the installed MinGW copyright notices, GCC Runtime Library Exception and referenced license texts. LLVM-MinGW builds require the toolchain's LLVM license and its architecture-specific MinGW runtime notices. The repository retains the official [LLVM compiler-rt license](https://github.com/llvm/llvm-project/blob/llvmorg-20.1.8/compiler-rt/LICENSE.TXT), including LLVM exceptions and legacy notices, for macOS compiler helpers as well.

The packager checks executable architecture and imports. Windows tools may import only explicitly recognized Windows system DLLs. macOS tools may import only `/System/Library` frameworks and `/usr/lib` libraries; Homebrew and other unbundled dylibs are rejected. Native build hosts also run both tools and verify the pinned version and GPL/x264 configuration. Cross builds perform static verification and require later native runtime acceptance; they do not claim an execution test.

## Build prerequisites

On macOS, install Xcode command-line tools, make, nasm and pkg-config. Build on the matching architecture with `npm run package:gpui:macos -- --x64` or `--arm64`. The packager places both tools in `Contents/MacOS` and seals them as part of the selected ad-hoc or optional Developer ID signing route.

On Windows, the local build route uses WSL distro `Ubuntu-24.04`, with `make`, `pkg-config`, `gcc-mingw-w64-x86-64-posix` and `nasm`. `PRESENTER_ORBIT_FFMPEG_WSL_DISTRIBUTION` selects another explicitly provisioned distro. The source is compiled in an owned temporary Linux directory; completed tools and source materials are written to the ignored `artifacts/package-ffmpeg/windows-x64/` directory. The script never installs compilers or modifies the distro itself.

Linux build workers can prepare Windows binaries directly:

```sh
node scripts/package-ffmpeg.mts --platform windows --x64
node scripts/package-ffmpeg.mts --platform windows --arm64
```

Windows ARM64 requires an LLVM-MinGW toolchain exposing `aarch64-w64-mingw32-clang`, `ar` and `ranlib` on PATH. The optional `PRESENTER_ORBIT_FFMPEG_MINGW_PREFIX` names an explicitly installed cross-toolchain prefix. ARM64 is never substituted with an x64 binary. A matching prepared directory can be transferred to a Windows build worker and selected using absolute `PRESENTER_ORBIT_FFMPEG_PREPARED_ROOT`; packaging verifies the source, recipe and architecture before using it.

The generated manifest allows an unchanged prepared build to be reused. A changed build recipe requires deliberately removing that generated architecture directory and rebuilding. Hash/configuration failures stop packaging instead of downloading a substitute. Successful preparation alone does not establish installed application startup, recording or persistence: those are verified by the separate package acceptance tests.

Linux packages retain their existing distribution FFmpeg dependency contract.
