local conf = require("conf_parser")

local MEDIA_ROOT = os.getenv("TMC_MEDIA_PATH") or "./media"
local SAVE_DIR = love.filesystem.getSaveDirectory()

local state = {
  path = {},
  selectedIndex = 1,
  scrollOffset = 0,
  lastSelectedTarget = nil,
}

local mediaData = {}

local UI = {
  bgColor = {0, 0, 0},
  textColor = {0.67, 0.67, 0.67},
  accentColor = {1, 0.8, 0},
  dimColor = {0.27, 0.27, 0.27},
  fontSize = 72,
  itemHeight = 81,
}

-- ------------------------------------------------------------
-- Helpers
-- ------------------------------------------------------------

local function getFilePath(path)
  return MEDIA_ROOT .. "/" .. table.concat(path, "/")
end

-- ------------------------------------------------------------
-- Metadata IO
-- ------------------------------------------------------------

local function loadStoredMetadata(videoPath)
  local relativePath = table.concat(videoPath, "/")
  local content = love.filesystem.read("metadata/" .. relativePath .. ".conf")
  return conf.parse(content or "")
end

local function storeMetadata(videoPath, data)
  local relativePath = table.concat(videoPath, "/")
  love.filesystem.write("metadata/" .. relativePath .. ".conf", conf.serialize(data))
end

-- ------------------------------------------------------------
-- Media Tree Loading
-- ------------------------------------------------------------

local function loadMediaTree(currentPath, currentFsPath)
  local tree = {}
  local h = io.popen('ls -1 "' .. currentFsPath .. '" 2>/dev/null')
  for name in h:lines() do
    if name:sub(1, 1) ~= "." then
      local fullFsPath = currentFsPath .. "/" .. name
      local newPath = {unpack(currentPath)}
      table.insert(newPath, name)

      local d = io.popen('test -d "' .. fullFsPath .. '" && echo yes')
      local isDir = d:read("*a") ~= ""
      d:close()

      if isDir then
        tree[name] = loadMediaTree(newPath, fullFsPath)
        tree[name].isDir = true
      elseif name:match("%.mp4$") or name:match("%.mkv$") or name:match("%.avi$") then
        tree[name] = {
          type = "video",
          videoPath = newPath,
          storedMetadata = loadStoredMetadata(newPath)
        }
      elseif name:match("%.rvz$") then
        tree[name] = { type = "wii_game" }
      elseif name:match("%.sh$") then
        tree[name] = { type = "script" }
      end
    end
  end
  h:close()
  return tree
end

-- ------------------------------------------------------------
-- On-demand metadata extraction
-- ------------------------------------------------------------

local function getFileMetadata(node)
  if not node.fileMetadata then
    local h = io.popen(string.format(
      'mpv --script="%s" --msg-level=all=no "%s" 2>/dev/null',
      SAVE_DIR .. "/mpv/preflight.lua", getFilePath(node.videoPath)
    ))
    node.fileMetadata = conf.parse(h:read("*a"))
    h:close()
  end

  -- Sync duration from file metadata to stored metadata if missing
  if not node.storedMetadata.duration and node.fileMetadata.duration then
    node.storedMetadata.duration = node.fileMetadata.duration
    storeMetadata(node.videoPath, node.storedMetadata)
  end

  return node.fileMetadata
end

-- ------------------------------------------------------------
-- Path resolution & Data Access
-- ------------------------------------------------------------

local function getNodeFromPath(path)
  local node = mediaData.tree
  for _, seg in ipairs(path) do
    if type(node) ~= "table" then return nil end
    node = node[seg]
  end
  return node
end

local function getVideoNodeAndMenu()
  local videoIdx, menuType
  for i, seg in ipairs(state.path) do
    if seg:sub(1, 1) == ":" then
      videoIdx = i - 1
      menuType = seg
      break
    end
    local node = getNodeFromPath({unpack(state.path, 1, i)})
    if node and node.type == "video" then
      videoIdx = i
      menuType = state.path[i+1]
      break
    end
  end

  if not videoIdx then return nil, nil end

  local videoPath = {unpack(state.path, 1, videoIdx)}
  return getNodeFromPath(videoPath), menuType
end

local function getWatchPercentage(node)
  local metadata = node.storedMetadata
  if not metadata.duration then return 0 end
  local pct = math.floor((tonumber(metadata.position or 0) / tonumber(metadata.duration)) * 100 + 0.5)
  return (pct > 90) and 100 or pct
end

-- ------------------------------------------------------------
-- Menu construction
-- ------------------------------------------------------------

