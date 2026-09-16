"use strict";

const path = require("path");
const fs = require("fs");
const { parseStringPromise } = require("xml2js");
const childProcess = require("child_process");
const vscode = require("vscode");
const {
  LanguageClient,
  TransportKind,
} = require("vscode-languageclient/node");

let client;

function workspaceRoot() {
  const uri = vscode.window.activeTextEditor?.document.uri;
  return (uri && vscode.workspace.getWorkspaceFolder(uri)) || vscode.workspace.workspaceFolders?.[0];
}

function executionCommand(provider, jmanCommand, root) {
  if (provider === "jman") return jmanCommand;
  const wrapper = provider === "gradle" ? "gradlew" : "mvnw";
  const executable = process.platform === "win32" ? `${wrapper}.cmd` : wrapper;
  const candidate = path.join(root, executable);
  return fs.existsSync(candidate) ? candidate : provider;
}

function operationArguments(buildSystem, operation) {
  const operations = {
    jman: { check: ["check"], build: ["build"], test: ["test"], run: ["run"] },
    gradle: { check: ["check"], build: ["build"], test: ["test"], run: ["run"] },
    maven: { check: ["test", "-DskipTests"], build: ["package"], test: ["test"] },
  };
  return operations[buildSystem]?.[operation];
}

async function prepareProjectOperation(operation, jmanCommand) {
  const activeUri = vscode.window.activeTextEditor?.document.uri.toString();
  const status = await client?.sendRequest("workspace/executeCommand", {
    command: "jman.java.status",
    arguments: activeUri ? [activeUri] : [],
  });
  const buildSystem = status?.workspace?.buildSystem;
  const folder = workspaceRoot();
  if (!folder || !["jman", "gradle", "maven"].includes(buildSystem)) return null;
  const args = operationArguments(buildSystem, operation);
  if (!args) return null;
  return {
    command: executionCommand(buildSystem, jmanCommand, folder.uri.fsPath),
    args,
    buildSystem,
  };
}

function createJmanTask(command, operation, args = []) {
  const folder = workspaceRoot();
  if (!folder) return null;
  return new vscode.Task(
    { type: "jman", operation },
    folder,
    `JMAN: ${operation}`,
    "jman",
    new vscode.ProcessExecution(command, [operation, folder.uri.fsPath, ...args]),
  );
}

async function executeJmanTask(command, operation, args = []) {
  const task = createJmanTask(command, operation, args);
  if (!task) {
    vscode.window.showWarningMessage("Open a workspace before running JMAN.");
    return;
  }
  return vscode.tasks.executeTask(task);
}

async function executeNativeJmanTask(command, operation, args = []) {
  const activeUri = vscode.window.activeTextEditor?.document.uri.toString();
  const status = await client?.sendRequest("workspace/executeCommand", {
    command: "jman.java.status",
    arguments: activeUri ? [activeUri] : [],
  });
  if (!status?.workspace?.nativeOperations) {
    vscode.window.showWarningMessage(
      `JMAN ${operation} is available for native jman.toml workspaces. ` +
      "This workspace is using an external Maven or Gradle model.",
    );
    return;
  }
  return executeJmanTask(command, operation, args);
}

function populateTestController(controller, facts) {
  controller.items.replace([]);
  const items = new Map();
  for (const fact of facts || []) {
    const uri = fact.uri ? vscode.Uri.parse(fact.uri) : undefined;
    const item = controller.createTestItem(fact.id, fact.label, uri);
    item.jmanSelector = fact.selector;
    if (fact.range) item.range = new vscode.Range(fact.range.start, fact.range.end);
    items.set(fact.id, item);
    if (fact.parentId) items.get(fact.parentId)?.children.add(item);
    else controller.items.add(item);
  }
}

async function refreshTestController(languageClient, controller) {
  const discovered = await languageClient.sendRequest("jman.java/tests/discover", {});
  const items = discovered?.items || [];
  populateTestController(controller, items);
  return items.length;
}

