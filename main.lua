local conf = require("conf_parser")

local MEDIA_ROOT = os.getenv("TMC_MEDIA_PATH") or "./media"
local SAVE_DIR = love.filesystem.getSaveDirectory()

local state = {path = {}, selectedIndex = 1, scrollOffset = 0}
local mediaTree = {}
local UI = {
  bgColor = {0, 0, 0}, textColor = {0.67, 0.67, 0.67},
  accentColor = {1, 0.8, 0}, dimColor = {0.27, 0.27, 0.27},
  fontSize = 72, itemHeight = 81
}

-- Metadata IO
local function metadataPath(videoPath)
  return "metadata/" .. table.concat(videoPath, "/") .. ".conf"
end

local function loadMetadata(videoPath)
  return conf.parse(love.filesystem.read(metadataPath(videoPath)) or "")
end

local function saveMetadata(videoPath, data)
  love.filesystem.write(metadataPath(videoPath), conf.serialize(data))
end

-- Media tree loading
local function loadTree(path, fsPath)
  local tree = {}
  local h = io.popen('ls -1 "' .. fsPath .. '" 2>/dev/null')
  for name in h:lines() do
    if name:sub(1, 1) ~= "." then
      local fullPath = fsPath .. "/" .. name
      local newPath = {unpack(path)}
      table.insert(newPath, name)
      local isDir = io.popen('test -d "' .. fullPath .. '" && echo yes'):read("*a") ~= ""
      
      if isDir then
        tree[name] = loadTree(newPath, fullPath)
        tree[name].isDir = true
      elseif name:match("%.mp4$") or name:match("%.mkv$") or name:match("%.avi$") then
        tree[name] = {type = "video", path = newPath, meta = loadMetadata(newPath)}
      elseif name:match("%.rvz$") then
        tree[name] = {type = "wii_game"}
      elseif name:match("%.sh$") then
        tree[name] = {type = "script"}
      end
    end
  end
  h:close()
  return tree
end

-- Get file metadata via mpv
local function getFileMeta(node)
  if not node.fileMeta then
    local h = io.popen(string.format('mpv --script="%s/mpv/preflight.lua" --msg-level=all=no "%s" 2>/dev/null',
      SAVE_DIR, MEDIA_ROOT .. "/" .. table.concat(node.path, "/")))
    node.fileMeta = conf.parse(h:read("*a"))
    h:close()
    
    if not node.meta.duration and node.fileMeta.duration then
      node.meta.duration = node.fileMeta.duration
      saveMetadata(node.path, node.meta)
    end
  end
  return node.fileMeta
end

-- Navigate tree by path
local function getNode(path)
  local node = mediaTree
  for _, seg in ipairs(path) do
    if type(node) ~= "table" then return nil end
    node = node[seg]
  end
  return node
end

-- Find video node and menu type from current path
local function getVideoContext()
  for i, seg in ipairs(state.path) do
    if seg:sub(1, 1) == ":" then
      return getNode({unpack(state.path, 1, i - 1)}), seg
    end
    local node = getNode({unpack(state.path, 1, i)})
    if node and node.type == "video" then
      return node, state.path[i + 1]
    end
  end
  return nil, nil
end

-- Calculate watch percentage
local function watchPct(node)
  if not node.meta.duration then return 0 end
  local pct = math.floor((tonumber(node.meta.position or 0) / tonumber(node.meta.duration)) * 100 + 0.5)
  return pct > 90 and 100 or pct
end

-- Sort menu items
local function sortItems(items)
  table.sort(items, function(a, b)
    if a.node.isDir ~= b.node.isDir then return a.node.isDir end
    if a.node.isDir then return a.label < b.label end
    
    local aVideo, bVideo = a.node.type == "video", b.node.type == "video"
    local aPct, bPct = aVideo and watchPct(a.node) or 0, bVideo and watchPct(b.node) or 0
    local aCat = not aVideo and 2 or (aPct >= 1 and aPct <= 89 and 1 or (aPct == 0 and 2 or 3))
    local bCat = not bVideo and 2 or (bPct >= 1 and bPct <= 89 and 1 or (bPct == 0 and 2 or 3))
    
    if aCat ~= bCat then return aCat < bCat end
    if aCat == 1 and aPct ~= bPct then return aPct < bPct end
    return a.label < b.label
  end)
