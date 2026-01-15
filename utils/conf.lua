local Conf = {}

function Conf.parse(text)
  if not text then return {} end
  local result = {}
  local currentSection = result
  for line in text:gmatch("[^\r\n]+") do
    line = line:match("^%s*(.-)%s*$")
    if line ~= "" and line:sub(1, 1) ~= "#" then
      local section = line:match("^%[(.+)%]$")
      if section then
        result[section] = result[section] or {}
        currentSection = result[section]
      else
        local key, value = line:match("^([^=]+)=(.*)$")
        if key then
          currentSection[key] = value
        end
      end
    end
  end
  return result
end

function Conf.stringify(data)
  local lines = {}
  for key, value in pairs(data) do
    if type(value) == "table" then
      table.insert(lines, "[" .. key .. "]")
      for k, v in pairs(value) do
        table.insert(lines, k .. "=" .. tostring(v))
      end
    else
      table.insert(lines, key .. "=" .. tostring(value))
    end
  end
  return table.concat(lines, "\n") .. "\n"
end

return Conf