function selectTestItems(roots, include, exclude) {
  const excluded = new Set((exclude || []).map((item) => item.id));
  const selected = [];
  const hasExcludedDescendant = (item) => {
    let found = false;
    item.children.forEach((child) => {
      if (excluded.has(child.id) || hasExcludedDescendant(child)) found = true;
    });
    return found;
  };
  const collect = (item) => {
    if (excluded.has(item.id)) return;
    if (item.children.size === 0 || !hasExcludedDescendant(item)) {
      selected.push(item);
      return;
    }
    item.children.forEach(collect);
  };
  if (include?.length) include.forEach(collect);
  else roots.forEach(collect);
  return selected;
}

function leafTestItems(items) {
  const leaves = [];
  const collect = (item) => {
    if (item.children.size === 0) leaves.push(item);
    else item.children.forEach(collect);
  };
  items.forEach(collect);
  return leaves;
}

function testItemsBySelector(roots) {
  const items = new Map();
  const collect = (item) => {
    if (item.jmanSelector) items.set(item.jmanSelector, item);
    item.children.forEach(collect);
  };
  roots.forEach(collect);
  return items;
}

function parseTestEventLine(line) {
  try {
    const event = JSON.parse(line);
    return event.reason === "test-case" && event.test?.selector ? event : undefined;
  } catch {
    return undefined;
  }
}

async function testEventsFromXml(xml) {
  const document = await parseStringPromise(xml, { explicitArray: true });
  const suites = document.testsuites?.testsuite || [document.testsuite].filter(Boolean);
  return suites.flatMap((suite) => (suite.testcase || []).map((testcase) => {
    const attributes = testcase.$ || {};
    const methodName = (attributes.name || "").split("(")[0];
    const failure = testcase.failure?.[0];
    const error = testcase.error?.[0];
    const skipped = testcase.skipped?.[0];
    const problem = failure || error;
    return {
      reason: "test-case",
      test: {
        selector: `${attributes.classname}#${methodName}`,
        status: failure ? "failed" : error ? "errored" : skipped ? "skipped" : "passed",
        durationMillis: Math.round(Number.parseFloat(attributes.time || "0") * 1000),
        message: problem?.$?.message,
        details: typeof problem === "string" ? problem : problem?._,
      },
    };
  }));
}

async function externalTestEvents(buildSystem, startedAt) {
  const pattern = buildSystem === "gradle"
    ? "**/build/test-results/test/TEST-*.xml"
    : "**/target/surefire-reports/TEST-*.xml";
  const files = await vscode.workspace.findFiles(pattern, "**/{build,target}/**/tmp/**");
  const events = [];
  for (const file of files) {
    const stat = await vscode.workspace.fs.stat(file);
    if (stat.mtime + 1000 < startedAt) continue;
    const contents = await vscode.workspace.fs.readFile(file);
    events.push(...await testEventsFromXml(Buffer.from(contents).toString("utf8")));
  }
  return events;
}

function executeJmanProcess(command, args, environment, outputChannel, token) {
  return new Promise((resolve, reject) => {
    const folder = workspaceRoot();
    if (!folder) {
      reject(new Error("Open a workspace before running JMAN."));
      return;
    }
    const process = childProcess.spawn(command, args, {
      cwd: folder.uri.fsPath,
      env: environment,
    });
    const append = (data) => outputChannel.appendLine(data.toString().trimEnd());
    process.stdout.on("data", append);
    process.stderr.on("data", append);
    token?.onCancellationRequested(() => process.kill());
    process.once("error", reject);
    process.once("close", (code, signal) => {
      if (code === 0) resolve();
      else reject(new Error(
        signal ? `JMAN was terminated by ${signal}` : `JMAN exited with status ${code}`,
      ));
    });
  });
}

