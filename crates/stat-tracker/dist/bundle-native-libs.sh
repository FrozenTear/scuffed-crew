#!/usr/bin/env bash
# Bundle the shared-library closure of a seed ELF into a lib dir and stamp each
# bundled lib with RPATH $ORIGIN. Shared by both jobs of the stat-tracker
# release workflow so the daemon and the GUI bundle libs the same way:
#   - daemon: seed = the binary  -> pulls the tesseract/leptonica/codec closure
#   - GUI:    seed = libxdo       -> pulls the one soname-fragmented lib Arch
#             can't provide (Ubuntu ships libxdo.so.3, Arch libxdo.so.4)
#
# Host-provided families (glibc, loader, toolchain runtimes, ubiquitous
# compression, OpenSSL, and the session/graphics stack) are never bundled.
# Callers pass an extra deny regex to widen the exclusion for their own host
# stack.
#
# Usage:
#   bundle-native-libs.sh <seed-elf> <out-lib-dir> [extra-deny-regex]
#   bundle-native-libs.sh --print-deny [extra-deny-regex]
#
# The seed binary's own RPATH is left to the caller. v0.4.1+ stamps:
#   daemon  $ORIGIN/../lib/scuffed-stat-tracker/ocr
#   GUI     $ORIGIN/../lib/scuffed-stat-tracker/gui
#   libs    $ORIGIN
# so OCR deps never sit on the GUI RUNPATH (v0.4.0 shipped a flat
# $ORIGIN/../lib that included Ubuntu 22.04 libcrypto.so.3 and shadowed
# system OpenSSL — OPENSSL_3.2.0 not found on Aerynos).
set -euo pipefail

# Host-provided libs (never bundle): glibc family, loader, toolchain runtimes,
# ubiquitous compression, OpenSSL, and session/graphics stack.
# libcrypto/libssl MUST stay host-provided: a 22.04-built libcrypto.so.3 only
# exports OPENSSL_3.0.* and, if placed on a binary's RUNPATH, wins over a
# newer /usr/lib/libcrypto.so.3 (needed by libcryptsetup and friends).
DENY='^(ld-linux|libc\.|libm\.|libdl\.|libpthread\.|librt\.|libresolv\.|libnsl\.|libutil\.|libgcc_s|libstdc\+\+|libz\.|libsystemd|libselinux|libwayland|libxkbcommon|libudev|libevdev|libEGL|libGL|libgbm|libdrm|libcrypto\.|libssl\.)'

print_deny() {
  local extra_deny="${1:-}"
  if [ -n "${extra_deny}" ]; then
    echo "${DENY}|${extra_deny}"
  else
    echo "${DENY}"
  fi
}

if [ "${1:-}" = "--print-deny" ]; then
  print_deny "${2:-}"
  exit 0
fi

seed="${1:?usage: bundle-native-libs.sh <seed-elf> <out-lib-dir> [extra-deny-regex]}"
out_lib="${2:?usage: bundle-native-libs.sh <seed-elf> <out-lib-dir> [extra-deny-regex]}"
extra_deny="${3:-}"
DENY="$(print_deny "${extra_deny}")"

mkdir -p "${out_lib}"

# BFS the dependency closure starting from the seed; bundle everything not
# denied, with consistent versions.
queue="${seed}"
seen=""
while [ -n "${queue}" ]; do
  next=""
  for f in ${queue}; do
    for dep in $(ldd "$f" 2>/dev/null | awk '/=>/ {print $3}' | grep -v '^$'); do
      base="$(basename "$dep")"
      echo "${seen}" | grep -qF " ${base} " && continue
      seen="${seen} ${base} "
      if echo "${base}" | grep -qE "${DENY}"; then
        continue
      fi
      cp -n "$dep" "${out_lib}"/
      next="${next} $dep"
    done
  done
  queue="${next}"
done

# Belt: refuse even if DENY is later edited incorrectly.
openssl_hits="$(find "${out_lib}" -maxdepth 1 \( -name 'libcrypto.so*' -o -name 'libssl.so*' \) -print 2>/dev/null || true)"
if [ -n "${openssl_hits}" ]; then
  echo "refusing to bundle OpenSSL (must stay host-provided):" >&2
  echo "${openssl_hits}" >&2
  exit 1
fi

for lib in "${out_lib}"/*; do
  [ -e "$lib" ] || continue
  # $ORIGIN must stay literal — the dynamic loader expands it at runtime.
  # shellcheck disable=SC2016
  patchelf --set-rpath '$ORIGIN' "$lib" || true
done
# shellcheck disable=SC2012  # controlled lib names, ls is fine here
echo "Bundled $(ls "${out_lib}" | wc -l) libraries into ${out_lib}:"
# shellcheck disable=SC2012
ls "${out_lib}"
