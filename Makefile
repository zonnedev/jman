SHELL := /usr/bin/env bash

NATIVE_FRONTEND := $(CURDIR)/target/native/libjman_javac_frontend.so
DEBUG_NATIVE_FRONTEND := $(CURDIR)/target/debug/libjman_javac_frontend.so
NATIVE_FRONTEND_INPUTS := \
	$(shell find tools/javac-bridge/src/main -type f) \
	scripts/build-native.sh \
	scripts/use-sdkman-java.sh

.PHONY: gates ci test test-rust test-java test-jman-runner test-processor-worker test-vineflower test-maven-import test-gradle-import test-gradle-annotation-processing test-project-importers test-real-semantics test-lsp test-jpms-correctness test-compatibility-matrix test-micronaut-correctness test-vscode-extension test-neovim-plugin test-release-automation package-vscode release stage-release native test-native clean clear

gates: test test-native test-maven-import test-gradle-import test-gradle-annotation-processing test-project-importers test-real-semantics test-lsp test-vscode-extension test-neovim-plugin

ci: test test-native test-vscode-extension test-release-automation

test: test-rust test-java test-jman-runner

test-rust: $(DEBUG_NATIVE_FRONTEND)
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" \
	cargo test --workspace

test-java:
	./scripts/test-java.sh

test-jman-runner:
	./scripts/build-jman-runner.sh

test-processor-worker:
	./scripts/build-processor-worker.sh

test-vineflower:
	./scripts/build-vineflower.sh
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native" \
	cargo test -p jman-java-lsp --features native-ffi \
		vineflower_fallback_is_content_addressed_and_locates_overload

test-maven-import: test-java
	./scripts/test-maven-import.sh

test-gradle-import:
	./scripts/test-gradle-import.sh

test-gradle-annotation-processing: test-java
	./scripts/test-gradle-annotation-processing.sh

test-project-importers: test-java test-maven-import test-gradle-import
	./scripts/import-maven-project.sh \
		"$(CURDIR)/target/integration-fixtures/spring-petclinic-maven" \
		"$(CURDIR)/target/reusable-petclinic-maven.ndjson"
	./scripts/import-gradle-project.sh \
		"$(CURDIR)/target/integration-fixtures/spring-petclinic" \
		"$(CURDIR)/target/reusable-petclinic-gradle.ndjson"
	grep -q '"buildSystem":"maven"' "$(CURDIR)/target/reusable-petclinic-maven.ndjson"
	grep -q '"buildSystem":"gradle"' "$(CURDIR)/target/reusable-petclinic-gradle.ndjson"

native: $(NATIVE_FRONTEND)

$(NATIVE_FRONTEND): $(NATIVE_FRONTEND_INPUTS)
	./scripts/build-native.sh

$(DEBUG_NATIVE_FRONTEND): $(NATIVE_FRONTEND)
	mkdir -p "$(dir $(DEBUG_NATIVE_FRONTEND))"
	cp "$(NATIVE_FRONTEND)" "$(DEBUG_NATIVE_FRONTEND)"

test-native: native
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	cargo test -p javac-frontend --features native-ffi -- --test-threads=1

test-real-semantics: native test-maven-import
	./scripts/test-real-semantics.sh

test-lsp: native test-maven-import test-gradle-annotation-processing test-processor-worker test-vineflower
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" cargo build -p jman-java-lsp --features native-ffi
	cargo run --quiet -p jman-java-lsp --example subprocess_probe -- \
		"$(CURDIR)/target/debug/jman-java-lsp" \
		"$(CURDIR)/target/integration-fixtures/spring-petclinic-maven" \
		gradle
	cargo run --quiet -p jman-java-lsp --example subprocess_probe -- \
		"$(CURDIR)/target/debug/jman-java-lsp" \
		"$(CURDIR)/target/integration-fixtures/spring-petclinic-maven" \
		maven
	. "$(CURDIR)/scripts/use-sdkman-java.sh"; \
	JAVA_HOME="$$JAVA_HOME" \
	JAVA_LSP_PROCESSOR_WORKER_CLASSPATH="$(CURDIR)/target/processor-worker.jar" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native" \
	cargo run --quiet -p jman-java-lsp --example processor_lsp_probe -- \
		"$(CURDIR)/target/debug/jman-java-lsp" \
		"$(CURDIR)/target/integration-fixtures/gradle-annotation-processing"

test-vscode-extension:
	cd editors/vscode && npm run check

test-neovim-plugin:
	nvim --headless -u NONE -i NONE -l editors/neovim/tests/minimal.lua

test-release-automation:
	./scripts/test-release-automation.sh

test-micronaut-correctness: native test-java test-processor-worker
	./scripts/test-micronaut-correctness.sh

test-jpms-correctness: native test-java test-processor-worker test-maven-import
	./scripts/test-jpms-correctness.sh

test-compatibility-matrix: test-java
	./scripts/test-compatibility-matrix.sh

package-vscode:
	./scripts/package-vscode.sh

release:
	./scripts/package-release.sh

stage-release:
	@test -n "$(TAG)" || { echo "usage: make stage-release TAG=v<version>" >&2; exit 2; }
	./scripts/stage-release-artifacts.sh "$(TAG)"

clean:
	cargo clean

# Remove every reproducible, ignored build artifact produced by this repository.
clear:
	cargo clean
	$(RM) -r -- \
		"$(CURDIR)/.jman" \
		"$(CURDIR)/.local" \
		"$(CURDIR)/target" \
		"$(CURDIR)/editors/vscode/node_modules" \
		"$(CURDIR)/editors/vscode/dist" \
		"$(CURDIR)/editors/vscode/server"
	find "$(CURDIR)" -type d -name __pycache__ -prune -exec rm -rf -- {} +