async function configureTesting(context, command, environment, outputChannel) {
  if (!vscode.tests?.createTestController) return;
  const controller = vscode.tests.createTestController(
    "io.github.zonnedev.jman.tests",
    "JMAN Java Tests",
  );
  context.subscriptions.push(controller);

  let retryTimer;
  const refresh = async (attempt = 0) => {
    const count = await refreshTestController(client, controller);
    if (count > 0) {
      if (retryTimer) clearTimeout(retryTimer);
      retryTimer = undefined;
    } else if (attempt < 20) {
      if (retryTimer) clearTimeout(retryTimer);
      retryTimer = setTimeout(() => refresh(attempt + 1), 250);
    }
  };
  controller.refreshHandler = refresh;
  await refresh();

  const runRequest = async (request, token) => {
    const run = controller.createTestRun(request);
    let executionTests = [];
    try {
      const tests = selectTestItems(
        controller.items,
        request.include,
        request.exclude,
      );
      executionTests = leafTestItems(tests);
      executionTests.forEach((item) => {
        run.enqueued(item);
        run.started(item);
      });
      if (tests.length === 0) {
        run.end();
        return;
      }
      const prepared = await client.sendRequest("jman.java/tests/run", {
        selectors: tests.map((item) => item.jmanSelector),
      });
      const folder = workspaceRoot();
      const args = prepared.arguments;
      const executable = executionCommand(
        prepared.program || "jman",
        command,
        folder.uri.fsPath,
      );
      const startedAt = Date.now();
      const process = childProcess.spawn(executable, args, {
        cwd: folder.uri.fsPath,
        env: environment,
      });
      const itemsBySelector = testItemsBySelector(controller.items);
      const reported = new Set();
      let stdoutBuffer = "";
      const report = (event) => {
        const item = itemsBySelector.get(event.test.selector);
        if (!item) return;
        reported.add(item.id);
        const duration = event.test.durationMillis;
        if (event.test.status === "passed") run.passed(item, duration);
        else if (event.test.status === "skipped") run.skipped(item);
        else {
          const text = [event.test.message, event.test.details]
            .filter(Boolean)
            .join("\n\n");
          const message = new vscode.TestMessage(text || "Test failed");
          if (event.test.status === "errored") run.errored(item, message, duration);
          else run.failed(item, message, duration);
        }
      };
      const append = (text) => {
        outputChannel.appendLine(text.trimEnd());
        run.appendOutput(text.replace(/\n/g, "\r\n"));
      };
      const consumeStdout = (data) => {
        stdoutBuffer += data.toString();
        const lines = stdoutBuffer.split(/\r?\n/);
        stdoutBuffer = lines.pop() || "";
        lines.forEach((line) => {
          const event = parseTestEventLine(line);
          if (event) report(event);
          else append(`${line}\n`);
        });
      };
      process.stdout.on("data", consumeStdout);
      process.stderr.on("data", (data) => append(data.toString()));
      token?.onCancellationRequested(() => process.kill());
      let ended = false;
      process.on("error", (error) => {
        if (ended) return;
        ended = true;
        executionTests.forEach((item) =>
          run.errored(item, new vscode.TestMessage(String(error))));
        outputChannel.error("JMAN test process failed", error);
        run.end();
      });
      process.on("close", async (code) => {
        if (ended) return;
        ended = true;
        if (prepared.report === "junit-xml") {
          try {
            const events = await externalTestEvents(prepared.buildSystem, startedAt);
            events.forEach(report);
          } catch (error) {
            outputChannel.error("Unable to read external test reports", error);
          }
        }
        const trailing = parseTestEventLine(stdoutBuffer);
        if (trailing) report(trailing);
        else if (stdoutBuffer) append(stdoutBuffer);
        executionTests.forEach((item) => {
          if (reported.has(item.id)) return;
          if (code === 0) run.passed(item);
          else run.errored(
            item,
            new vscode.TestMessage(`JMAN test exited with status ${code} without reporting this test`),
          );
        });
        run.end();
      });
    } catch (error) {
      const message = new vscode.TestMessage(String(error));
      executionTests.forEach((item) => run.errored(item, message));
      outputChannel.error("Unable to run JMAN tests", error);
      vscode.window.showErrorMessage(`Unable to run JMAN tests: ${error}`);
      run.end();
    }
  };
  const profile = controller.createRunProfile(
    "Run with JMAN",
    vscode.TestRunProfileKind.Run,
    runRequest,
    true,
  );
  context.subscriptions.push(profile);
  const watcher = vscode.workspace.createFileSystemWatcher(
    "**/src/test/java/**/*.java",
  );
  watcher.onDidCreate(() => refresh());
  watcher.onDidChange(() => refresh());
  watcher.onDidDelete(() => refresh());
  context.subscriptions.push(watcher, {
    dispose() {
      if (retryTimer) clearTimeout(retryTimer);
    },
  });
  const runSelector = async (selector) => {
    let item = testItemsBySelector(controller.items).get(selector);
    if (!item) {
      await refresh();
      item = testItemsBySelector(controller.items).get(selector);
    }
    if (!item) {
      vscode.window.showErrorMessage(`JMAN test was not discovered: ${selector}`);
      return;
    }
    return runRequest({ include: [item], exclude: [] });
  };
  const runAll = () => runRequest({ include: undefined, exclude: [] });
  return { refresh, runSelector, runAll };
}

