local source = debug.getinfo(1, "S").source:sub(2)
local repository = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(vim.fs.normalize(source))))
local runtime = repository .. "/editors/neovim"

if not vim.tbl_contains(vim.opt.runtimepath:get(), runtime) then
	vim.opt.runtimepath:prepend(runtime)
end

return dofile(runtime .. "/lua/jman/init.lua")
