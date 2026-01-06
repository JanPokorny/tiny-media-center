local conf = require("conf_parser")

local MEDIA_ROOT = os.getenv("TMC_MEDIA_PATH") or "./media"
local SAVE_DIR = love.filesystem.getSaveDirectory()
local MPV_DIR = SAVE_DIR .. "/mpv"

local state = {
  path = {},
  selectedIndex = 1,
  scrollOffset = 0,
  targetScrollOffset = 0,
  searchQuery = "",
  lastSelectedTarget = nil,
}

local cache = {
  tree = {},
  tracks = {},
  playbackOptions = {},
  metadata = {},
}

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

local function isVideoFile(name)
  return name:match("%.mp4$") or name:match("%.mkv$")
      or name:match("%.avi$") or name:match("%.mov$")
      or name:match("%.webm$")
end

local function isWiiGameFile(name)
  return name:match("%.iso$") or name:match("%.rvz$")
end

local function getFilePath(path)
  return MEDIA_ROOT .. "/" .. table.concat(path, "/")
end

-- ------------------------------------------------------------
-- Media tree
-- ------------------------------------------------------------

local function loadMediaTree(path)
  local tree = {}
  local h = io.popen('ls -1 "' .. path .. '" 2>/dev/null')
  for name in h:lines() do
    if name:sub(1, 1) ~= "." then
      local full = path .. "/" .. name
      local d = io.popen('test -d "' .. full .. '" && echo yes')
      local isDir = d:read("*a") ~= ""
      d:close()

      if isDir then
        tree[name] = loadMediaTree(full)
      elseif isVideoFile(name) then
        tree[name] = { type = "video" }
      elseif isWiiGameFile(name) then
        tree[name] = { type = "wii_game" }
      end
    end
  end
  h:close()
  return tree
end

-- ------------------------------------------------------------
-- mpv integration
-- ------------------------------------------------------------

local function loadTracks(filePath)
  if cache.tracks[filePath] then
    return cache.tracks[filePath]
  end

  local h = io.popen(string.format(
    'mpv --script="%s" --msg-level=all=no "%s" 2>/dev/null',
    MPV_DIR .. "/print-info.lua", filePath
  ))
  local out = h:read("*a")
  h:close()

  local tracks = conf.parse(out)
  cache.tracks[filePath] = tracks
  return tracks
end

local function loadMetadata(videoPath)
  local relativePath = table.concat(videoPath, "/")
  
  if cache.metadata[relativePath] then
    return cache.metadata[relativePath]
  end
  
  local metadataFileRelativePath = "metadata/" .. relativePath .. ".conf"
  local content, size = love.filesystem.read(metadataFileRelativePath)
  if not content then 
    cache.metadata[relativePath] = {}
    return {}
  end
  
  local metadata = conf.parse(content)
  cache.metadata[relativePath] = metadata
  return metadata
end

local function saveMetadata(videoPath, data)
  local relativePath = table.concat(videoPath, "/")
  local metadataFileRelativePath = "metadata/" .. relativePath .. ".conf"
  love.filesystem.write(metadataFileRelativePath, conf.serialize(data))
  cache.metadata[relativePath] = data
end

local function getWatchPercentage(videoPath)
  local metadata = loadMetadata(videoPath)
  local pos = tonumber(metadata.position)
  local dur = tonumber(metadata.duration)
  
  if not pos or not dur or dur == 0 then
    return 0
  end
  
  local pct = math.floor((pos / dur) * 100 + 0.5)
  
  if pct > 90 then
    return 100
  end
  
  return pct
end

local function preloadMetadataForLevel()
  local node = cache.tree
  for _, seg in ipairs(state.path) do
    node = node[seg]
    if not node then return end
  end

  for name, child in pairs(node) do
    if child.type == "video" then
      local videoPath = {}
      for _, seg in ipairs(state.path) do
        table.insert(videoPath, seg)
      end
      table.insert(videoPath, name)
      loadMetadata(videoPath)
    end
  end
end

-- ------------------------------------------------------------
-- Path resolution
-- ------------------------------------------------------------

local function getVideoPathAndMenu()
  local node = cache.tree
  local videoIdx

  for i, seg in ipairs(state.path) do
    if seg:sub(1, 1) == ":" then
      videoIdx = i - 1
      break
    end
    node = node[seg]
    if node and node.type == "video" then
      videoIdx = i
      break
    end
  end

  if not videoIdx then return nil, nil end

  local videoPath = {}
  for i = 1, videoIdx do
    table.insert(videoPath, state.path[i])
  end

  return videoPath, state.path[videoIdx + 1]
end

-- ------------------------------------------------------------
-- Menu construction
-- ------------------------------------------------------------

local function buildTrackLabel(prefix, track)
  local label = prefix .. " " .. track.id
  if track.lang then label = label .. " [" .. track.lang .. "]" end
  if track.title then label = label .. " - " .. track.title end
  if track.channels then label = label .. " (" .. track.channels .. "ch)" end
  return label
end

