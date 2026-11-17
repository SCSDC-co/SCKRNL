local dap = require("dap")

local target = "aarch64-unknown-none-softfloat"
local kernel = vim.fn.getcwd() .. "/target/" .. target .. "/debug/SCKRNL"

local qemu_cmd = {
	"scripts/run",
	"--debug",
}

local qemu_job

local function stop_qemu()
	if qemu_job then
		pcall(vim.fn.jobstop, qemu_job)
		qemu_job = nil
	end
end

local function build()
	local out = vim.fn.system({ "cargo", "build" })
	if vim.v.shell_error ~= 0 then
		vim.notify(out, vim.log.levels.ERROR)
		return false
	end
	return true
end

local function start_qemu()
	stop_qemu()
	vim.cmd("botright 12new")
	qemu_job = vim.fn.jobstart(qemu_cmd, { term = true })
	vim.cmd("wincmd p")
end

local config = {
	name = "Kernel on QEMU (aarch64, A55)",
	type = "codelldb",
	request = "launch",
	targetCreateCommands = { "target create " .. kernel },
	processCreateCommands = { "gdb-remote localhost:1234" },
	stopOnEntry = false,
}

dap.configurations.rust = { config }

vim.keymap.set("n", "<leader>dk", function()
	if not build() then
		return
	end
	start_qemu()
	vim.defer_fn(function()
		dap.run(vim.deepcopy(config))
	end, 500)
end, { desc = "Debug kernel in QEMU" })

for _, ev in ipairs({ "event_terminated", "event_exited" }) do
	dap.listeners.after[ev]["kernel"] = stop_qemu
end
dap.listeners.before.disconnect["kernel"] = stop_qemu
