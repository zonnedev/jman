SHELL := /usr/bin/env bash
UV ?= uv
DOCS_RUN := $(UV) run --isolated --no-project --with-requirements docs/requirements.txt

NATIVE_FRONTEND := $(CURDIR)/target/native/libjman_javac_frontend.so
DEBUG_NATIVE_FRONTEND := $(CURDIR)/target/debug/libjman_javac_frontend.so
NATIVE_FRONTEND_INPUTS := \
	$(shell find tools/javac-bridge/src/main -type f) \
	scripts/build-native.sh \
	scripts/setup-test-jdks.sh \
	scripts/use-test-java.sh

.PHONY: gates release-gates ci test test-toolchain-bootstrap test-rust test-java test-jman-runner test-processor-worker vineflower jacoco maven-tool compatibility-tools test-coverage test-vineflower test-maven-import test-gradle-import test-gradle-annotation-processing test-project-importers test-publishing test-real-semantics test-lsp test-jpms-correctness test-compatibility-matrix test-vscode-extension test-neovim-plugin test-installer test-release-automation test-docs docs serve-docs prepare-release package-vscode release stage-release native test-native clean clear

gates: test test-native test-coverage test-maven-import test-gradle-import test-gradle-annotation-processing test-project-importers test-publishing test-real-semantics test-lsp test-vscode-extension test-neovim-plugin test-installer test-docs

release-gates: gates test-compatibility-matrix test-jpms-correctness

ci: test test-native test-coverage test-publishing test-vscode-extension test-neovim-plugin test-installer test-release-automation test-docs

test: test-toolchain-bootstrap test-rust test-java test-jman-runner

test-toolchain-bootstrap:
	./scripts/test-toolchain-bootstrap.sh

test-rust: $(DEBUG_NATIVE_FRONTEND) vineflower jacoco
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" \
	./scripts/run-test-command.sh cargo test --workspace

test-java:
	./scripts/test-java.sh

test-jman-runner:
	./scripts/build-jman-runner.sh

test-processor-worker:
	./scripts/build-processor-worker.sh

vineflower:
	./scripts/build-vineflower.sh

jacoco:
	./scripts/build-jacoco.sh

maven-tool:
	./scripts/setup-compatibility-tools.sh --maven

compatibility-tools:
	./scripts/setup-compatibility-tools.sh --all

test-coverage: jacoco
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	JMAN_JACOCO_AGENT="$(CURDIR)/target/jacoco-0.8.15-agent.jar" \
	JMAN_JACOCO_CLI="$(CURDIR)/target/jacoco-0.8.15-cli.jar" \
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" \
	./scripts/run-test-command.sh cargo test -p jman-build coverage_ -- --nocapture
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	JMAN_JACOCO_AGENT="$(CURDIR)/target/jacoco-0.8.15-agent.jar" \
	JMAN_JACOCO_CLI="$(CURDIR)/target/jacoco-0.8.15-cli.jar" \
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" \
	./scripts/run-test-command.sh cargo test -p jman-cli compiles_tests_and_launches_junit_platform_console -- --nocapture

test-vineflower: vineflower
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native" \
	./scripts/run-test-command.sh cargo test -p jman-java-lsp --features native-ffi \
		vineflower_fallback_is_content_addressed_and_locates_overload

test-maven-import: test-java maven-tool
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

test-publishing: $(DEBUG_NATIVE_FRONTEND) vineflower compatibility-tools
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native$${LD_LIBRARY_PATH:+:$${LD_LIBRARY_PATH}}" \
	cargo build -p jman-cli
	./scripts/test-publishing.sh

native: $(NATIVE_FRONTEND)

$(NATIVE_FRONTEND): $(NATIVE_FRONTEND_INPUTS)
	./scripts/build-native.sh

$(DEBUG_NATIVE_FRONTEND): $(NATIVE_FRONTEND)
	mkdir -p "$(dir $(DEBUG_NATIVE_FRONTEND))"
	cp "$(NATIVE_FRONTEND)" "$(DEBUG_NATIVE_FRONTEND)"

test-native: native
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" \
	./scripts/run-test-command.sh cargo test -p javac-frontend --features native-ffi -- --test-threads=1

test-real-semantics: native test-maven-import
	./scripts/test-real-semantics.sh

test-lsp: native test-maven-import test-gradle-annotation-processing test-processor-worker test-vineflower
	JAVAC_FRONTEND_LIB_DIR="$(CURDIR)/target/native" ./scripts/run-test-command.sh cargo build -p jman-java-lsp --features native-ffi
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	./scripts/run-test-command.sh cargo run --quiet -p jman-java-lsp --example subprocess_probe -- \
		"$(CURDIR)/target/debug/jman-java-lsp" \
		"$(CURDIR)/target/integration-fixtures/spring-petclinic-maven" \
		gradle
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	./scripts/run-test-command.sh cargo run --quiet -p jman-java-lsp --example subprocess_probe -- \
		"$(CURDIR)/target/debug/jman-java-lsp" \
		"$(CURDIR)/target/integration-fixtures/spring-petclinic-maven" \
		maven
	. "$(CURDIR)/scripts/use-test-java.sh"; \
	JAVA_HOME="$$JAVA_HOME" \
	JAVA_LSP_PROCESSOR_WORKER_CLASSPATH="$(CURDIR)/target/processor-worker.jar" \
	LD_LIBRARY_PATH="$(CURDIR)/target/native" \
	./scripts/run-test-command.sh cargo run --quiet -p jman-java-lsp --example processor_lsp_probe -- \
		"$(CURDIR)/target/debug/jman-java-lsp" \
		"$(CURDIR)/target/integration-fixtures/gradle-annotation-processing"

test-vscode-extension:
	cd editors/vscode && npm run check

test-neovim-plugin:
	@for test in editors/neovim/tests/*.lua; do \
		nvim --headless -u NONE -i NONE -l "$$test" || exit $$?; \
	done

test-installer:
	./scripts/test-install.sh

test-release-automation:
	./scripts/test-external-fixtures.sh
	./scripts/test-prepare-release.sh
	./scripts/test-release-automation.sh

test-docs:
	./scripts/test-docs.sh

docs:
	$(DOCS_RUN) mkdocs build --strict

serve-docs:
	$(DOCS_RUN) mkdocs serve

prepare-release:
	@test -n "$(VERSION)" || { echo "usage: make prepare-release VERSION=<version>" >&2; exit 2; }
	./scripts/prepare-release.sh "$(VERSION)"

test-jpms-correctness: native test-java test-processor-worker test-maven-import
	./scripts/test-jpms-correctness.sh

test-compatibility-matrix: test-java compatibility-tools
	./scripts/test-compatibility-matrix.sh

package-vscode: native jacoco
	./scripts/package-vscode.sh

release: native jacoco
	./scripts/package-release.sh

stage-release:
	@test -n "$(TAG)" || { echo "usage: make stage-release TAG=v<version>" >&2; exit 2; }
	./scripts/stage-release-artifacts.sh "$(TAG)"

package: release package-vscode

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