local function sortItems(items)
  table.sort(items, function(a, b)
    if a.isDir and not b.isDir then
      return true
    elseif not a.isDir and b.isDir then
      return false
    end
    
    if a.isDir then
      return a.label < b.label
    end
    
    local aPct = a.watchPct or 0
    local bPct = b.watchPct or 0
    local aIsVideo = a.isVideo
    local bIsVideo = b.isVideo
    
    local function getCategory(pct, isVid)
      if not isVid then return 2
      elseif pct >= 1 and pct <= 89 then return 1
      elseif pct == 0 then return 2
      else return 3
      end
    end
    
    local aCat = getCategory(aPct, aIsVideo)
    local bCat = getCategory(bPct, bIsVideo)
    
    if aCat ~= bCat then
      return aCat < bCat
    end
    
    if aCat == 1 then
      if aPct ~= bPct then
        return aPct < bPct
      end
    end
    
    return a.label < b.label
  end)
end

local function getMenuItems()
  local videoPath, menuType = getVideoPathAndMenu()

  -- ----------------------------------------------------------
  -- Track submenus
  -- ----------------------------------------------------------

  if menuType == ":audio" or menuType == ":sub" then
    local filePath = getFilePath(videoPath)
    local tracks = loadTracks(filePath)
    local items = {}

    if menuType == ":sub" then
      table.insert(items, {
        label = "none",
        target = "none",
        action = "select_sub",
        trackId = ""
      })
    end

    for _, track in pairs(tracks) do
      if (menuType == ":audio" and track.type == "audio")
      or (menuType == ":sub"   and track.type == "sub") then
        local label = buildTrackLabel(track.type:sub(1,1), track)
        table.insert(items, {
          label = label,
          target = label,
          action = menuType == ":audio" and "select_audio" or "select_sub",
          trackId = track.id
        })
      end
    end

    table.sort(items, function(a, b)
      return a.trackId < b.trackId
    end)

    return items
  end

  -- ----------------------------------------------------------
  -- Video menu
  -- ----------------------------------------------------------

  if videoPath then
    local filePath = getFilePath(videoPath)
    local tracks = loadTracks(filePath)
    local playbackOptions = cache.playbackOptions[filePath] or {}

    local pct = getWatchPercentage(videoPath)
    local playLabel = "play"
    if pct > 0 then
      playLabel = "play [" .. pct .. "%]"
    end

    local items = {
      { label = playLabel, target = "play", action = "play" }
    }

    -- Audio label
    local aid = tonumber(playbackOptions.aid)
    if not aid then
      for _, t in pairs(tracks) do
        if t.type == "audio" and t.selected == "yes" then
          aid = tonumber(t.id)
          break
        end
      end
    end

    local audioLabel = "audio"
    if aid and tracks["a:" .. aid] then
      audioLabel = "audio [" .. (tracks["a:" .. aid].lang or "?") .. "]"
    end
    table.insert(items, { label = audioLabel, target = ":audio", action = "audio_menu" })

    -- Subtitle label
    local sid = playbackOptions.sid
    if sid == nil then
        for _, t in pairs(tracks) do
            if t.type == "sub" and t.selected == "yes" then
                sid = t.id
                break
            end
        end
    end
    if sid then sid = tostring(sid) end

    local subLabel = "subtitles"
    if sid == "" then
      subLabel = "subtitles [none]"
    elseif sid and tracks["s:" .. sid] then
      subLabel = "subtitles [" .. (tracks["s:" .. sid].lang or "und") .. "]"
    end
    table.insert(items, { label = subLabel, target = ":sub", action = "sub_menu" })

    return items
  end

  -- ----------------------------------------------------------
  -- Directory listing
  -- ----------------------------------------------------------

  local node = cache.tree
  for _, seg in ipairs(state.path) do
    node = node[seg]
    if not node then return {} end
  end

  local items = {}
  for name, child in pairs(node) do
    local item = { 
      label = name,
      target = name
    }
    
    if type(child) == "table" and not child.type then
      item.isDir = true
    elseif child.type == "video" then
      item.isVideo = true
      local videoPath = {}
      for _, seg in ipairs(state.path) do
        table.insert(videoPath, seg)
      end
      table.insert(videoPath, name)
      
      local pct = getWatchPercentage(videoPath)
      item.watchPct = pct
      
      if pct > 0 then
        item.label = name .. " [" .. pct .. "%]"
      end
    elseif child.type == "wii_game" then
      item.isWiiGame = true
      item.action = "play_wii_game"
    end
    
    table.insert(items, item)
  end
  
  sortItems(items)

  if state.searchQuery ~= "" then
    local q = state.searchQuery:lower()
    local filtered = {}
    for _, it in ipairs(items) do
      if it.label:lower():find(q, 1, true) then
        table.insert(filtered, it)
      end
    end
    return filtered
  end

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

  if item.action == "play" then
    local videoPath, _ = getVideoPathAndMenu()
    local filePath = getFilePath(videoPath)
    local metadata = loadMetadata(videoPath)
    local playbackOptions = cache.playbackOptions[filePath] or {}

    local args = {
      "--fullscreen",
      "--msg-level=all=no",
      string.format('--config-dir="%s"', MPV_DIR),
      string.format('--script="%s"', MPV_DIR .. "/print-position.lua")
    }

    if metadata.position and tonumber(metadata.position) < tonumber(metadata.duration) - 3 then
      table.insert(args, string.format("--start=%s", metadata.position))
    end
    if playbackOptions.aid then
      table.insert(args, string.format("--aid=%s", playbackOptions.aid))
    end
    if playbackOptions.sid and #playbackOptions.sid > 0 then
      table.insert(args, string.format("--sid=%s", playbackOptions.sid))
    end

    local cmd = string.format('mpv %s "%s"', table.concat(args, " "), filePath)
    print("running: " .. cmd)
    local h = io.popen(cmd)
    local output = h:read("*a")
    h:close()
    print("mpv output:\n" .. output)
    saveMetadata(videoPath, conf.parse(output))
  elseif item.action == "play_wii_game" then
    local gamePath = {unpack(state.path)}
    table.insert(gamePath, item.target)
    local filePath = getFilePath(gamePath)
    local cmd = string.format('dolphin --batch --exec="%s"', filePath)
    print("running: " .. cmd)
    io.popen(cmd)
  elseif item.action == "audio_menu" then
    table.insert(state.path, ":audio")
    state.selectedIndex = 1
    state.scrollOffset = 0
  elseif item.action == "sub_menu" then
    table.insert(state.path, ":sub")
    state.selectedIndex = 1
    state.scrollOffset = 0
  elseif item.action == "select_audio" then
    local videoPath, _ = getVideoPathAndMenu()
    local filePath = getFilePath(videoPath)
    if not cache.playbackOptions[filePath] then cache.playbackOptions[filePath] = {} end
    cache.playbackOptions[filePath].aid = item.trackId
    table.remove(state.path)
    state.selectedIndex = 2
    state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
  elseif item.action == "select_sub" then
    local videoPath, _ = getVideoPathAndMenu()
    local filePath = getFilePath(videoPath)
    if not cache.playbackOptions[filePath] then cache.playbackOptions[filePath] = {} end
    cache.playbackOptions[filePath].sid = item.trackId
    table.remove(state.path)
    state.selectedIndex = 3
    state.scrollOffset = (state.selectedIndex - 1) * UI.itemHeight
  else
    table.insert(state.path, item.target)
    state.selectedIndex = 1
    state.searchQuery = ""
    state.scrollOffset = 0
    preloadMetadataForLevel()
  end
