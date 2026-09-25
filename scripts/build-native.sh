#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=use-test-java.sh
source "${project_dir}/scripts/use-test-java.sh"
classes_dir="${project_dir}/target/native-classes"
native_dir="${project_dir}/target/native"
platform_dir="${native_dir}/platform"

rm -rf "${classes_dir}" "${native_dir}"
mkdir -p "${classes_dir}" "${native_dir}" "${platform_dir}/lib" "${platform_dir}/legal"
if [[ ! -f "${JAVA_HOME}/lib/ct.sym" ]]; then
  printf 'GraalVM compiler platform is missing: %s\n' "${JAVA_HOME}/lib/ct.sym" >&2
  exit 1
fi
cp "${JAVA_HOME}/lib/ct.sym" "${platform_dir}/lib/ct.sym"
for legal_file in LICENSE ADDITIONAL_LICENSE_INFO ASSEMBLY_EXCEPTION; do
  cp "${JAVA_HOME}/legal/java.base/${legal_file}" "${platform_dir}/legal/${legal_file}"
done

mapfile -t sources < <(find "${project_dir}/tools/javac-bridge/src/main/java" -name '*.java' -print | sort)
javac -Werror -Xlint:all \
  --add-exports jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED \
  --add-exports jdk.compiler/com.sun.tools.javac.parser=ALL-UNNAMED \
  --add-exports jdk.compiler/com.sun.tools.javac.util=ALL-UNNAMED \
  -d "${classes_dir}" "${sources[@]}"
cp -R "${project_dir}/tools/javac-bridge/src/main/resources/." "${classes_dir}/"
image_build_digest="$(
  cd "${classes_dir}"
  find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1
)"
image_build_id="${image_build_digest:0:8}-${image_build_digest:8:4}-${image_build_digest:12:4}-${image_build_digest:16:4}-${image_build_digest:20:12}"

(
  cd "${native_dir}"
  native-image \
    -J--add-exports=jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED \
    -J--add-exports=jdk.compiler/com.sun.tools.javac.parser=ALL-UNNAMED \
    -J--add-exports=jdk.compiler/com.sun.tools.javac.util=ALL-UNNAMED \
    --shared \
    --no-fallback \
    -march=compatibility \
    -H:+UnlockExperimentalVMOptions \
    -H:ImageBuildID="${image_build_id}" \
    -H:IncludeResourceBundles=jdk.compiler:com.sun.tools.javac.resources.compiler,jdk.compiler:com.sun.tools.javac.resources.javac \
    -H:-UnlockExperimentalVMOptions \
    -cp "${classes_dir}" \
    -o libjman_javac_frontend \
    io.github.zonnedev.jman.javac.NativeBridge
)