async function activate(context) {
  const configuration = vscode.workspace.getConfiguration("jman.java");
  const configuredPath = configuration.get("server.path", "");
  const command =
    configuredPath || context.asAbsolutePath(path.join("server", "jman"));
  const javaHome = configuration.get("javaHome", "") || process.env.JAVA_HOME;
  const environment = {
    ...process.env,
    ...configuration.get("server.extraEnv", {}),
  };
  if (javaHome) {
    environment.JAVA_HOME = javaHome;
  }

  const buildSystem = configuration.get("buildSystem", "auto");
  const buildSync = configuration.get("buildSync", "prompt");
  const initializationOptions = { buildSync };
  if (buildSystem !== "auto") {
    initializationOptions.buildSystem = buildSystem;
  }
  const watchers = [
    "**/jman.toml",
    "**/jman.lock",
    "**/pom.xml",
    "**/.mvn/**/*",
    "**/build.gradle",
    "**/build.gradle.kts",
    "**/*.gradle",
    "**/*.gradle.kts",
    "**/settings.gradle",
    "**/settings.gradle.kts",
    "**/gradle.properties",
    "**/gradle-wrapper.properties",
    "**/gradle/*.versions.toml",
    "**/buildSrc/**/*",
  ].map((pattern) => vscode.workspace.createFileSystemWatcher(pattern));
  watchers.forEach((watcher) => context.subscriptions.push(watcher));

  // vscode-languageclient 10 uses the LogOutputChannel methods (trace, debug,
  // info, warn, and error) while reporting startup failures.
  const outputChannel =
    vscode.window.createOutputChannel("JMAN Java", { log: true });
  context.subscriptions.push(outputChannel);
  const status = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Left,
    10,
  );
  status.name = "JMAN Java";
  status.text = "$(sync~spin) JMAN Java: importing";
  status.command = "jmanJava.showStatus";
  status.show();
  context.subscriptions.push(status);
  const updateBuildSyncStatus = (sync) => {
    switch (sync?.state) {
      case "required":
        status.text = "$(warning) JMAN Java: sync required";
        status.tooltip = `${sync.pendingChanges || 0} build file change(s) require synchronization`;
        status.command = "jmanJava.syncWorkspace";
        break;
      case "syncing":
        status.text = "$(sync~spin) JMAN Java: syncing";
        status.tooltip = "Refreshing the Maven or Gradle project model";
        status.command = "jmanJava.showStatus";
        break;
      case "failed":
        status.text = "$(error) JMAN Java: sync failed";
        status.tooltip = sync.error || "The last build synchronization failed";
        status.command = "jmanJava.syncWorkspace";
        break;
      default:
        status.text = "$(check) JMAN Java";
        status.tooltip = "JMAN Java is ready";
        status.command = "jmanJava.showStatus";
    }
  };
  let refreshTests;
  let runTestSelector;
  let runAllTests;
  client = new LanguageClient(
    "io.github.zonnedev.jman.lsp",
    "JMAN Java",
    {
      command,
      args: ["lsp"],
      transport: TransportKind.stdio,
      options: {
        cwd: vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
        env: environment,
      },
    },
    {
      documentSelector: [{ scheme: "file", language: "java" }],
      initializationOptions,
      synchronize: { fileEvents: watchers, configurationSection: "jman.java" },
      outputChannel,
    },
  );
  client.onNotification("jman.java/buildSyncStatus", async (sync) => {
    updateBuildSyncStatus(sync);
    if (sync?.state === "ready" && refreshTests) await refreshTests();
    if (sync?.state === "required" && buildSync === "prompt") {
      const selection = await vscode.window.showInformationMessage(
        "Java build files changed. Synchronize the Maven/Gradle project model?",
        "Sync Now",
      );
      if (selection === "Sync Now") {
        await vscode.commands.executeCommand("jmanJava.syncWorkspace");
      }
    } else if (sync?.state === "failed") {
      outputChannel.error("Java project synchronization failed", sync.error);
    }
  });
  const executeWorkspaceOperation = async (operation) => {
    const prepared = await prepareProjectOperation(operation, command);
    if (!prepared) {
      vscode.window.showWarningMessage(
        `The current project model does not support ${operation}.`,
      );
      return;
    }
    return executeJmanProcess(
      prepared.command,
      prepared.args,
      environment,
      outputChannel,
    );
  };
  context.subscriptions.push(client);
  context.subscriptions.push(
    vscode.commands.registerCommand("jmanJava.showStatus", async () => {
      if (!client) {
        vscode.window.showWarningMessage("JMAN Java is not running.");
        return;
      }
      const result = await client.sendRequest("workspace/executeCommand", {
        command: "jman.java.status",
        arguments: [],
      });
      updateBuildSyncStatus(result.buildSync);
      const cache = result.cache || {
        bytes: 0,
        structural: { hits: 0, entries: 0 },
        semantic: { hits: 0, entries: 0 },
      };
      const message =
        `JMAN Java ready — ${result.indexedDocuments} indexed, ` +
        `${result.semanticDocuments} semantic, ${result.openDocuments} open, ` +
        `revision ${result.structuralRevision}; cache ` +
        `${cache.structural.hits}/${cache.structural.entries} structural hits, ` +
        `${cache.semantic.hits}/${cache.semantic.entries} semantic hits, ` +
        `${Math.round(cache.bytes / 1024)} KiB`;
      outputChannel.info(message, result);
      vscode.window.showInformationMessage(message);
    }),
    vscode.commands.registerCommand("jmanJava.restart", async () => {
      status.text = "$(sync~spin) JMAN Java: restarting";
      await client.restart();
      status.text = "$(check) JMAN Java";
      status.tooltip = "JMAN Java is ready";
    }),
    vscode.commands.registerCommand("jmanJava.check", () =>
      executeWorkspaceOperation("check")),
    vscode.commands.registerCommand("jmanJava.build", () =>
      executeWorkspaceOperation("build")),
    vscode.commands.registerCommand("jmanJava.run", () =>
      executeWorkspaceOperation("run")),
    vscode.commands.registerCommand("jmanJava.test", (selector) => {
      if (selector && runTestSelector) return runTestSelector(selector);
      if (!selector && runAllTests) return runAllTests();
      return executeWorkspaceOperation("test");
    }),
    vscode.commands.registerCommand("jman.java.test", (selector) => {
      if (selector && runTestSelector) return runTestSelector(selector);
      return executeNativeJmanTask(command, "test", selector ? ["--tests", selector] : []);
    }),
    vscode.commands.registerCommand("jmanJava.syncWorkspace", async () => {
      if (!client) return;
      updateBuildSyncStatus({ state: "syncing" });
      try {
        const result = await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Synchronizing Java project",
            cancellable: true,
          },
          async (_progress, token) => {
            const statusResult = await client.sendRequest("workspace/executeCommand", {
              command: "jman.java.status",
              arguments: [],
            });
            if (statusResult?.workspace?.nativeOperations) {
              const folder = workspaceRoot();
              await executeJmanProcess(
                command,
                ["--no-progress", "sync", folder.uri.fsPath],
                environment,
                outputChannel,
                token,
              );
            }
            return client.sendRequest("workspace/executeCommand", {
              command: "jman.java.syncWorkspace",
              arguments: [],
            });
          },
        );
        updateBuildSyncStatus(result?.buildSync);
        if (result?.buildSync?.state === "failed") {
          vscode.window.showErrorMessage(
            `Java project synchronization failed: ${result.buildSync.error}`,
          );
        } else {
          vscode.window.showInformationMessage("Java project synchronized.");
        }
      } catch (error) {
        updateBuildSyncStatus({ state: "failed", error: String(error) });
        throw error;
      }
    }),
    vscode.commands.registerCommand("jmanJava.changeSignature", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!client || !editor || editor.document.languageId !== "java") {
        vscode.window.showWarningMessage("Open a Java method before changing its signature.");
        return;
      }
      const parameters = await vscode.window.showInputBox({
        title: "Change Java method signature",
        prompt: "New parameter declarations, separated by commas",
        placeHolder: "String name, int count",
      });
      if (parameters === undefined) return;
      const orderValue = await vscode.window.showInputBox({
        title: "Map existing call arguments",
        prompt: "Zero-based old argument indexes in the new order",
        placeHolder: "1, 0",
      });
      if (orderValue === undefined) return;
      const newParameters = parameters.trim()
        ? parameters.split(",").map((value) => value.trim())
        : [];
      const argumentOrder = orderValue.trim()
        ? orderValue.split(",").map((value) => Number.parseInt(value.trim(), 10))
        : [];
      if (argumentOrder.some((value) => !Number.isInteger(value) || value < 0)) {
        vscode.window.showErrorMessage("Argument order must contain non-negative indexes.");
        return;
      }
      const result = await client.sendRequest("jman.java/changeSignature", {
        textDocument: { uri: editor.document.uri.toString() },
        position: editor.selection.active,
        newParameters,
        argumentOrder,
      });
      const edit = new vscode.WorkspaceEdit();
      for (const [uri, edits] of Object.entries(result.changes || {})) {
        const target = vscode.Uri.parse(uri);
        for (const change of edits) {
          edit.replace(
            target,
            new vscode.Range(
              change.range.start.line,
              change.range.start.character,
              change.range.end.line,
              change.range.end.character,
            ),
            change.newText,
          );
        }
      }
      await vscode.workspace.applyEdit(edit);
    }),
    vscode.commands.registerCommand("jmanJava.rebuildIndex", async () => {
      await vscode.window.withProgress(
        {
          location: vscode.ProgressLocation.Notification,
          title: "Rebuilding JMAN Java workspace index",
          cancellable: false,
        },
        () => client.sendRequest("workspace/executeCommand", {
          command: "jman.java.rebuildIndex",
          arguments: [],
        }),
      );
      vscode.window.showInformationMessage("JMAN Java workspace index rebuilt.");
    }),
    vscode.commands.registerCommand("jmanJava.clearWorkspaceCache", async () => {
      const confirmation = await vscode.window.showWarningMessage(
        "Clear and rebuild the JMAN Java cache for this workspace?",
        { modal: true },
        "Clear and Rebuild",
      );
      if (confirmation !== "Clear and Rebuild") return;
      await vscode.window.withProgress(
        {
          location: vscode.ProgressLocation.Notification,
          title: "Clearing JMAN Java workspace cache",
          cancellable: false,
        },
        () => client.sendRequest("workspace/executeCommand", {
          command: "jman.java.clearWorkspaceCache",
          arguments: [],
        }),
      );
      vscode.window.showInformationMessage("JMAN Java workspace cache cleared and rebuilt.");
    }),
  );
  try {
    await client.start();
    const workspaceStatus = await client.sendRequest("workspace/executeCommand", {
      command: "jman.java.status",
      arguments: [],
    });
    if (["jman", "gradle", "maven"].includes(workspaceStatus?.workspace?.buildSystem)) {
      const testing = await configureTesting(
        context,
        command,
        environment,
        outputChannel,
      );
      refreshTests = testing?.refresh;
      runTestSelector = testing?.runSelector;
      runAllTests = testing?.runAll;
    }
    status.text = "$(check) JMAN Java";
    status.tooltip = "JMAN Java is ready";
  } catch (error) {
    status.text = "$(error) JMAN Java";
    status.tooltip = String(error);
    outputChannel.show(true);
    throw error;
  }
}

async function deactivate() {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

module.exports = {
  activate,
  deactivate,
  executionCommand,
  populateTestController,
  leafTestItems,
  parseTestEventLine,
  operationArguments,
  testEventsFromXml,
  refreshTestController,
  selectTestItems,
  testItemsBySelector,
};
