#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
petclinic="${project_dir}/target/integration-fixtures/spring-petclinic-maven"
gson="${project_dir}/target/integration-fixtures/gson-maven"
local_repository="${project_dir}/target/maven-repository"

(
  cd "${gson}"
  export JAVA_HOME="/home/jfsanchez/.sdkman/candidates/java/21.0.2-open"
  export PATH="${JAVA_HOME}/bin:${PATH}"
  mvn \
    --batch-mode \
    --no-transfer-progress \
    -q \
    "-Dmaven.repo.local=${local_repository}" \
    -Denforcer.skip=true \
    -DskipTests \
    -pl gson \
    generate-sources
)

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo run \
    --quiet \
    -p javac-frontend \
    --features native-ffi \
    --example semantic_probe \
    -- \
    "${petclinic}/src/main/java/org/springframework/samples/petclinic/PetClinicApplication.java" \
    17 \
    "${petclinic}/target/javac-frontend-classpath.txt" \
    "${petclinic}/src/main/java" \
    org.springframework.boot.SpringApplication

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo run \
    --quiet \
    -p javac-frontend \
    --features native-ffi \
    --example semantic_probe \
    -- \
    "${gson}/gson/src/main/java/com/google/gson/GsonBuilder.java" \
    8 \
    "${gson}/gson/target/javac-frontend-classpath.txt" \
    "${gson}/gson/src/main/java:${gson}/gson/target/generated-sources/java-templates" \
    com.google.errorprone.annotations.CanIgnoreReturnValue

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo run \
    --quiet \
    -p javac-frontend \
    --features native-ffi \
    --example navigation_probe \
    -- \
    "${petclinic}/src/main/java/org/springframework/samples/petclinic/owner/Owner.java" \
    "${petclinic}/src/main/java/org/springframework/samples/petclinic/owner/OwnerController.java" \
    17 \
    "${petclinic}/target/javac-frontend-classpath.txt" \
    "${petclinic}/src/main/java" \
    org.springframework.samples.petclinic.owner.Owner \
    owner

JAVAC_FRONTEND_LIB_DIR="${project_dir}/target/native" \
  cargo run \
    --quiet \
    -p javac-frontend \
    --features native-ffi \
    --example navigation_probe \
    -- \
    "${gson}/gson/src/main/java/com/google/gson/Gson.java" \
    "${gson}/gson/src/main/java/com/google/gson/GsonBuilder.java" \
    8 \
    "${gson}/gson/target/javac-frontend-classpath.txt" \
    "${gson}/gson/src/main/java:${gson}/gson/target/generated-sources/java-templates" \
    com.google.gson.Gson \
    gson
