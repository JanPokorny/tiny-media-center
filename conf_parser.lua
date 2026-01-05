-- Simple parser for key=value config format with [section] headers

local M = {}

function M.parse(text)
  local result = {}
  local currentSection = result
  
  for line in text:gmatch("[^\r\n]+") do
    line = line:match("^%s*(.-)%s*$")
    if line ~= "" and line:sub(1, 1) ~= "#" then
      local section = line:match("^%[(.+)%]$")
      if section then
        result[section] = {}
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

function M.serialize(data)
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

return M