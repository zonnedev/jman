local M = {}

local defaults = {
	cmd = nil,
	java_home = nil,
	build_java_home = nil,
	build_system = "auto",
	build_sync = "prompt",
	extra_env = {},
	notify = true,
	keymaps = true,
}

local state = {
	options = vim.deepcopy(defaults),
	sync = { state = "starting" },
	terminal_id = 0,
}

local build_markers = {
	jman = { "jman.toml" },
	gradle = { "settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts" },
	maven = { "pom.xml" },
}

local gradle_root_markers = { "settings.gradle", "settings.gradle.kts", "gradlew", "gradlew.bat" }

local operation_arguments = {
	jman = {
		check = { "check" },
		build = { "build" },
		test = { "test" },
		run = { "run" },
	},
	gradle = {
		check = { "check" },
		build = { "build" },
		test = { "test" },
		run = { "run" },
	},
	maven = {
		check = { "test", "-DskipTests" },
		build = { "package" },
		test = { "test" },
	},
}

local function executable()
	if state.options.cmd and state.options.cmd ~= "" then
		return state.options.cmd
	end
	local configured = vim.env.JMAN_BIN
	if configured and configured ~= "" then
		return configured
	end
	local found = vim.fn.exepath("jman")
	return found ~= "" and found or "jman"
end

function M.executable()
	return executable()
end

function M.config()
	return vim.deepcopy(state.options)
end

local function parents(path)
	local current = vim.fs.dirname(vim.fs.normalize(path))
	return function()
		if not current then
			return nil
		end
		local result = current
		local parent = vim.fs.dirname(current)
		current = parent ~= current and parent or nil
		return result
	end
end

local function root_and_system(path)
	local candidates = {}
	local gradle_root
	for directory in parents(path) do
		if not gradle_root then
			for _, marker in ipairs(gradle_root_markers) do
				if vim.uv.fs_stat(directory .. "/" .. marker) then
					gradle_root = directory
					break
				end
			end
		end
		for system, markers in pairs(build_markers) do
			if not candidates[system] then
				for _, marker in ipairs(markers) do
					if vim.uv.fs_stat(directory .. "/" .. marker) then
						candidates[system] = directory
						break
					end
				end
			end
		end
	end
	candidates.gradle = gradle_root or candidates.gradle
	if state.options.build_system ~= "auto" then
		local configured = state.options.build_system
		return candidates[configured], candidates[configured] and configured or nil
	end
	if candidates.jman then
		return candidates.jman, "jman"
	end
	if candidates.gradle then
		return candidates.gradle, "gradle"
	end
	if candidates.maven then
		return candidates.maven, "maven"
	end
end

local function set_sync(sync)
	state.sync = sync or { state = "unknown" }
	vim.schedule(function()
		vim.api.nvim_exec_autocmds("User", {
			pattern = "JmanStatusChanged",
			modeline = false,
		})
	end)
end

local function client_for_buffer(bufnr)
	for _, client in ipairs(vim.lsp.get_clients({ bufnr = bufnr or 0, name = "jman-java" })) do
		return client
	end
end

local function notify(message, level)
	if state.options.notify then
		vim.notify(message, level or vim.log.levels.INFO, { title = "JMAN Java" })
	end
end

local function json_object(value)
	return type(value) == "table" and value or {}
end

local function json_value(value, fallback)
	if value == nil or value == vim.NIL then
		return fallback
	end
	return value
end

local function execute_command(command, callback)
	local client = client_for_buffer(0)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	client:request("workspace/executeCommand", {
		command = command,
		arguments = {},
	}, function(error, result)
		if error then
			notify(error.message or tostring(error), vim.log.levels.ERROR)
			return
		end
		if type(result) == "table" and type(result.buildSync) == "table" then
			set_sync(result.buildSync)
		end
		if callback then
			callback(result)
		end
	end, 0)
end

local function path_separator()
	return package.config:sub(1, 1) == "\\" and ";" or ":"
end

