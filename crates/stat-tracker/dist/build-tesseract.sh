#!/usr/bin/env bash
# Build pinned Tesseract 5.5.x into a prefix for the release daemon.
#
# Ubuntu 22.04's tesseract-ocr package stays on PATH. --generate-tessdata
# shells out to those 4.1.1 tools. This prefix is libraries and headers only
# (no training tools) and must not be put on PATH.
#
# OpenMP is on, matching the libgomp build the release ships today.
# -march=native is off. libcurl and libarchive are off so they are not
# bundled. Leptonica is the system 1.82. The SONAME is libtesseract.so.5
# (upstream 5.5 cmake would emit libtesseract.so.5.5).
#
# Usage: build-tesseract.sh
# Env:
#   TESSERACT_TAG         default 5.5.3
#   TESSERACT_PREFIX      default $HOME/.local/tesseract-$TAG
#   TESSERACT_SHA256      default the pinned 5.5.3 tarball hash
#   TESSERACT_BUILD_JOBS  default 2
#   TESSERACT_SRC_URL     default the GitHub tag archive
set -euo pipefail

TAG="${TESSERACT_TAG:-5.5.3}"
PREFIX="${TESSERACT_PREFIX:-$HOME/.local/tesseract-${TAG}}"
SHA256="${TESSERACT_SHA256:-9218e62793116d42a9f6d14cd9348518b27f382096eea3d0f2d1a24616bb5884}"
JOBS="${TESSERACT_BUILD_JOBS:-2}"
URL="${TESSERACT_SRC_URL:-https://github.com/tesseract-ocr/tesseract/archive/refs/tags/${TAG}.tar.gz}"

verify_install() {
    local lib soname mod lept_ver
    lept_ver="$(pkg-config --modversion lept)"
    case "${lept_ver}" in
        1.82.*) ;;
        *)
            echo "expected Ubuntu 22.04 leptonica 1.82.x, got ${lept_ver}" >&2
            exit 1
            ;;
    esac
    lib="${PREFIX}/lib/libtesseract.so.5"
    if [[ ! -e "${lib}" ]]; then
        echo "missing ${lib}" >&2
        find "${PREFIX}/lib" -name 'libtesseract.so*' >&2 || true
        exit 1
    fi
    soname="$(readelf -d "${lib}" | awk -F'[][]' '/SONAME/ {print $2; exit}')"
    [[ "${soname}" == "libtesseract.so.5" ]] || {
        echo "SONAME is ${soname:-<none>}, want libtesseract.so.5" >&2
        exit 1
    }
    if ! readelf -d "${lib}" | grep -q 'libgomp'; then
        echo "libtesseract is missing libgomp (OpenMP was not enabled)" >&2
        readelf -d "${lib}" >&2
        exit 1
    fi
    if readelf -d "${lib}" | grep -E 'libcurl|libarchive'; then
        echo "libtesseract must not link libcurl or libarchive" >&2
        exit 1
    fi
    export PKG_CONFIG_PATH="${PREFIX}/lib/pkgconfig${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}"
    mod="$(pkg-config --modversion tesseract)"
    [[ "${mod}" == "${TAG}" ]] || {
        echo "pkg-config tesseract is ${mod}, want ${TAG}" >&2
        exit 1
    }
    echo "Tesseract ${mod} at ${PREFIX} (SONAME ${soname}, leptonica ${lept_ver}, OpenMP on)"
}

if [[ -f "${PREFIX}/lib/pkgconfig/tesseract.pc" && -e "${PREFIX}/lib/libtesseract.so.5" ]]; then
    echo "Tesseract ${TAG} already installed at ${PREFIX}"
    verify_install
    exit 0
fi

lept_ver="$(pkg-config --modversion lept)"
case "${lept_ver}" in
    1.82.*) echo "building against leptonica ${lept_ver}" ;;
    *)
        echo "expected Ubuntu 22.04 leptonica 1.82.x, got ${lept_ver}" >&2
        exit 1
        ;;
esac

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT
tarball="${workdir}/tesseract.tar.gz"
curl -fsSL --retry 3 --connect-timeout 30 --max-time 300 -o "${tarball}" "${URL}"
echo "${SHA256}  ${tarball}" | sha256sum -c -
tar -xzf "${tarball}" -C "${workdir}"
src="$(find "${workdir}" -mindepth 1 -maxdepth 1 -type d -name 'tesseract-*' | head -n 1)"
[[ -n "${src}" && -f "${src}/CMakeLists.txt" ]] || {
    echo "tarball has no CMakeLists.txt" >&2
    exit 1
}

python3 - "${src}/CMakeLists.txt" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = "PROPERTIES SOVERSION\n                          ${VERSION_MAJOR}.${VERSION_MINOR})"
new = "PROPERTIES SOVERSION\n                          ${VERSION_MAJOR})"
if old not in text:
    sys.exit("could not pin SOVERSION to the major version (libtesseract.so.5)")
path.write_text(text.replace(old, new, 1))
PY

build="${workdir}/build"
cmake -S "${src}" -B "${build}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="${PREFIX}" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DBUILD_SHARED_LIBS=ON \
    -DOPENMP_BUILD=ON \
    -DENABLE_NATIVE=OFF \
    -DDISABLE_CURL=ON \
    -DDISABLE_ARCHIVE=ON \
    -DBUILD_TRAINING_TOOLS=OFF \
    -DBUILD_TESTS=OFF \
    -DGRAPHICS_DISABLED=ON \
    -DCMAKE_C_FLAGS_RELEASE="-O2 -DNDEBUG" \
    -DCMAKE_CXX_FLAGS_RELEASE="-O2 -DNDEBUG"

if grep -R -F -q \
    --include='flags.make' --include='build.ninja' --include='link.txt' \
    -- '-march=native' "${build}"; then
    echo "refusing to build Tesseract with -march=native" >&2
    grep -R -n -F \
        --include='flags.make' --include='build.ninja' --include='link.txt' \
        -- '-march=native' "${build}" >&2 || true
    exit 1
fi

cmake --build "${build}" --parallel "${JOBS}"
cmake --install "${build}"
verify_install
