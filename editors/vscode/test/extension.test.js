"use strict";

const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const Module = require("node:module");
const path = require("node:path");

async function main() {
  const subscriptions = [];
  const logChannel = {
    trace() {},
    debug() {},
    info() {},
    warn() {},
    error() {},
    appendLine() {},
    show() {},
    dispose() {},
  };
  let outputChannelOptions;
  let clientOptions;
  let serverOptions;
  let started = false;
  let status;
  const commands = new Map();
  const notifications = new Map();
  const requests = [];
  let shownMessage;
  const watcherPatterns = [];
  let runProfileHandler;
  let testingController;
  const testStates = new Map();
  const testRunSelectors = [];
  const collection = () => {
    const values = new Map();
    return {
      add(item) { values.set(item.id, item); },
      replace(items) {
        values.clear();
        items.forEach((item) => values.set(item.id, item));
      },
      forEach(callback) { values.forEach(callback); },
      get size() { return values.size; },
    };
  };

  class FakeLanguageClient {
    constructor(_id, _name, optionsForServer, options) {
      serverOptions = optionsForServer;
      clientOptions = options;
    }

    async start() {
      started = true;
    }

    async stop() {}

    async restart() {}

    onNotification(method, callback) {
      notifications.set(method, callback);
      return { dispose() {} };
    }

    async sendRequest(method, params) {
      if (method === "jman.java/tests/discover") {
        return { items: [
          { id: "class:run", label: "GreetingTest", selector: "dev.GreetingTest" },
          {
            id: "test:run", parentId: "class:run", label: "greets",
            selector: "dev.GreetingTest#greets",
          },
        ] };
      }
      if (method === "jman.java/tests/run") {
        testRunSelectors.push(params.selectors);
        return { arguments: ["--no-progress", "test", "--report", "json"] };
      }
      assert.equal(method, "workspace/executeCommand");
      requests.push(params.command);
      if (params.command === "jman.java.status") {
        return {
          protocolVersion: 1,
          workspace: {
            buildSystem: "jman",
            nativeOperations: true,
            buildRuntime: {
              buildToolVersion: "8.7",
              javaMajor: 21,
            },
          },
          indexedDocuments: 12,
          semanticDocuments: 10,
          openDocuments: 2,
          structuralRevision: 3,
          buildSync: { state: "ready", pendingChanges: 0, error: null },
          cache: {
            structural: { entries: 12, hits: 11, misses: 1 },
            semantic: { entries: 10, hits: 9, misses: 1 },
            bytes: 4096,
          },
        };
      }
      if (params.command === "jman.java.syncWorkspace") {
        return { buildSync: { state: "ready", pendingChanges: 0, error: null } };
      }
      return null;
    }

    dispose() {}
  }

  const vscode = {
    Uri: { parse(value) { return { value, toString() { return value; } }; } },
    Range: class Range {
      constructor(start, end) { this.start = start; this.end = end; }
    },
    TestMessage: class TestMessage {
      constructor(message) { this.message = message; }
    },
    TestRunProfileKind: { Run: 1 },
    tests: {
      createTestController() {
        testingController = {
          items: collection(),
          createTestItem(id, label, uri) {
            return { id, label, uri, children: collection() };
          },
          createRunProfile(_name, _kind, handler) {
            runProfileHandler = handler;
            return { dispose() {} };
          },
          createTestRun() {
            return {
              enqueued(item) { testStates.set(item.id, "enqueued"); },
              started(item) { testStates.set(item.id, "started"); },
              passed(item) { testStates.set(item.id, "passed"); },
              failed(item, message) { testStates.set(item.id, `failed:${message.message}`); },
              skipped(item) { testStates.set(item.id, "skipped"); },
              errored(item, message) { testStates.set(item.id, `errored:${message.message}`); },
              appendOutput() {},
              end() { testStates.set("run", "ended"); },
            };
          },
          dispose() {},
        };
        return testingController;
      },
    },
    workspace: {
      workspaceFolders: [{ uri: { fsPath: "/workspace" } }],
      getConfiguration() {
        return {
          get(key, fallback) {
            const values = {
              "server.path": "/extension/server/jman",
              javaHome: "/graalvm",
              buildJavaHome: "/build-jdk",
              buildSystem: "auto",
              buildSync: "prompt",
              "server.extraEnv": {
                JAVA_LSP_BUILD_JAVA_HOME: "/legacy-build-jdk",
              },
            };
            return values[key] ?? fallback;
          },
        };
      },
      createFileSystemWatcher(pattern) {
        watcherPatterns.push(pattern);
        return {
          onDidCreate() {},
          onDidChange() {},
          onDidDelete() {},
          dispose() {},
        };
      },
    },
    window: {
      createOutputChannel(_name, options) {
        outputChannelOptions = options;
        return logChannel;
      },
      createStatusBarItem() {
        status = {
          text: "",
          tooltip: "",
          visible: false,
          show() {
            this.visible = true;
          },
          dispose() {},
        };
        return status;
      },
      showInformationMessage(message, action) {
        shownMessage = message;
        if (action === "Sync Now") return "Sync Now";
      },
      showWarningMessage() {
        return "Clear and Rebuild";
      },
      showErrorMessage(message) {
        shownMessage = message;
      },
      withProgress(_options, task) {
        return task();
      },
    },
    commands: {
      registerCommand(name, callback) {
        commands.set(name, callback);
        return { dispose() {} };
      },
      executeCommand(name) {
        return commands.get(name)?.();
      },
    },
    StatusBarAlignment: { Left: 1 },
    ProgressLocation: { Notification: 15 },
  };

  const originalLoad = Module._load;
  Module._load = function load(request, parent, isMain) {
    if (request === "vscode") {
      return vscode;
    }
    if (request === "vscode-languageclient/node") {
      return { LanguageClient: FakeLanguageClient, TransportKind: { stdio: 0 } };
    }
    if (request === "child_process") {
      return {
        spawn(_command, args) {
          const process = new EventEmitter();
          process.stdout = new EventEmitter();
          process.stderr = new EventEmitter();
          process.kill = () => {};
          queueMicrotask(() => {
            if (args.includes("test")) {
              process.stdout.emit("data", Buffer.from(`${JSON.stringify({
                reason: "test-case",
                test: {
                  selector: "dev.GreetingTest#greets",
                  status: "failed",
                  durationMillis: 12,
                  message: "expected Ada but was Bob",
                },
              })}\n`));
              process.emit("close", 1);
            } else {
              process.emit("close", 0);
            }
          });
          return process;
        },
      };
    }
    return originalLoad.call(this, request, parent, isMain);
  };

  let extension;
  try {
    extension = require(path.join(__dirname, "..", "extension.js"));
    await extension.activate({
      subscriptions,
      asAbsolutePath(value) {
        return path.join("/extension", value);
      },
    });
  } finally {
    Module._load = originalLoad;
  }

  assert.deepEqual(outputChannelOptions, { log: true });
  assert.equal(serverOptions.command, "/extension/server/jman");
  assert.deepEqual(serverOptions.args, ["lsp"]);
  assert.equal(serverOptions.options.env.JAVA_HOME, "/graalvm");
  assert.equal(
    serverOptions.options.env.JAVA_LSP_BUILD_JAVA_HOME,
    "/build-jdk",
  );
  assert(watcherPatterns.includes("**/jman.toml"));
  assert(watcherPatterns.includes("**/jman.lock"));
  assert.equal(clientOptions.outputChannel, logChannel);
  const roots = [];
  const controller = {
    items: {
      replace() { roots.length = 0; },
      add(item) { roots.push(item); },
    },
    createTestItem(id, label, uri) {
      const children = [];
      return {
        id,
        label,
        uri,
        children: {
          add(item) { children.push(item); },
          forEach(callback) { children.forEach(callback); },
          get size() { return children.length; },
          values: children,
        },
      };
    },
  };
  extension.populateTestController(controller, [
    { id: "class:1", label: "GreetingTest", uri: "file:///GreetingTest.java", selector: "dev.GreetingTest" },
    {
      id: "test:1", parentId: "class:1", label: "greets", kind: "test",
      uri: "file:///GreetingTest.java", selector: "dev.GreetingTest#greets",
      range: { start: { line: 2, character: 7 }, end: { line: 2, character: 13 } },
    },
    {
      id: "test:2", parentId: "class:1", label: "welcomes", kind: "test",
      uri: "file:///GreetingTest.java", selector: "dev.GreetingTest#welcomes",
    },
  ]);
  assert.equal(roots.length, 1);
  assert.equal(roots[0].children.values[0].jmanSelector, "dev.GreetingTest#greets");
  assert.equal(roots[0].children.values[0].range.start.line, 2);
  const classItem = roots[0];
  const methodItem = classItem.children.values[0];
  const otherMethodItem = classItem.children.values[1];
  assert.deepEqual(
    extension.selectTestItems(roots, [classItem], []),
    [classItem],
    "running a class must use its class selector instead of expanding methods",
  );
  assert.deepEqual(
    extension.selectTestItems(roots, [methodItem], []),
    [methodItem],
    "running one method must preserve its method selector",
  );
  assert.deepEqual(
    extension.selectTestItems(roots, undefined, []),
    [classItem],
    "Run All must use one selector per test class",
  );
  assert.deepEqual(
    extension.selectTestItems(roots, [classItem], [methodItem]),
    [otherMethodItem],
    "a method exclusion must expand the class and retain other methods",
  );
  assert.deepEqual(
    extension.leafTestItems([classItem]),
    [methodItem, otherMethodItem],
    "class execution must report its individual methods to VS Code",
  );
  assert.equal(
    extension.testItemsBySelector(roots).get("dev.GreetingTest#greets"),
    methodItem,
  );
  assert.deepEqual(
    extension.parseTestEventLine(JSON.stringify({
      reason: "test-case",
      module: "app",
      test: {
        selector: "dev.GreetingTest#greets",
        status: "failed",
        durationMillis: 12,
        message: "expected: <Ada> but was: <Bob>",
      },
    })).test,
    {
      selector: "dev.GreetingTest#greets",
      status: "failed",
      durationMillis: 12,
      message: "expected: <Ada> but was: <Bob>",
    },
  );
  assert.equal(extension.parseTestEventLine("ordinary test output"), undefined);
  assert.deepEqual(extension.operationArguments("jman", "build"), ["build"]);
  assert.deepEqual(extension.operationArguments("gradle", "build"), ["build"]);
  assert.deepEqual(extension.operationArguments("maven", "build"), ["package"]);
  assert.deepEqual(extension.operationArguments("maven", "test"), ["test"]);
  assert.equal(
    extension.executionCommand("gradle", "/server/jman", "/missing"),
    "gradle",
  );
  assert.equal(extension.executionCommand("jman", "/server/jman", "/missing"), "/server/jman");
  const xmlEvents = await extension.testEventsFromXml(
    `<testsuite><testcase classname="dev.GreetingTest" name="greets()" time="0.012">` +
    `<failure message="expected Ada">stack trace</failure></testcase>` +
    `<testcase classname="dev.GreetingTest" name="welcomes()" time="0.003"/>` +
    `</testsuite>`,
  );
  assert.equal(xmlEvents[0].test.selector, "dev.GreetingTest#greets");
  assert.equal(xmlEvents[0].test.status, "failed");
  assert.equal(xmlEvents[0].test.durationMillis, 12);
  assert.equal(xmlEvents[1].test.status, "passed");
  let discoveryRequests = 0;
  const discoveredCount = await extension.refreshTestController(
    {
      async sendRequest(method, params) {
        discoveryRequests += 1;
        assert.equal(method, "jman.java/tests/discover");
        assert.deepEqual(params, {});
        return {
          items: [{
            id: "class:refresh",
            label: "RefreshedTest",
            uri: "file:///RefreshedTest.java",
            selector: "dev.RefreshedTest",
          }],
        };
      },
    },
    controller,
  );
  assert.equal(discoveryRequests, 1);
  assert.equal(discoveredCount, 1);
  assert.equal(roots[0].jmanSelector, "dev.RefreshedTest");
  assert.deepEqual(clientOptions.initializationOptions, { buildSync: "prompt" });
  assert.equal(started, true);
  assert(runProfileHandler, "native JMAN workspaces must register a test run profile");
  let classToRun;
  testingController.items.forEach((item) => { classToRun = item; });
  await runProfileHandler(
    { include: [classToRun], exclude: [] },
    { onCancellationRequested() { return { dispose() {} }; } },
  );
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(testStates.get("test:run"), "failed:expected Ada but was Bob");
  assert.equal(testStates.get("run"), "ended");
  assert.deepEqual(testRunSelectors[0], ["dev.GreetingTest"]);
  testStates.clear();
  await commands.get("jman.java.test")("dev.GreetingTest#greets");
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    testStates.get("test:run"),
    "failed:expected Ada but was Bob",
    "the LSP CodeLens command must update the same TestController item",
  );
  assert.equal(testStates.get("run"), "ended");
  assert.deepEqual(testRunSelectors[1], ["dev.GreetingTest#greets"]);
  assert.equal(status.visible, true);
  assert.equal(status.text, "$(check) JMAN Java");
  assert.equal(status.command, "jmanJava.showStatus");
  assert(commands.has("jmanJava.showStatus"));
  assert(commands.has("jmanJava.restart"));
  assert(commands.has("jmanJava.syncWorkspace"));
  assert(commands.has("jmanJava.check"));
  assert(commands.has("jmanJava.build"));
  assert(commands.has("jmanJava.run"));
  assert(commands.has("jmanJava.test"));
  assert(commands.has("jmanJava.rebuildIndex"));
  assert(commands.has("jmanJava.clearWorkspaceCache"));
  await commands.get("jmanJava.showStatus")();
  assert.match(shownMessage, /12 indexed, 10 semantic, 2 open, revision 3/);
  assert.match(shownMessage, /11\/12 structural hits/);
  assert.match(shownMessage, /9\/10 semantic hits/);
  assert.match(shownMessage, /4 KiB/);
  assert.match(shownMessage, /Gradle 8\.7 on Java 21/);
  await notifications.get("jman.java/buildSyncStatus")({
    state: "required",
    pendingChanges: 2,
  });
  assert.equal(status.text, "$(check) JMAN Java");
  assert.equal(status.command, "jmanJava.showStatus");
  assert.match(shownMessage, /project synchronized/);
  await commands.get("jmanJava.rebuildIndex")();
  assert.match(shownMessage, /index rebuilt/);
  await commands.get("jmanJava.clearWorkspaceCache")();
  assert.match(shownMessage, /cache cleared and rebuilt/);
  assert.deepEqual(requests, [
    "jman.java.status",
    "jman.java.status",
    "jman.java.status",
    "jman.java.syncWorkspace",
    "jman.java.rebuildIndex",
    "jman.java.clearWorkspaceCache",
  ]);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
