local toml = require("utils.tinytoml")
local Conf = {}

function Conf.parse(text)
  if not text then return {} end
  return toml.parse(text, {load_from_string=true})
end

function Conf.stringify(data)
  return toml.encode(data)
end

return Conf
