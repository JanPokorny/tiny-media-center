local MEDIA_ROOT = os.getenv("TMC_MEDIA_PATH") or "./media"

local state = {
  view = "BROWSER",
  currentPath = "",
  items = {},
  selectedIndex = 1,
  filter = "",
  scrollOffset = 0,
  targetScrollOffset = 0,
  movie = nil,
}

local UI = {
  bgColor = {0, 0, 0},
  textColor = {0.67, 0.67, 0.67},
  accentColor = {1, 0.8, 0},
  dimColor = {0.27, 0.27, 0.27},
  fontSize = 48,
  itemHeight = 54,
  font = nil,
}

function scanDirectory(path)
  local items = {}
  local fullPath = path == "" and MEDIA_ROOT or (MEDIA_ROOT .. "/" .. path)
  
  local handle = io.popen('ls -1 "' .. fullPath .. '" 2>/dev/null')
  if not handle then
    print("ERROR: Cannot open directory: " .. fullPath)
    return items
  end
  
  for file in handle:lines() do
    if file:sub(1,1) ~= "." then
      local itemPath = path == "" and file or (path .. "/" .. file)
      local itemFullPath = fullPath .. "/" .. file
      local testDir = io.popen('test -d "' .. itemFullPath .. '" && echo "yes"')
      local isDir = testDir and testDir:read("*a"):match("yes") or false
      if testDir then testDir:close() end
      
      if isDir then
        table.insert(items, {name = file, path = itemPath, fullPath = itemFullPath, type = "directory"})
      elseif file:match("%.mp4$") or file:match("%.mkv$") or file:match("%.avi$") or file:match("%.mov$") or file:match("%.webm$") then
        table.insert(items, {name = file:gsub("%.[^.]+$", ""), path = itemPath, fullPath = itemFullPath, type = "file"})
      elseif file:match("%.sh$") then
        table.insert(items, {name = file:gsub("%.sh$", ""), path = itemPath, fullPath = itemFullPath, type = "script"})
      end
    end
  end
  handle:close()
  
  print("Scanned " .. fullPath .. " - found " .. #items .. " items")
  return items
end

function filterItems(items, query)
  if query == "" then return items end
  
  local filtered = {}
  local lowerQuery = query:lower()
  for _, item in ipairs(items) do
    if item.name:lower():find(lowerQuery, 1, true) then
      table.insert(filtered, item)
    end
  end
  return filtered
end

function navigateBack()
  if state.filter ~= "" then
    state.filter = ""
    state.selectedIndex = 1
  elseif state.currentPath ~= "" then
    local parts = {}
    for part in state.currentPath:gmatch("[^/]+") do
      table.insert(parts, part)
    end
    table.remove(parts)
    state.currentPath = table.concat(parts, "/")
    state.items = scanDirectory(state.currentPath)
    state.selectedIndex = 1
    state.filter = ""
  end
end

function selectItem()
  local filtered = filterItems(state.items, state.filter)
  if #filtered == 0 then return end
  local item = filtered[state.selectedIndex]
  if item.type == "directory" then
    state.currentPath = item.path
    state.items = scanDirectory(state.currentPath)
    state.selectedIndex = 1
    state.filter = ""
  elseif item.type == "file" then
    state.movie = item
    state.view = "PLAYING"
    os.execute(string.format('mpv --config-dir="%s" "%s" &', love.filesystem.getSaveDirectory() .. "/conf/mpv", item.fullPath))
  elseif item.type == "script" then
    local scriptCmd = string.format('sh "%s" &', item.fullPath)
    print("Executing script: " .. scriptCmd)
    os.execute(scriptCmd)
  end
end

function love.load()
  love.filesystem.createDirectory('conf/mpv')
  love.filesystem.write('conf/mpv/mpv.conf', love.filesystem.read('conf/mpv/mpv.conf'))
  love.filesystem.write('conf/mpv/input.conf', love.filesystem.read('conf/mpv/input.conf'))
  love.mouse.setVisible(false)
  UI.font = love.graphics.newFont("KodeMono-regular.ttf", UI.fontSize)
  love.graphics.setFont(UI.font)
  state.items = scanDirectory("")
end

function love.update(dt)
  local diff = state.targetScrollOffset - state.scrollOffset
  state.scrollOffset = state.scrollOffset + diff * 10 * dt
end

function love.draw()
  love.graphics.setBackgroundColor(UI.bgColor)
  
  local w, h = love.graphics.getDimensions()
  local centerY = h / 2
  
  if state.view == "BROWSER" then
    love.graphics.setColor(UI.dimColor)
    local title = state.currentPath == "" and "tiny media center" or state.currentPath:match("[^/]+$")
    love.graphics.print(title, 30, 20)
    love.graphics.print(os.date("%H:%M"), w - 150, 20)
    
    local filtered = filterItems(state.items, state.filter)
    state.targetScrollOffset = (state.selectedIndex - 1) * UI.itemHeight
    
    love.graphics.stencil(function()
      love.graphics.rectangle("fill", 0, h * 0.1, w, h * 0.8)
    end, "replace", 1)
    love.graphics.setStencilTest("greater", 0)
    
    for i, item in ipairs(filtered) do
      local y = centerY - state.scrollOffset + (i - 1) * UI.itemHeight - 27
      local distFromCenter = math.abs(y - centerY)
      local fadeStart = h * 0.3
      local fadeEnd = h * 0.4
      local alpha = 1.0
      if distFromCenter > fadeStart then
        alpha = math.max(0, 1 - (distFromCenter - fadeStart) / (fadeEnd - fadeStart))
      end
      
      if i == state.selectedIndex then
        love.graphics.setColor(UI.accentColor[1], UI.accentColor[2], UI.accentColor[3], alpha)
        love.graphics.print("> " .. item.name, 50, y)
      else
        love.graphics.setColor(UI.textColor[1], UI.textColor[2], UI.textColor[3], alpha)
        love.graphics.print("  " .. item.name, 50, y)
      end
    end
    
    love.graphics.setStencilTest()
    
    if state.filter ~= "" then
      love.graphics.setColor(UI.textColor)
      love.graphics.print(state.filter .. "_", 30, h - 70)
    else
      love.graphics.setColor(UI.dimColor)
      love.graphics.print("type to search", 30, h - 70)
    end
    
  elseif state.view == "PLAYING" then
    love.graphics.setColor(UI.textColor)
    love.graphics.printf("Playing in mpv...\nPress ESC to return to browser", 0, h/2 - 50, w, "center")
  end
end

function love.keypressed(key)
  if state.view == "BROWSER" then
    if key == "up" then
      local filtered = filterItems(state.items, state.filter)
      state.selectedIndex = math.max(1, state.selectedIndex - 1)
    elseif key == "down" then
      local filtered = filterItems(state.items, state.filter)
      state.selectedIndex = math.min(#filtered, state.selectedIndex + 1)
    elseif key == "return" then
      selectItem()
    elseif key == "escape" or key == "acback" then
      navigateBack()
    elseif key == "backspace" and #state.filter > 0 then
      state.filter = state.filter:sub(1, -2)
      state.selectedIndex = 1
    elseif #key == 1 then
      state.filter = state.filter .. key
      state.selectedIndex = 1
    end
  elseif state.view == "PLAYING" then
    if key == "escape" then
      state.view = "BROWSER"
      state.movie = nil
    end
  end
end

function love.gamepadpressed(joystick, button)
  if button == "dpup" then
    love.keypressed("up")
  elseif button == "dpdown" then
    love.keypressed("down")
  elseif button == "a" then
    love.keypressed("return")
  elseif button == "b" then
    love.keypressed("escape")
  end
end