local function build_tool_environment(java_home, use_build_java_home)
	local environment = vim.tbl_extend("force", vim.fn.environ(), state.options.extra_env)
	if (not java_home or java_home == "") and use_build_java_home ~= false then
		java_home = state.options.build_java_home
	end
	if not java_home or java_home == "" then
		return environment
	end
	environment.JAVA_HOME = java_home
	local path_key = environment.PATH and "PATH" or environment.Path and "Path" or "PATH"
	local java_bin = java_home .. "/bin"
	environment[path_key] = environment[path_key] and (java_bin .. path_separator() .. environment[path_key])
		or java_bin
	return environment
end

local function operation_environment(system, java_home)
	if system == "jman" then
		return build_tool_environment(state.options.java_home, false)
	end
	return build_tool_environment(java_home, true)
end

local function wrapper_command(system, root)
	if system == "jman" then
		return executable()
	end
	local windows = package.config:sub(1, 1) == "\\"
	local wrapper = system == "gradle" and "gradlew" or "mvnw"
	if windows then
		wrapper = wrapper .. ".cmd"
	end
	local candidate = root .. "/" .. wrapper
	if vim.uv.fs_stat(candidate) then
		return candidate
	end
	return system == "maven" and "mvn" or "gradle"
end

local function apply_code_action(client, action, bufnr)
	if action.edit then
		vim.lsp.util.apply_workspace_edit(action.edit, client.offset_encoding)
	end
	local command = action.command
	if type(command) == "string" then
		command = { command = command, arguments = action.arguments }
	end
	if command then
		client:exec_cmd(command, { bufnr = bufnr })
	end
end

local function lsp_diagnostics(bufnr)
	return vim.tbl_filter(
		function(diagnostic)
			return diagnostic ~= nil
		end,
		vim.tbl_map(function(diagnostic)
			return diagnostic.user_data and diagnostic.user_data.lsp
		end, vim.diagnostic.get(bufnr))
	)
end

function M.code_actions(only, apply)
	local bufnr = vim.api.nvim_get_current_buf()
	local client = client_for_buffer(bufnr)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	local params = vim.lsp.util.make_range_params(0, client.offset_encoding)
	params.context = {
		only = only,
		diagnostics = lsp_diagnostics(bufnr),
	}
	client:request("textDocument/codeAction", params, function(error, actions)
		if error then
			notify(error.message or tostring(error), vim.log.levels.ERROR)
			return
		end
		actions = json_object(actions)
		if #actions == 0 then
			notify("No JMAN code actions available", vim.log.levels.INFO)
			return
		end
		vim.schedule(function()
			if apply and #actions == 1 then
				apply_code_action(client, actions[1], bufnr)
				return
			end
			vim.ui.select(actions, {
				prompt = "JMAN code actions",
				format_item = function(action)
					return action.title
				end,
			}, function(action)
				if action then
					apply_code_action(client, action, bufnr)
				end
			end)
		end)
	end, bufnr)
end

function M.organize_imports()
	M.code_actions({ "source.organizeImports" }, true)
end

local function terminal(command, args, title, root, environment)
	local invocation = { command }
	vim.list_extend(invocation, args)
	vim.cmd("botright new")
	local buffer = vim.api.nvim_get_current_buf()
	state.terminal_id = state.terminal_id + 1
	local safe_title = title:gsub("%c", " ")
	vim.api.nvim_buf_set_name(buffer, string.format("JMAN://%s/%d", safe_title, state.terminal_id))
	vim.bo[buffer].bufhidden = "wipe"
	vim.bo[buffer].swapfile = false
	vim.fn.termopen(invocation, {
		cwd = root,
		env = environment or build_tool_environment(),
		on_exit = function(_, code)
			vim.schedule(function()
				notify(
					code == 0 and (title .. " completed") or (title .. " failed with status " .. code),
					code == 0 and vim.log.levels.INFO or vim.log.levels.ERROR
				)
			end)
		end,
	})
	vim.cmd("startinsert")
end

local function active_workspace(bufnr)
	local file = vim.api.nvim_buf_get_name(bufnr or 0)
	local root, system = root_and_system(file)
	if not root then
		notify("No JMAN, Maven, or Gradle Java workspace was detected", vim.log.levels.ERROR)
		return
	end
	return root, system
end