end

-- Generate menu items
local function getMenuItems()
  local video, menu = getVideoContext()
  
  -- Track submenus
  if menu == ":audio" or menu == ":sub" then
    local items = {}
    local fm = getFileMeta(video)
    
    if menu == ":sub" then
      table.insert(items, {label = "none", target = "none", action = "select_sub", trackId = ""})
    end
    
    for key, value in pairs(fm) do
      local trackType, trackId = key:match("^track_([^_]+)_(.*)$")
      if trackType and ":" .. trackType == menu then
        table.insert(items, {label = value, target = value, action = "select_" .. trackType, trackId = trackId})
      end
    end
    
    table.sort(items, function(a, b)
      if a.trackId == "" then return true end
      if b.trackId == "" then return false end
      return tonumber(a.trackId) < tonumber(b.trackId)
    end)
    return items
  end
  
  -- Video menu
  if video then
    local fm, meta = getFileMeta(video), video.meta
    local pct = watchPct(video)
    local playLabel = "play"
    
    if pct > 0 or meta.duration then
      local parts = {"play [", pct .. "%"}
      if meta.duration then
        local remaining = tonumber(meta.duration) - (tonumber(meta.position) or 0)
        table.insert(parts, ", ends at " .. os.date("%H:%M", os.time() + remaining))
      end
      playLabel = table.concat(parts) .. "]"
    end
    
    local audioLabel = meta.aid and fm["track_audio_" .. meta.aid] 
      and "audio [" .. fm["track_audio_" .. meta.aid] .. "]" or "audio"
    
    local subLabel = meta.sid == "" and "subtitles [none]"
      or (meta.sid and fm["track_sub_" .. meta.sid] and "subtitles [" .. fm["track_sub_" .. meta.sid] .. "]" or "subtitles")
    
    return {
      {label = playLabel, target = "play", action = "play", node = video},
      {label = audioLabel, target = ":audio", action = "audio_menu", node = video},
      {label = subLabel, target = ":sub", action = "sub_menu", node = video}
    }
  end
  
  -- Directory listing
  local dir = getNode(state.path)
  if not dir then return {} end
  
  local items = {}
  for name, child in pairs(dir) do
    if name ~= 'isDir' then
      local item = {label = name, target = name, node = child}
      
      if child.type == "video" then
        local pct = watchPct(child)
        if pct > 0 then item.label = name .. " [" .. pct .. "%]" end
      elseif child.type == "wii_game" then
        item.action = "play_wii_game"
      elseif child.type == "script" then
        item.action = "run_script"
      end
      table.insert(items, item)
    end
  end
  
  sortItems(items)
  return items
end

