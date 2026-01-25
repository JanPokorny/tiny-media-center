local tinytoml = require("vendor.tinytoml")

local config_defaults = {
  media_path = ".",
}

local config = {}
for k, v in pairs(config_defaults) do config[k] = v end
local success, loaded_config = pcall(tinytoml.parse, (os.getenv("XDG_CONFIG_HOME") or os.getenv("HOME") .. "/.config") .. "/tiny-media-center/config.toml")
if success then for k, v in pairs(loaded_config) do config[k] = v end end
return config