end

function navigateOut()
  if state.searchQuery ~= "" then
    state.searchQuery = ""
  elseif #state.path > 0 then
    local lastSegment = state.path[#state.path]
    table.remove(state.path)
    
    preloadMetadataForLevel()
    
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
  love.filesystem.write("mpv/print-info.lua", love.filesystem.read("attachments/mpv/print-info.lua"))
  love.filesystem.write("mpv/print-position.lua", love.filesystem.read("attachments/mpv/print-position.lua"))
  love.filesystem.write("mpv/mpv.conf", love.filesystem.read("attachments/mpv/mpv.conf"))
  love.filesystem.write("mpv/input.conf", love.filesystem.read("attachments/mpv/input.conf"))
  love.graphics.setFont(love.graphics.newFont("KodeMono-Regular.ttf", UI.fontSize))
  love.graphics.setBackgroundColor(UI.bgColor)
  love.mouse.setVisible(false)
  cache.tree = loadMediaTree(MEDIA_ROOT)
  preloadMetadataForLevel()
end

function love.update(dt)
  state.scrollOffset =
    state.scrollOffset + (state.targetScrollOffset - state.scrollOffset) * 10 * dt
end

function love.draw()
  local w, h = love.graphics.getDimensions()
  local centerY = h / 2

  local title = state.path[#state.path] or "tiny media center"
  if title:sub(1,1) == ":" then title = title:sub(2) end

  love.graphics.setColor(UI.dimColor)
  love.graphics.print(title, 30, 20)

  local items = getMenuItems()
  state.targetScrollOffset = (state.selectedIndex - 1) * UI.itemHeight

  for i, item in ipairs(items) do
    local y = centerY - state.scrollOffset + (i - 1) * UI.itemHeight
    if i == state.selectedIndex then
      love.graphics.setColor(UI.accentColor)
      love.graphics.print("> " .. item.label, 50, y)
    else
      love.graphics.setColor(UI.textColor)
      love.graphics.print("  " .. item.label, 50, y)
    end
  end
end

function love.keypressed(key)
  if key == "up" then state.selectedIndex = math.max(1, state.selectedIndex - 1)
  elseif key == "down" then state.selectedIndex = math.min(#getMenuItems(), state.selectedIndex + 1)
  elseif key == "return" then navigateIn()
  elseif key == "escape" or key == "appback" or key == "sleep" then navigateOut()
  elseif key == "backspace" then
    state.searchQuery = state.searchQuery:sub(1, -2)
    state.selectedIndex = 1
    state.scrollOffset = 0
  elseif #key == 1 then
    state.searchQuery = state.searchQuery .. key
    state.selectedIndex = 1
    state.scrollOffset = 0
  else
    print("unhandled key: " .. key)
  end
end