local function workspace_operation(operation)
	local root, detected_system = active_workspace()
	if not root then
		return
	end
	local client = client_for_buffer(0)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	client:request("workspace/executeCommand", {
		command = "jman.java.status",
		arguments = {},
	}, function(error, result)
		if error then
			notify(error.message or tostring(error), vim.log.levels.ERROR)
			return
		end
		local workspace = json_object(json_object(result).workspace)
		local system = json_value(workspace.buildSystem, detected_system)
		local args = operation_arguments[system] and operation_arguments[system][operation]
		if not args then
			notify(
				string.format("The %s project model does not support %s", system or "current", operation),
				vim.log.levels.WARN
			)
			return
		end
		local runtime = json_object(workspace.buildRuntime)
		vim.schedule(function()
			terminal(
				wrapper_command(system, root),
				vim.deepcopy(args),
				operation:gsub("^%l", string.upper),
				root,
				operation_environment(system, runtime.javaHome)
			)
		end)
	end, 0)
end

local function test_selector(bufnr, cursor_line)
	local lines = vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
	local package_name = ""
	local class_name
	for _, line in ipairs(lines) do
		package_name = line:match("^%s*package%s+([%w_.]+)%s*;") or package_name
		local declared = line:match("%f[%a]class%s+([%w_$]+)")
			or line:match("%f[%a]record%s+([%w_$]+)")
			or line:match("%f[%a]enum%s+([%w_$]+)")
		class_name = declared or class_name
	end
	if not class_name then
		return nil, "No Java test class was found"
	end
	local method
	local annotated = false
	local ignored = { ["if"] = true, ["for"] = true, ["while"] = true, ["switch"] = true, ["catch"] = true }
	for index = math.min(cursor_line, #lines), 1, -1 do
		local name = lines[index]:match("([%a_$][%w_$]*)%s*%(")
		if name and not ignored[name] then
			method = name
			for annotation = math.max(1, index - 8), index - 1 do
				if lines[annotation]:match("@[%w_.$]*Test") then
					annotated = true
					break
				end
			end
			break
		end
	end
	local qualified = package_name ~= "" and (package_name .. "." .. class_name) or class_name
	if method and annotated then
		return qualified .. "#" .. method
	end
	return qualified
end

local function position_in_range(line, character, range)
	if not range or not range.start or not range["end"] then
		return false
	end
	local starts_before = line > range.start.line or (line == range.start.line and character >= range.start.character)
	local ends_after = line < range["end"].line or (line == range["end"].line and character <= range["end"].character)
	return starts_before and ends_after
end

local function nearest_test_item(items, line, character)
	local candidates = vim.tbl_filter(function(item)
		return item.selector and position_in_range(line, character, item.range)
	end, items or {})
	table.sort(candidates, function(left, right)
		if left.kind ~= right.kind then
			return left.kind == "test"
		end
		local left_lines = left.range["end"].line - left.range.start.line
		local right_lines = right.range["end"].line - right.range.start.line
		if left_lines ~= right_lines then
			return left_lines < right_lines
		end
		return left.selector < right.selector
	end)
	return candidates[1]
end

function M.status()
	local client = client_for_buffer(0)
	local sync = state.sync.state or "unknown"
	return client and ("JMAN:" .. sync) or "JMAN:off"
end

local function status_message(result)
	result = json_object(result)
	local workspace = json_object(result.workspace)
	local cache = json_object(result.cache)
	local structural = json_object(cache.structural)
	local semantic = json_object(cache.semantic)
	local runtime = json_object(workspace.buildRuntime)
	local runtime_summary = ""
	if type(workspace.buildRuntime) == "table" then
		runtime_summary = string.format(
			" · %s %s on Java %s",
			workspace.buildSystem == "gradle" and "Gradle" or workspace.buildSystem == "maven" and "Maven" or "build",
			json_value(runtime.buildToolVersion, "unknown"),
			json_value(runtime.javaMajor, "unknown")
		)
	end
	local build_sync = json_object(result.buildSync)
	return string.format(
		"%s · %d indexed · %d semantic · %d open · sync %s · cache %d/%d structural, %d/%d semantic, %.1f MiB%s",
		json_value(workspace.buildSystem, "unknown"),
		json_value(result.indexedDocuments, 0),
		json_value(result.semanticDocuments, 0),
		json_value(result.openDocuments, 0),
		json_value(build_sync.state, "unknown"),
		json_value(structural.hits, 0),
		json_value(structural.entries, 0),
		json_value(semantic.hits, 0),
		json_value(semantic.entries, 0),
		json_value(cache.bytes, 0) / 1024 / 1024,
		runtime_summary
	)
end

function M.show_status()
	execute_command("jman.java.status", function(result)
		notify(status_message(result))
	end)
end

function M.sync()
	local file = vim.api.nvim_buf_get_name(0)
	local root, system = root_and_system(file)
	if not root then
		notify("No Java workspace was detected", vim.log.levels.ERROR)
		return
	end
	set_sync({ state = "syncing" })
	if system ~= "jman" then
		execute_command("jman.java.syncWorkspace", function()
			notify("Java project model synchronized")
		end)
		return
	end
	vim.system({ executable(), "--no-progress", "sync", root }, {
		cwd = root,
		env = operation_environment("jman"),
		text = true,
	}, function(result)
		vim.schedule(function()
			if result.code ~= 0 then
				set_sync({ state = "failed", error = result.stderr })
				notify(result.stderr ~= "" and result.stderr or "jman sync failed", vim.log.levels.ERROR)
				return
			end
			execute_command("jman.java.syncWorkspace", function()
				notify("JMAN workspace synchronized")
			end)
		end)
	end)
end

function M.check()
	workspace_operation("check")
end
function M.build()
	workspace_operation("build")
end
function M.run()
	workspace_operation("run")
end
function M.test()
	workspace_operation("test")
end

local function human_test_arguments(prepared)
	local arguments = {}
	local skip_next = false
	for index, argument in ipairs(prepared.arguments or {}) do
		if skip_next then
			skip_next = false
		elseif
			prepared.report == "json-lines"
			and argument == "--report"
			and prepared.arguments[index + 1] == "json"
		then
			skip_next = true
		else
			table.insert(arguments, argument)
		end
	end
	return arguments
end

local function run_test_selectors(selectors, bufnr)
	bufnr = bufnr or vim.api.nvim_get_current_buf()
	if not vim.api.nvim_buf_is_valid(bufnr) then
		notify("The Java buffer is no longer available", vim.log.levels.WARN)
		return
	end
	local root = active_workspace(bufnr)
	if not root then
		return
	end
	local client = client_for_buffer(bufnr)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	local uri = vim.uri_from_bufnr(bufnr)
	client:request("jman.java/tests/run", {
		selectors = selectors,
		uri = uri ~= "" and uri or nil,
	}, function(error, prepared)
		if error then
			notify(error.message or tostring(error), vim.log.levels.ERROR)
			return
		end
		prepared = json_object(prepared)
		if not prepared.program then
			notify("The language server returned no test execution plan", vim.log.levels.ERROR)
			return
		end
		vim.schedule(function()
			terminal(
				wrapper_command(prepared.program, root),
				human_test_arguments(prepared),
				"Test " .. table.concat(selectors, ", "),
				root,
				operation_environment(prepared.program, prepared.buildJavaHome)
			)
		end)
	end, bufnr)
end

function M.test_pattern(pattern)
	pattern = pattern or vim.fn.input("JMAN test pattern: ")
	if pattern and pattern ~= "" then
		run_test_selectors({ pattern })
	end
end

function M.test_nearest()
	local bufnr = vim.api.nvim_get_current_buf()
	local client = client_for_buffer(bufnr)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	local cursor = vim.api.nvim_win_get_cursor(0)
	client:request("jman.java/tests/discover", {
		textDocument = vim.lsp.util.make_text_document_params(bufnr),
	}, function(error, result)
		if not vim.api.nvim_buf_is_valid(bufnr) then
			return
		end
		local item = not error and nearest_test_item(json_object(json_object(result).items), cursor[1] - 1, cursor[2])
		if item then
			run_test_selectors({ item.selector }, bufnr)
			return
		end
		local selector, fallback_error = test_selector(bufnr, cursor[1])
		if selector then
			run_test_selectors({ selector }, bufnr)
		else
			notify(
				error and (error.message or tostring(error)) or fallback_error or "No test was found at the cursor",
				vim.log.levels.ERROR
			)
		end
	end, bufnr)
end

function M.tests()
	local bufnr = vim.api.nvim_get_current_buf()
	local client = client_for_buffer(bufnr)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	client:request("jman.java/tests/discover", {}, function(error, result)
		if error then
			notify(error.message or tostring(error), vim.log.levels.ERROR)
			return
		end
		local discovered = json_object(json_object(result).items)
		local tests = vim.tbl_filter(function(item)
			return item.selector ~= nil and (item.kind == "class" or item.kind == "test")
		end, discovered)
		local items = {}
		for _, item in ipairs(discovered) do
			items[item.id] = item
		end
		vim.schedule(function()
			vim.ui.select(tests, {
				prompt = "JMAN tests",
				format_item = function(item)
					local parent = item.parentId and items[item.parentId]
					if parent then
						return string.format("%s › %s", parent.label, item.label)
					end
					return string.format("%s (all tests)", item.label)
				end,
			}, function(item)
				if item then
					run_test_selectors({ item.selector }, bufnr)
				end
			end)
		end)
	end, bufnr)
end

function M.rebuild_index()
	execute_command("jman.java.rebuildIndex", function()
		notify("JMAN Java index rebuilt")
	end)
end

function M.clear_cache()
	vim.ui.select({ "Clear and rebuild", "Cancel" }, {
		prompt = "Clear the JMAN Java cache for this workspace?",
	}, function(choice)
		if choice == "Clear and rebuild" then
			execute_command("jman.java.clearWorkspaceCache", function()
				notify("JMAN Java cache cleared and rebuilt")
			end)
		end
	end)
end

function M.restart()
	for _, client in ipairs(vim.lsp.get_clients({ name = "jman-java" })) do
		client:stop(true)
	end
	vim.schedule(function()
		vim.cmd("edit")
		notify("JMAN Java restarted")
	end)
end

function M.change_signature()
	local client = client_for_buffer(0)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	vim.ui.input({ prompt = "New parameters: " }, function(parameters)
		if parameters == nil then
			return
		end
		vim.ui.input({ prompt = "Old argument indexes in new order: " }, function(order)
			if order == nil then
				return
			end
			local new_parameters = {}
			for value in parameters:gmatch("[^,]+") do
				table.insert(new_parameters, vim.trim(value))
			end
			local argument_order = {}
			for value in order:gmatch("[^,]+") do
				local number = tonumber(vim.trim(value))
				if not number or number < 0 or number % 1 ~= 0 then
					notify("Argument order must contain non-negative integers", vim.log.levels.ERROR)
					return
				end
				table.insert(argument_order, number)
			end
			client:request("jman.java/changeSignature", {
				textDocument = vim.lsp.util.make_text_document_params(0),
				position = vim.lsp.util.make_position_params(0, client.offset_encoding).position,
				newParameters = new_parameters,
				argumentOrder = argument_order,
			}, function(error, edit)
				if error then
					notify(error.message or tostring(error), vim.log.levels.ERROR)
				elseif type(edit) == "table" then
					vim.lsp.util.apply_workspace_edit(edit, client.offset_encoding)
				end
			end, 0)
		end)
	end)
end

local function create_commands()
	local commands = {
		JmanStatus = M.show_status,
		JmanSync = M.sync,
		JmanCheck = M.check,
		JmanBuild = M.build,
		JmanRun = M.run,
		JmanTest = M.test,
		JmanTests = M.tests,
		JmanTestNearest = M.test_nearest,
		JmanRebuildIndex = M.rebuild_index,
		JmanClearCache = M.clear_cache,
		JmanRestartLsp = M.restart,
		JmanChangeSignature = M.change_signature,
		JmanCodeAction = function()
			M.code_actions(nil, false)
		end,
		JmanOrganizeImports = M.organize_imports,
	}
	for name, callback in pairs(commands) do
		vim.api.nvim_create_user_command(name, callback, { force = true })
	end
	vim.api.nvim_create_user_command("JmanTestPattern", function(command)
		M.test_pattern(command.args)
	end, { nargs = "?", force = true })
end

local function server_environment()
	local environment = vim.deepcopy(state.options.extra_env)
	if state.options.java_home and state.options.java_home ~= "" then
		environment.JAVA_HOME = state.options.java_home
	end
	if state.options.build_java_home and state.options.build_java_home ~= "" then
		environment.JAVA_LSP_BUILD_JAVA_HOME = state.options.build_java_home
	end
	return environment
end

local function configure_lsp()
	vim.lsp.config("jman-java", {
		cmd = { executable(), "lsp", "--stdio" },
		filetypes = { "java" },
		root_dir = function(bufnr, callback)
			local root = root_and_system(vim.api.nvim_buf_get_name(bufnr))
			callback(root)
		end,
		init_options = {
			buildSystem = state.options.build_system ~= "auto" and state.options.build_system or nil,
			buildSync = state.options.build_sync,
		},
		cmd_env = server_environment(),
		commands = {
			["jman.java.test"] = function(command)
				M.test_pattern(command.arguments and command.arguments[1])
			end,
		},
		on_attach = function(_, bufnr)
			local group = vim.api.nvim_create_augroup("jman_test_codelens", { clear = false })
			vim.api.nvim_clear_autocmds({ group = group, buffer = bufnr })
			vim.api.nvim_create_autocmd({ "BufEnter", "InsertLeave", "BufWritePost" }, {
				group = group,
				buffer = bufnr,
				callback = function()
					vim.lsp.codelens.refresh({ bufnr = bufnr })
				end,
			})
			vim.lsp.codelens.refresh({ bufnr = bufnr })
		end,
		handlers = {
			["jman.java/buildSyncStatus"] = function(_, result)
				set_sync(result)
				if result and result.state == "failed" then
					notify(result.error or "Build synchronization failed", vim.log.levels.ERROR)
				elseif result and result.state == "required" and state.options.build_sync == "prompt" then
					vim.schedule(function()
						vim.ui.select(
							{ "Sync now", "Later" },
							{ prompt = "JMAN Java project model changed" },
							function(choice)
								if choice == "Sync now" then
									M.sync()
								end
							end
						)
					end)
				end
			end,
		},
	})
	vim.lsp.enable("jman-java")
end

local function configure_keymaps()
	if not state.options.keymaps then
		return
	end
	local maps = {
		{ "<leader>js", M.sync, "JMAN Sync" },
		{ "<leader>jc", M.check, "JMAN Check" },
		{ "<leader>jb", M.build, "JMAN Build" },
		{ "<leader>jr", M.run, "JMAN Run" },
		{ "<leader>jt", M.test_nearest, "JMAN Test Nearest" },
		{ "<leader>jT", M.test, "JMAN Test All" },
		{ "<leader>jl", M.tests, "JMAN List Tests" },
		{ "<leader>ji", M.show_status, "JMAN Status" },
		{
			"<leader>ja",
			function()
				M.code_actions(nil, false)
			end,
			"JMAN Code Action",
		},
		{ "<leader>jo", M.organize_imports, "JMAN Organize Imports" },
	}
	for _, map in ipairs(maps) do
		vim.keymap.set("n", map[1], map[2], { desc = map[3] })
	end
end

local function validate_options(options)
	local build_systems = { auto = true, jman = true, gradle = true, maven = true }
	local sync_modes = { manual = true, prompt = true, automatic = true }
	if not build_systems[options.build_system] then
		error("jman.nvim: build_system must be auto, jman, gradle, or maven")
	end
	if not sync_modes[options.build_sync] then
		error("jman.nvim: build_sync must be manual, prompt, or automatic")
	end
	if options.cmd ~= nil and type(options.cmd) ~= "string" then
		error("jman.nvim: cmd must be a string")
	end
	if type(options.extra_env) ~= "table" then
		error("jman.nvim: extra_env must be a table")
	end
end

function M.setup(options)
	state.options = vim.tbl_deep_extend("force", vim.deepcopy(defaults), options or {})
	validate_options(state.options)
	create_commands()
	configure_keymaps()
	configure_lsp()
end

M._test = {
	build_tool_environment = build_tool_environment,
	human_test_arguments = human_test_arguments,
	lsp_diagnostics = lsp_diagnostics,
	operation_arguments = operation_arguments,
	operation_environment = operation_environment,
	root_and_system = root_and_system,
	server_environment = server_environment,
	status_message = status_message,
	nearest_test_item = nearest_test_item,
	test_selector = test_selector,
	wrapper_command = wrapper_command,
}

return M