-- Navigation
function navigateIn()
  local items = getMenuItems()
  local item = items[state.selectedIndex]
  if not item then return end
  
  local action = item.action or "browse"
  
  if action == "play" then
    local node = item.node
    local args = {
      "--fullscreen", "--msg-level=all=no",
      string.format('--config-dir="%s/mpv"', SAVE_DIR),
      string.format('--script="%s/mpv/runtime.lua"', SAVE_DIR)
    }
    
    if node.meta.position and tonumber(node.meta.position) < tonumber(node.meta.duration) - 3 then
      table.insert(args, "--start=" .. node.meta.position)
    end
    if node.meta.aid then table.insert(args, "--aid=" .. node.meta.aid) end
    if node.meta.sid and #node.meta.sid > 0 then table.insert(args, "--sid=" .. node.meta.sid) end
    
    local h = io.popen(string.format('mpv %s "%s"', table.concat(args, " "), 
      MEDIA_ROOT .. "/" .. table.concat(node.path, "/")))
    local output = h:read("*a")
    h:close()
    
    for k, v in pairs(conf.parse(output)) do node.meta[k] = v end
    saveMetadata(node.path, node.meta)
    
  elseif action == "play_wii_game" or action == "run_script" then
    local cmd = action == "play_wii_game" and 'dolphin-emu --batch --exec="%s"' or 'bash "%s"'
    local filePath = MEDIA_ROOT .. "/" .. table.concat(state.path, "/") .. "/" .. item.target
    io.popen(string.format(cmd, filePath))
    
  elseif action == "audio_menu" or action == "sub_menu" then
    table.insert(state.path, item.target)
    state.selectedIndex, state.scrollOffset = 1, 0
    
  elseif action == "select_audio" or action == "select_sub" then
    local video = getVideoContext()
    video.meta[action == "select_audio" and "aid" or "sid"] = item.trackId
    saveMetadata(video.path, video.meta)
    table.remove(state.path)
    state.selectedIndex = action == "select_audio" and 2 or 3
    state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
    
  else -- browse
    table.insert(state.path, item.target)
    state.selectedIndex, state.scrollOffset = 1, 0
  end
end

function navigateOut()
  if #state.path > 0 then
    local last = state.path[#state.path]
    table.remove(state.path)
    
    state.selectedIndex = 1
    for i, item in ipairs(getMenuItems()) do
      if item.target == last then
        state.selectedIndex = i
        break
      end
    end
    state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
  end
end

-- LOVE callbacks
function love.load()
  love.filesystem.createDirectory("metadata")
  love.filesystem.createDirectory("mpv")
  for _, file in ipairs({"preflight.lua", "runtime.lua", "mpv.conf", "input.conf"}) do
    love.filesystem.write("mpv/" .. file, love.filesystem.read("attachments/mpv/" .. file))
  end
  love.graphics.setFont(love.graphics.newFont("KodeMono-Regular.ttf", UI.fontSize))
  love.graphics.setBackgroundColor(UI.bgColor)
  love.mouse.setVisible(false)
  mediaTree = loadTree({}, MEDIA_ROOT)
end

function love.update(dt)
  state.scrollOffset = state.scrollOffset + (((state.selectedIndex - 1) * UI.itemHeight) - state.scrollOffset) * 10 * dt
end

function love.draw()
  local w, h = love.graphics.getDimensions()
  for i, item in ipairs(getMenuItems()) do
    local y = h / 2 - state.scrollOffset + (i - 1) * UI.itemHeight
    love.graphics.setColor(i == state.selectedIndex and UI.accentColor or UI.textColor)
    love.graphics.print((i == state.selectedIndex and "> " or "  ") .. item.label, 50, y)
  end
  
  local title = state.path[#state.path] or "tiny media center"
  if title:sub(1, 1) == ":" then title = title:sub(2) end
  love.graphics.setColor(UI.dimColor)
  love.graphics.print(title, 30, 20)
end

function love.keypressed(key)
  if key == "up" then
    state.selectedIndex = math.max(1, state.selectedIndex - 1)
  elseif key == "down" then
    state.selectedIndex = math.min(#getMenuItems(), state.selectedIndex + 1)
  elseif key == "return" then
    navigateIn()
  elseif key == "escape" or key == "appback" or key == "sleep" then
    navigateOut()
  elseif #key == 1 then
    local items = getMenuItems()
    for i = 0, #items - 1 do
      local index = 1 + (state.selectedIndex + i) % #items
      if items[index] and items[index].label:lower():sub(1, 1) == key then
        state.selectedIndex = index
        break
      end
    end
  end
end

function love.mousepressed(x, y, button, istouch, presses)
  if button == 1 then
    navigateIn()
  elseif button == 2 then
    navigateOut()
  end
end
