if vim.g.loaded_jman_repository_bridge then
	return
end
vim.g.loaded_jman_repository_bridge = true

local source = debug.getinfo(1, "S").source:sub(2)
local repository = vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(source)))
local runtime = repository .. "/editors/neovim"
if vim.fn.isdirectory(runtime) == 1 then
	vim.opt.runtimepath:prepend(runtime)
end
