vim.opt.runtimepath:prepend(vim.fn.getcwd() .. "/editors/neovim")

local jman = require("jman")
local temporary = vim.fn.tempname()
vim.fn.mkdir(temporary .. "/gradle/src/main/java/demo", "p")
vim.fn.mkdir(temporary .. "/gradle/module/src/main/java/demo", "p")
vim.fn.mkdir(temporary .. "/maven/src/main/java/demo", "p")
vim.fn.writefile({ 'rootProject.name = "demo"' }, temporary .. "/gradle/settings.gradle.kts")
vim.fn.writefile({ "#!/bin/sh" }, temporary .. "/gradle/gradlew")
vim.fn.writefile({ 'plugins { id("java") }' }, temporary .. "/gradle/module/build.gradle.kts")
vim.fn.writefile({ "<project/>" }, temporary .. "/maven/pom.xml")
vim.fn.writefile({ "#!/bin/sh" }, temporary .. "/maven/mvnw")

local gradle_file = temporary .. "/gradle/src/main/java/demo/App.java"
local gradle_module_file = temporary .. "/gradle/module/src/main/java/demo/App.java"
local maven_file = temporary .. "/maven/src/main/java/demo/App.java"
local root, system = jman._test.root_and_system(gradle_file)
assert(root == temporary .. "/gradle", "Gradle root was not detected")
assert(system == "gradle", "Gradle build system was not detected")
root, system = jman._test.root_and_system(gradle_module_file)
assert(root == temporary .. "/gradle", "Gradle settings root must win over a nested project build file")
assert(system == "gradle", "Nested Gradle module must retain the Gradle build system")
assert(
	jman._test.wrapper_command(system, root) == temporary .. "/gradle/gradlew",
	"Nested Gradle modules must use the root project wrapper"
)
root, system = jman._test.root_and_system(maven_file)
assert(root == temporary .. "/maven", "Maven root was not detected")
assert(system == "maven", "Maven build system was not detected")

assert(
	jman._test.wrapper_command("gradle", temporary .. "/gradle") == temporary .. "/gradle/gradlew",
	"Gradle wrapper must be preferred"
)
assert(
	jman._test.wrapper_command("maven", temporary .. "/maven") == temporary .. "/maven/mvnw",
	"Maven wrapper must be preferred"
)
assert(jman._test.operation_arguments.maven.build[1] == "package", "Maven builds must use package")
assert(jman._test.operation_arguments.maven.run == nil, "Unsupported Maven run must stay disabled")

local environment = jman._test.build_tool_environment("/jdks/21")
assert(environment.JAVA_HOME == "/jdks/21", "Prepared build Java home was not applied")
local path = environment.PATH or environment.Path
assert(path:sub(1, 12) == "/jdks/21/bin", "Build Java bin was not prepended to PATH")

local arguments = jman._test.human_test_arguments({
	report = "json-lines",
	arguments = { "--no-progress", "test", "--report", "json", "--tests", "demo.AppTest#works" },
})
assert(
	table.concat(arguments, " ") == "--no-progress test --tests demo.AppTest#works",
	"Neovim test terminals must receive human-readable JMAN output"
)
local external_arguments = { "test", "--tests", "demo.AppTest.works" }
local preserved = jman._test.human_test_arguments({ report = "junit-xml", arguments = external_arguments })
assert(vim.deep_equal(preserved, external_arguments), "External test arguments must remain unchanged")

local configured, configuration_error = pcall(jman.setup, { build_sync = "sometimes" })
assert(not configured, "Invalid build synchronization modes must be rejected")
assert(configuration_error:match("build_sync"), "Configuration errors must name the invalid option")

vim.fn.delete(temporary, "rf")
vim.cmd("qa!")