local function sortItems(items)
  table.sort(items, function(a, b)
    if a.node.isDir and not b.node.isDir then return true
    elseif not a.node.isDir and b.node.isDir then return false
    end

    if a.node.isDir then return a.label < b.label end

    local aIsVideo = a.node.type == "video"
    local bIsVideo = b.node.type == "video"

    local function getCategory(pct, isVid)
      if not isVid then return 2
      elseif pct >= 1 and pct <= 89 then return 1
      elseif pct == 0 then return 2
      else return 3
      end
    end

    local aPct = aIsVideo and getWatchPercentage(a.node) or 0
    local bPct = bIsVideo and getWatchPercentage(b.node) or 0

    local aCat = getCategory(aPct, aIsVideo)
    local bCat = getCategory(bPct, bIsVideo)

    if aCat ~= bCat then return aCat < bCat end
    if aCat == 1 and aPct ~= bPct then return aPct < bPct end

    return a.label < b.label
  end)
end

local function getMenuItems()
  local videoNode, menuType = getVideoNodeAndMenu()

  -- ----------------------------------------------------------
  -- Track submenus
  -- ----------------------------------------------------------
  if menuType == ":audio" or menuType == ":sub" then
    local items = {}
    local fileMetadata = getFileMetadata(videoNode)

    if menuType == ":sub" then
      table.insert(items, { label = "none", target = "none", action = "select_sub", trackId = "" })
    end

    for key, value in pairs(fileMetadata) do
      local trackType, trackId = key:match("^track_([^_]+)_(.*)$")
      if trackType and ":" .. trackType == menuType then
        table.insert(items, {
          label = value,
          target = value,
          action = "select_" .. trackType,
          trackId = trackId
        })
      end
    end

    table.sort(items, function(a, b) 
      if a.trackId == "" then return true end
      if b.trackId == "" then return false end
      return tonumber(a.trackId) < tonumber(b.trackId)
    end)
    return items
  end

  -- ----------------------------------------------------------
  -- Video menu
  -- ----------------------------------------------------------
  if videoNode then
    local fileMetadata = getFileMetadata(videoNode)
        local storedMetadata = videoNode.storedMetadata
    
        local pct = getWatchPercentage(videoNode)
        local playLabel = "play"
    
        if pct > 0 or storedMetadata.duration then
          local parts = {"play ["}
          table.insert(parts, (pct > 0 and pct or 0) .. "%")
          if storedMetadata.duration then
            local remaining = tonumber(storedMetadata.duration) - (tonumber(storedMetadata.position) or 0)
            table.insert(parts, ", ends at " .. os.date("%H:%M", os.time() + remaining))
          end
          table.insert(parts, "]")
          playLabel = table.concat(parts)
        end
    
        local audioLabel = "audio"
        if storedMetadata.aid and fileMetadata["track_audio_" .. storedMetadata.aid] then
          audioLabel = "audio [" .. fileMetadata["track_audio_" .. storedMetadata.aid] .. "]"
        end
    
        local subLabel = "subtitles"
        if storedMetadata.sid == "" then
          subLabel = "subtitles [none]"
        elseif storedMetadata.sid and fileMetadata["track_sub_" .. storedMetadata.sid] then
          subLabel = "subtitles [" .. fileMetadata["track_sub_" .. storedMetadata.sid] .. "]"
        end

    return {
      { label = playLabel, target = "play", action = "play", node = videoNode },
      { label = audioLabel, target = ":audio", action = "audio_menu", node = videoNode },
      { label = subLabel, target = ":sub", action = "sub_menu", node = videoNode },
    }
  end

  -- ----------------------------------------------------------
  -- Directory listing
  -- ----------------------------------------------------------
  local dirNode = getNodeFromPath(state.path)
  if not dirNode then return {} end

  local items = {}
  for name, childNode in pairs(dirNode) do
    if name ~= 'isDir' then
        local item = { label = name, target = name, node = childNode }

        if childNode.type == "video" then
            local pct = getWatchPercentage(childNode)
            if pct > 0 then
                item.label = name .. " [" .. pct .. "%]"
            end
        elseif childNode.type == "wii_game" then
            item.action = "play_wii_game"
        elseif childNode.type == "script" then
            item.action = "run_script"
        end
        table.insert(items, item)
    end
  end

  sortItems(items)
  return items
end

-- ------------------------------------------------------------
-- Navigation
-- ------------------------------------------------------------

