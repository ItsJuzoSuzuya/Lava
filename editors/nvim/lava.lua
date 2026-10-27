-- Lava support for Neovim 0.11+: filetype, syntax and the language server.
--
-- Load it from your config, for example in ~/.config/nvim/plugin/lava.lua:
--
--   dofile("/home/nick/Lava/editors/nvim/lava.lua")
--
-- The server is the `lava` command if it's installed, otherwise the release
-- build of this repository (`cargo build --release`).

local root = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":h:h:h")

-- Syntax highlighting and buffer settings live next to this file
vim.opt.runtimepath:append(root .. "/editors/nvim")
vim.filetype.add({ extension = { lv = "lava" } })

local has_cmp, cmp_lsp = pcall(require, "cmp_nvim_lsp")

vim.lsp.config("lava", {
  cmd = { vim.fn.executable("lava") == 1 and "lava" or root .. "/target/release/lava", "lsp" },
  filetypes = { "lava" },
  root_markers = { "main.lv", ".git" },
  capabilities = has_cmp and cmp_lsp.default_capabilities() or nil,
})
vim.lsp.enable("lava")
