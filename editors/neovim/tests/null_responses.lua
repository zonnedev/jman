vim.opt.runtimepath:prepend(vim.fn.getcwd() .. "/editors/neovim")

local jman = require("jman")

local response = {
	workspace = {
		buildSystem = "jman",
		buildRuntime = vim.NIL,
	},
	cache = {
		bytes = 0,
		structural = vim.NIL,
		semantic = vim.NIL,
	},
	buildSync = vim.NIL,
	indexedDocuments = 3,
	semanticDocuments = vim.NIL,
	openDocuments = 1,
}
local message = jman._test.status_message(response)

assert(message:match("^jman · 3 indexed · 0 semantic · 1 open"), "status must render JSON null values safely")
assert(message:match("sync unknown"), "null build synchronization state must be reported as unknown")
assert(not message:match(" on Java "), "null build runtime must not produce a runtime summary")

local empty = jman._test.status_message(vim.NIL)
assert(empty:match("^unknown · 0 indexed"), "a null status response must render safe defaults")

local notified
local original_clients = vim.lsp.get_clients
local original_notify = vim.notify
vim.lsp.get_clients = function()
	return {
		{
			request = function(_, method, _, callback)
				assert(method == "workspace/executeCommand", "status must use the execute-command request")
				callback(nil, response)
			end,
		},
	}
end
vim.notify = function(value)
	notified = value
end
jman.show_status()
vim.lsp.get_clients = original_clients
vim.notify = original_notify

assert(notified == message, "JmanStatus must safely render a response containing JSON null values")

vim.cmd("qa!")
