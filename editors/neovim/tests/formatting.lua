vim.opt.runtimepath:prepend(vim.fn.getcwd() .. "/editors/neovim")

local jman = require("jman")
local original_get_clients = vim.lsp.get_clients
local original_format = vim.lsp.buf.format
local received
local client = {
	id = 42,
	server_capabilities = { documentFormattingProvider = true },
}

vim.lsp.get_clients = function(options)
	assert(options.bufnr == 7, "Formatting must select the requested buffer")
	assert(options.name == "jman-java", "Formatting must select only JMAN Java")
	return { client }
end
vim.lsp.buf.format = function(options)
	received = options
end

assert(jman.format(7) == true, "JMAN formatting was not dispatched")
assert(received.bufnr == 7, "Formatting used the wrong buffer")
assert(received.id == 42, "Formatting was not restricted to the JMAN client")
assert(received.async == false, "Explicit formatting must complete before returning")
assert(received.timeout_ms == 5000, "Default formatting timeout was not applied")

vim.lsp.get_clients = original_get_clients
vim.lsp.buf.format = original_format
vim.cmd("qa!")