function navigateIn()
  local items = getMenuItems()
  local item = items[state.selectedIndex]
  if not item then return end

  state.lastSelectedTarget = item.target
  local action = item.action or "browse"
  local node = item.node

  if action == "play" then
    local filePath = getFilePath(node.videoPath)
    local storedMetadata = node.storedMetadata

    local args = {
      "--fullscreen", "--msg-level=all=no",
      string.format('--config-dir="%s"', SAVE_DIR .. "/mpv"),
      string.format('--script="%s"', SAVE_DIR .. "/mpv/runtime.lua")
    }

    if storedMetadata.position and tonumber(storedMetadata.position) < tonumber(storedMetadata.duration) - 3 then
      table.insert(args, string.format("--start=%s", storedMetadata.position))
    end
    if storedMetadata.aid then table.insert(args, string.format("--aid=%s", storedMetadata.aid)) end
    if storedMetadata.sid and #storedMetadata.sid > 0 then table.insert(args, string.format("--sid=%s", storedMetadata.sid)) end

    local cmd = string.format('mpv %s "%s"', table.concat(args, " "), filePath)
    print("running: " .. cmd)
    local h = io.popen(cmd)
    local output = h:read("*a")
    h:close()
    print("mpv output:\n" .. output)

    local runtimeMetadata = conf.parse(output)
    for k, v in pairs(runtimeMetadata) do node.storedMetadata[k] = v end
    storeMetadata(node.videoPath, node.storedMetadata)

  elseif action == "play_wii_game" or action == "run_script" then
    local path = {unpack(state.path), item.target}
    local cmd = (action == "play_wii_game") and 'dolphin-emu --batch --exec="%s"' or 'bash "%s"'
    io.popen(string.format(cmd, getFilePath(path)))
  elseif action == "audio_menu" or action == "sub_menu" then
    table.insert(state.path, item.target)
    state.selectedIndex, state.scrollOffset = 1, 0
  elseif action == "select_audio" then
    local videoNode, _ = getVideoNodeAndMenu()
    videoNode.storedMetadata.aid = item.trackId
    storeMetadata(videoNode.videoPath, videoNode.storedMetadata)
    table.remove(state.path) -- Go back to video menu
    state.selectedIndex = 2 -- Select audio menu item (index 2)
    state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
  elseif action == "select_sub" then
    local videoNode, _ = getVideoNodeAndMenu()
    videoNode.storedMetadata.sid = item.trackId
    storeMetadata(videoNode.videoPath, videoNode.storedMetadata)
    table.remove(state.path) -- Go back to video menu
    state.selectedIndex = 3 -- Select subtitle menu item (index 3)
    state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
  else -- browse
    table.insert(state.path, item.target)
    state.selectedIndex, state.scrollOffset = 1, 0
  end
end

function navigateOut()
  if #state.path > 0 then
    local lastSegment = state.path[#state.path]
    table.remove(state.path)

    local items = getMenuItems()
    state.selectedIndex = 1

    for i, item in ipairs(items) do
      if item.target == lastSegment then
        state.selectedIndex = i
        break
      end
    end
  end
  state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
end

-- ------------------------------------------------------------
-- LOVE callbacks
-- ------------------------------------------------------------

function love.load()
  love.filesystem.createDirectory("metadata")
  love.filesystem.createDirectory("mpv")
  love.filesystem.write("mpv/preflight.lua", love.filesystem.read("attachments/mpv/preflight.lua"))
  love.filesystem.write("mpv/runtime.lua", love.filesystem.read("attachments/mpv/runtime.lua"))
  love.filesystem.write("mpv/mpv.conf", love.filesystem.read("attachments/mpv/mpv.conf"))
  love.filesystem.write("mpv/input.conf", love.filesystem.read("attachments/mpv/input.conf"))
  love.graphics.setFont(love.graphics.newFont("KodeMono-Regular.ttf", UI.fontSize))
  love.graphics.setBackgroundColor(UI.bgColor)
  love.mouse.setVisible(false)
  mediaData.tree = loadMediaTree({}, MEDIA_ROOT)
end

function love.update(dt)
  state.scrollOffset = state.scrollOffset + (((state.selectedIndex - 1) * UI.itemHeight) - state.scrollOffset) * 10 * dt
end

function love.draw()
  local w, h = love.graphics.getDimensions()
  local centerY = h / 2

  for i, item in ipairs(getMenuItems()) do
    local y = centerY - state.scrollOffset + (i - 1) * UI.itemHeight
    if i == state.selectedIndex then
      love.graphics.setColor(UI.accentColor)
      love.graphics.print("> " .. item.label, 50, y)
    else
      love.graphics.setColor(UI.textColor)
      love.graphics.print("  " .. item.label, 50, y)
    end
  end
  local title = state.path[#state.path] or "tiny media center"
  if title:sub(1,1) == ":" then title = title:sub(2) end
  love.graphics.setColor(UI.dimColor)
  love.graphics.print(title, 30, 20)
end

function love.keypressed(key)
  if key == "up" then state.selectedIndex = math.max(1, state.selectedIndex - 1)
  elseif key == "down" then state.selectedIndex = math.min(#getMenuItems(), state.selectedIndex + 1)
  elseif key == "return" then navigateIn()
  elseif key == "escape" or key == "appback" or key == "sleep" then navigateOut()
  elseif #key == 1 then
    local items = getMenuItems()
    for i = 0, #items - 1 do
      local index = 1 + (state.selectedIndex + i) % #items
      if items[index] and items[index].label:lower():sub(1, 1):lower() == key then
        state.selectedIndex = index
        break
      end
    end
  else
    print("unhandled key: " .. key)
  end
end