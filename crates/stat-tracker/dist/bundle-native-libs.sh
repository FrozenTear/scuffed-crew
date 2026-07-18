#!/usr/bin/env bash
# Bundle the shared-library closure of a seed ELF into a lib dir and stamp each
# bundled lib with RPATH $ORIGIN. Shared by both jobs of the stat-tracker
# release workflow so the daemon and the GUI bundle libs the same way:
#   - daemon: seed = the binary  -> pulls the tesseract/leptonica/codec closure
#   - GUI:    seed = libxdo       -> pulls the one soname-fragmented lib Arch
#             can't provide (Ubuntu ships libxdo.so.3, Arch libxdo.so.4)
#
# Host-provided families (glibc, loader, toolchain runtimes, ubiquitous
# compression, and the session/graphics stack) are never bundled. Callers pass
# an extra deny regex to widen the exclusion for their own host stack.
#
# Usage: bundle-native-libs.sh <seed-elf> <out-lib-dir> [extra-deny-regex]
#
# The seed binary's own RPATH is left to the caller (the daemon and GUI want
# $ORIGIN/../lib on the binary, $ORIGIN on the libs); this script only copies
# and stamps the bundled libraries.
set -euo pipefail

seed="${1:?usage: bundle-native-libs.sh <seed-elf> <out-lib-dir> [extra-deny-regex]}"
out_lib="${2:?usage: bundle-native-libs.sh <seed-elf> <out-lib-dir> [extra-deny-regex]}"
extra_deny="${3:-}"

mkdir -p "${out_lib}"

# Host-provided libs (never bundle): glibc family, loader, toolchain runtimes,
# ubiquitous compression, and session/graphics stack.
DENY='^(ld-linux|libc\.|libm\.|libdl\.|libpthread\.|librt\.|libresolv\.|libnsl\.|libutil\.|libgcc_s|libstdc\+\+|libz\.|libsystemd|libselinux|libwayland|libxkbcommon|libudev|libevdev|libEGL|libGL|libgbm|libdrm)'
if [ -n "${extra_deny}" ]; then
  DENY="${DENY}|${extra_deny}"
fi

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
