local M = {}

local defaults = {
	cmd = nil,
	java_home = nil,
	build_system = "auto",
	build_sync = "prompt",
	extra_env = {},
	notify = true,
	keymaps = true,
}

local state = {
	options = vim.deepcopy(defaults),
	sync = { state = "starting" },
	clients = {},
}

local build_markers = {
	jman = { "jman.toml" },
	gradle = { "settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts" },
	maven = { "pom.xml" },
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
	for directory in parents(path) do
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
		if result and result.buildSync then
			state.sync = result.buildSync
		end
		if callback then
			callback(result)
		end
	end, 0)
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
		if not actions or #actions == 0 then
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

local function terminal(args, title)
	local file = vim.api.nvim_buf_get_name(0)
	local root, system = root_and_system(file)
	if not root or system ~= "jman" then
		notify("JMAN build operations require a native jman.toml workspace", vim.log.levels.ERROR)
		return
	end
	local command = { executable() }
	vim.list_extend(command, args)
	vim.cmd("botright new")
	local buffer = vim.api.nvim_get_current_buf()
	vim.api.nvim_buf_set_name(buffer, "JMAN://" .. title)
	vim.bo[buffer].bufhidden = "wipe"
	vim.fn.termopen(command, {
		cwd = root,
		env = vim.tbl_extend("force", vim.fn.environ(), state.options.extra_env),
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

function M.status()
	local client = client_for_buffer(0)
	local sync = state.sync.state or "unknown"
	return client and ("JMAN:" .. sync) or "JMAN:off"
end

function M.show_status()
	execute_command("jman.java.status", function(result)
		if not result then
			return
		end
		local workspace = result.workspace or {}
		local cache = result.cache or {}
		notify(
			string.format(
				"%s workspace · %d indexed · %d semantic · sync %s · cache %.1f MiB",
				workspace.buildSystem or "unknown",
				result.indexedDocuments or 0,
				result.semanticDocuments or 0,
				result.buildSync and result.buildSync.state or "unknown",
				(cache.bytes or 0) / 1024 / 1024
			)
		)
	end)
end

function M.sync()
	local file = vim.api.nvim_buf_get_name(0)
	local root, system = root_and_system(file)
	if not root then
		notify("No Java workspace was detected", vim.log.levels.ERROR)
		return
	end
	if system ~= "jman" then
		execute_command("jman.java.syncWorkspace", function()
			notify("Java project model synchronized")
		end)
		return
	end
	vim.system({ executable(), "--no-progress", "sync", root }, {
		cwd = root,
		env = vim.tbl_extend("force", vim.fn.environ(), state.options.extra_env),
		text = true,
	}, function(result)
		vim.schedule(function()
			if result.code ~= 0 then
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
	terminal({ "check" }, "Check")
end
function M.build()
	terminal({ "build" }, "Build")
end
function M.run()
	terminal({ "run" }, "Run")
end
function M.test()
	terminal({ "test" }, "Test")
end

function M.test_pattern(pattern)
	pattern = pattern or vim.fn.input("JMAN test pattern: ")
	if pattern and pattern ~= "" then
		terminal({ "test", "--tests", pattern }, "Test " .. pattern)
	end
end

function M.test_nearest()
	local selector, error = test_selector(0, vim.api.nvim_win_get_cursor(0)[1])
	if not selector then
		notify(error, vim.log.levels.ERROR)
		return
	end
	M.test_pattern(selector)
end

function M.tests()
	local client = client_for_buffer(0)
	if not client then
		notify("No JMAN Java language server is attached", vim.log.levels.WARN)
		return
	end
	client:request("jman.java/tests/discover", {}, function(error, result)
		if error then
			notify(error.message or tostring(error), vim.log.levels.ERROR)
			return
		end
		local tests = vim.tbl_filter(function(item)
			return item.kind == "test"
		end, result and result.items or {})
		vim.schedule(function()
			vim.ui.select(tests, {
				prompt = "JMAN tests",
				format_item = function(item)
					return item.selector
				end,
			}, function(item)
				if item then
					M.test_pattern(item.selector)
				end
			end)
		end)
	end, 0)
end

function M.rebuild_index()
	execute_command("jman.java.rebuildIndex", function()
		notify("JMAN Java index rebuilt")
	end)
end

function M.clear_cache()
	execute_command("jman.java.clearWorkspaceCache", function()
		notify("JMAN Java cache cleared and rebuilt")
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
				if not number then
					notify("Argument order must contain integers", vim.log.levels.ERROR)
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
				elseif edit then
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
	}
	for name, callback in pairs(commands) do
		vim.api.nvim_create_user_command(name, callback, { force = true })
	end
	vim.api.nvim_create_user_command("JmanTestPattern", function(command)
		M.test_pattern(command.args)
	end, { nargs = "?", force = true })
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
		cmd_env = vim.tbl_extend("force", state.options.extra_env, {
			JAVA_HOME = state.options.java_home or vim.env.JAVA_HOME,
		}),
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
				state.sync = result or { state = "unknown" }
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
		{ "<leader>jman", M.build, "JMAN Build" },
		{ "<leader>jr", M.run, "JMAN Run" },
		{ "<leader>jt", M.test_nearest, "JMAN Test Nearest" },
		{ "<leader>jT", M.test, "JMAN Test All" },
		{ "<leader>jl", M.tests, "JMAN List Tests" },
		{ "<leader>ji", M.show_status, "JMAN Status" },
	}
	for _, map in ipairs(maps) do
		vim.keymap.set("n", map[1], map[2], { desc = map[3] })
	end
end

function M.setup(options)
	state.options = vim.tbl_deep_extend("force", vim.deepcopy(defaults), options or {})
	create_commands()
	configure_keymaps()
	configure_lsp()
end

M._test = {
	lsp_diagnostics = lsp_diagnostics,
	root_and_system = root_and_system,
	test_selector = test_selector,
}

return M
