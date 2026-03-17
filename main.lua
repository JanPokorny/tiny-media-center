---@class Node
---@field name string
---@field path string[]
---@field type "directory" | "video" | "wii_game" | "script"

---@class DirectoryNode : Node
---@field type "directory"
---@field children table<string, Node>

---@class VideoNode : Node
---@field type "video"
---@field meta table<string, any>

---@class WiiGameNode : Node
---@field type "wii_game"

---@class ScriptNode : Node
---@field type "script"

local tinytoml = require("vendor.tinytoml")
local config = require("config")
local BackgroundComponent = require("components.background")
local MenuItemComponent = require("components.menu_item")
local MenuComponent = require("components.menu")
local LoadingScreen = require("components.loading_screen")

local SAVE_DIR = love.filesystem.getSaveDirectory()

local state = { path = {} }
---@type DirectoryNode
local mediaTree = { name = "", path = {}, type = "directory", children = {} }
local currentMenu
local loadingScreen = nil
local activeThread = nil
local threadCallback = nil
local threadStreaming = false

local function stripExtension(filename)
  return filename:match("(.+)%.[^.]+$") or filename
end

---@param node VideoNode
local function saveMetadata(node)
  local mediaPath = table.concat(node.path, "/")
  local tmcPath = stripExtension(mediaPath) .. ".tmc"
  local f = io.open(config.media_path .. "/" .. tmcPath, "w")
  if f then
    f:write(tinytoml.encode(node.meta))
    f:close()
  end
end

---@param path string[]
---@return Node
local function getNode(path)
  ---@type Node
  local node = mediaTree
  for _, seg in ipairs(path) do
    if node.type ~= "directory" then return nil end
    node = node --[[@as DirectoryNode]].children[seg]
    if not node then return nil end
  end
  return node
end

---@param path string[]
---@return Node, string
local function getVideoContext(path)
  for i, seg in ipairs(path) do
    if seg:sub(1, 1) == ":" then return getNode({ unpack(path, 1, i - 1) }), seg end
    local node = getNode({ unpack(path, 1, i) })
    if node and node.type == "video" then return node, path[i + 1] end
  end
end

---@param node Node
local function watchPct(node)
  if node.type ~= "video" or not node.meta.duration then return 0 end
  local videoNode = node --[[@as VideoNode]]
  local pct = math.floor((tonumber(videoNode.meta.position or 0) / tonumber(videoNode.meta.duration)) * 100 + 0.5)
  return pct >= 90 and 100 or pct
end

---@param text string
---@param code string
---@param callback fun(result: string)
---@param streaming? boolean
local function runBackground(text, code, callback, streaming)
  loadingScreen = LoadingScreen:new(text)
  love.thread.getChannel("result"):clear()
  local thread = love.thread.newThread(code)
  activeThread = thread
  threadCallback = callback
  threadStreaming = streaming or false
  love.window.setVSync(0)
  love.timer.sleep(0)
  thread:start()
end

local function stopBackground()
  loadingScreen = nil
  activeThread = nil
  threadCallback = nil
  threadStreaming = false
  love.window.setVSync(1)
end

local getAudioSubMenuItems
local getVideoMenuItems
local getDirectoryMenuItems

getAudioSubMenuItems = function(video, menu)
  local raw_items = menu == ":sub" and { { label = "none", target = "none", action = "select_sub", trackId = "" } } or {}
  local videoNode = video --[[@as VideoNode]]
  for key, value in pairs(videoNode.meta) do
    local trackType, trackId = key:match("^track_([^_]+)_(.*)$")
    if trackType and ":" .. trackType == menu then
      table.insert(raw_items, { label = value, target = value, action = "select_" .. trackType, trackId = trackId })
    end
  end
  table.sort(raw_items, function(a, b)
    return a.trackId == "" or (b.trackId ~= "" and tonumber(a.trackId) < tonumber(b.trackId))
  end)

  local items = {}
  for _, raw_item in ipairs(raw_items) do
    table.insert(items, MenuItemComponent:new({
      label = raw_item.label,
      target = raw_item.target,
      select = function()
        local parentMenu = currentMenu
        local oldPath = state.path

        local videoFromCtx = getVideoContext(oldPath) --[[@as VideoNode]]
        local key = raw_item.action == "select_audio" and "aid" or "sid"
        videoFromCtx.meta[key] = raw_item.trackId
        saveMetadata(videoFromCtx)

        if parentMenu.navigateOut then
          parentMenu.navigateOut()
        end

        local newIndex = raw_item.action == "select_audio" and 2 or 3
        currentMenu:setItems(getVideoMenuItems(video), newIndex)
      end
    }))
  end
  return items
end

getVideoMenuItems = function(video)
  local videoNode = video --[[@as VideoNode]]
  local meta, pct = videoNode.meta, watchPct(videoNode)
  local playLabel = "Play"
  local audioLabel = meta.aid and meta["track_audio_" .. meta.aid] and "Audio [" .. meta["track_audio_" .. meta.aid] .. "]" or
      "Audio"
  local subLabel = meta.sid == "" and "Subtitles [none]" or
      (meta.sid and meta["track_sub_" .. meta.sid] and "Subtitles [" .. meta["track_sub_" .. meta.sid] .. "]" or "Subtitles")

  return {
    MenuItemComponent:new({
      label = playLabel,
      target = "play",
      node = video,
      select = function()
        local node = video --[[@as VideoNode]]
        local args = { '--fullscreen', '--msg-level=all=no', '--save-position-on-quit=no',
          '--input-default-bindings=no', '--osd-on-seek=msg-bar', '--sub-font-provider=none',
          '--sub-font-size=60', '--osd-font-provider=none', '--osd-font-size=60',
          string.format('--config-dir="%s/mpv"', SAVE_DIR),
          string.format('--script="%s/mpv/runtime.lua"', SAVE_DIR),
          string.format('--script="%s/mpv/visualiser.lua"', SAVE_DIR) }

        if node.meta.position and tonumber(node.meta.position) < tonumber(node.meta.duration) - 3 then
          table.insert(args, "--start=" .. node.meta.position)
        end
        if node.meta.aid then table.insert(args, "--aid=" .. node.meta.aid) end
        if node.meta.sid and #node.meta.sid > 0 then table.insert(args, "--sid=" .. node.meta.sid) end

        local cmd = string.format('mpv %s "%s"', table.concat(args, " "),
          config.media_path .. "/" .. table.concat(node.path, "/"))

        runBackground("Playing...", string.format([[
          local ch = love.thread.getChannel("result")
          local h = io.popen(%q)
          local output = h:read("*a")
          h:close()
          ch:push(output)
        ]], cmd), function(result)
          local updatedData = tinytoml.parse(result, { load_from_string = true })
          for k, v in pairs(updatedData) do node.meta[k] = v end
          saveMetadata(node)
          currentMenu:setItems(getVideoMenuItems(video), 1)
        end)
      end
    }),
    MenuItemComponent:new({
      label = audioLabel,
      target = ":audio",
      node = video,
      select = function()
        local parentMenu = currentMenu
        local oldPath = state.path
        state.path = { unpack(oldPath) }
        table.insert(state.path, ":audio")
        currentMenu = MenuComponent:new({ items = getAudioSubMenuItems(video, ":audio") })
        currentMenu:resetScroll()
        currentMenu.navigateOut = function()
          currentMenu = parentMenu
          state.path = oldPath
          currentMenu:resetScroll()
        end
      end
    }),
    MenuItemComponent:new({
      label = subLabel,
      target = ":sub",
      node = video,
      select = function()
        local parentMenu = currentMenu
        local oldPath = state.path
        state.path = { unpack(oldPath) }
        table.insert(state.path, ":sub")
        currentMenu = MenuComponent:new({ items = getAudioSubMenuItems(video, ":sub") })
        currentMenu:resetScroll()
        currentMenu.navigateOut = function()
          currentMenu = parentMenu
          state.path = oldPath
          currentMenu:resetScroll()
        end
      end
    })
  }
end

getDirectoryMenuItems = function(path)
  local dir = getNode(path)
  if not dir or dir.type ~= "directory" then return {} end

  local raw_items = {}
  for name, child in pairs(dir --[[@as DirectoryNode]].children) do
    local displayName = child.type == "directory" and name or stripExtension(name)
    local item = { label = displayName, target = name, node = child }
    if child.type == "video" then
      local pct = watchPct(child)
      if pct >= 100 then
        item.dim = true
      elseif pct > 0 then
        item.label = "* " .. displayName
      end
    elseif child.type == "wii_game" then
      item.action = "play_wii_game"
    elseif child.type == "script" then
      item.action = "run_script"
    end
    table.insert(raw_items, item)
  end

  table.sort(raw_items, function(a, b)
    local aPct, bPct = watchPct(a.node), watchPct(b.node)
    local aCat = aPct >= 1 and aPct <= 89 and 1 or (aPct == 0 and 2 or 3)
    local bCat = bPct >= 1 and bPct <= 89 and 1 or (bPct == 0 and 2 or 3)
    if aCat ~= bCat then return aCat < bCat end
    return a.label < b.label
  end)

  local items = {}
  for _, raw_item in ipairs(raw_items) do
    local action = raw_item.action or "browse"
    if action == "play_wii_game" or action == "run_script" then
      raw_item.select = function()
        local cmd = action == "play_wii_game" and 'dolphin-emu --batch --exec="%s"' or 'bash "%s"'
        local fullCmd = string.format(cmd, config.media_path .. "/" .. table.concat(path, "/") .. "/" .. raw_item.target)
        runBackground(action == "play_wii_game" and "Playing..." or "Running...", string.format([[
          local ch = love.thread.getChannel("result")
          os.execute(%q)
          ch:push("")
        ]], fullCmd), function() end)
      end
    else -- browse
      raw_item.select = function()
        local parentMenu = currentMenu
        local oldPath = state.path
        state.path = { unpack(oldPath) }
        table.insert(state.path, raw_item.target)

        local function showMenu(newItems)
          currentMenu = MenuComponent:new({ items = newItems, selectedIndex = 1 })
          currentMenu:resetScroll()
          currentMenu.navigateOut = function()
            currentMenu = parentMenu
            state.path = oldPath
            currentMenu:resetScroll()
          end
        end

        if raw_item.node.type == "video" then
          showMenu(getVideoMenuItems(raw_item.node))
        else -- directory
          showMenu(getDirectoryMenuItems(state.path))
        end
      end
    end
    table.insert(items, MenuItemComponent:new(raw_item))
  end
  return items
end

local backgroundComponent = BackgroundComponent:new()

local function buildMediaTree(callback)
  runBackground("Processing new media...", string.format([[
    local ch = love.thread.getChannel("result")
    local tinytoml = require("vendor.tinytoml")
    local mediaPath = %q
    local saveDir = %q

    local function stripExtension(filename)
      return filename:match("(.+)%%.[^.]+$") or filename
    end

    local status = love.thread.getChannel("status")

    local children = {}
    local typeByExt = { mp4 = "video", mkv = "video", avi = "video", mp3 = "video", rvz = "wii_game", sh = "script" }

    local h = io.popen('cd "' .. mediaPath .. '" && find . -type f 2>/dev/null')
    for line in h:lines() do
      local rel = line:match("^%%./(.*)")
      if rel then
        local parts = {}
        for p in rel:gmatch("[^/]+") do
          if p:sub(1, 1) == "." then
            parts = nil; break
          end
          parts[#parts + 1] = p
        end
        if parts then
          local nodeType = typeByExt[parts[#parts]:match("%%.([^%%.]+)$")]
          if nodeType then
            local curPath = {}
            for i = 1, #parts - 1 do
              curPath[#curPath + 1] = parts[i]
            end
            local name = parts[#parts]
            curPath[#curPath + 1] = name
            local relPath = table.concat(curPath, "/")
            local entry = nodeType .. "\t" .. relPath

            if nodeType == "video" then
              local tmcPath = stripExtension(relPath) .. ".tmc"
              local f = io.open(mediaPath .. "/" .. tmcPath, "r")
              local meta = ""
              if f then
                meta = f:read("*a")
                f:close()
              end
              local parsed = meta ~= "" and tinytoml.parse(meta, { load_from_string = true }) or {}
              if not parsed.duration then
                local fullPath = mediaPath .. "/" .. relPath
                status:push(relPath)
                local ph = io.popen(string.format('mpv --script="%%s/mpv/preflight.lua" --msg-level=all=no "%%s" 2>/dev/null', saveDir, fullPath))
                local extracted = ph:read("*a")
                ph:close()
                if extracted and #extracted > 0 then
                  meta = extracted
                  local fw = io.open(mediaPath .. "/" .. tmcPath, "w")
                  if fw then fw:write(meta); fw:close() end
                end
                os.execute('subliminal download -l en -HI -FO "' .. fullPath .. '"')
              end
              entry = entry .. "\t" .. meta
            end

            ch:push(entry)
          end
        end
      end
    end
    h:close()
    ch:push("__DONE__")
  ]], config.media_path, SAVE_DIR), callback, true)
end

local function processScanResults()
  local ch = love.thread.getChannel("result")
  local children = {}

  while true do
    local entry = ch:pop()
    if not entry or entry == "__DONE__" then break end

    local nodeType, relPath, meta = entry:match("^([^\t]+)\t([^\t]+)\t?(.*)")
    if nodeType and relPath then
      local parts = {}
      for p in relPath:gmatch("[^/]+") do parts[#parts + 1] = p end

      local cur = children
      local curPath = {}
      for i = 1, #parts - 1 do
        curPath[#curPath + 1] = parts[i]
        cur[parts[i]] = cur[parts[i]] or {
          name = parts[i], path = { unpack(curPath) }, type = "directory", children = {}
        }
        cur = cur[parts[i]].children
      end

      local name = parts[#parts]
      curPath[#curPath + 1] = name
      local node = { name = name, path = { unpack(curPath) }, type = nodeType }

      if nodeType == "video" then
        node.meta = (meta and #meta > 0) and tinytoml.parse(meta, { load_from_string = true }) or {}
      end

      cur[name] = node
    end
  end

  mediaTree.children = children
  stopBackground()
  currentMenu = MenuComponent:new({
    items = getDirectoryMenuItems(state.path)
  })
  currentMenu:resetScroll()
end

function love.load()
  love.filesystem.createDirectory("mpv")
  for _, f in ipairs({ "preflight.lua", "runtime.lua", "visualiser.lua", "input.conf", "subfont.ttf" }) do
    love.filesystem.write("mpv/" .. f, love.filesystem.read("attachments/mpv/" .. f))
  end

  love.graphics.setFont(love.graphics.newFont("attachments/mpv/subfont.ttf", config.style.font_size))
  love.graphics.setBackgroundColor(config.style.background_color)
  love.mouse.setVisible(false)
  love.math.setRandomSeed(os.time())
  currentMenu = MenuComponent:new({})

  buildMediaTree(function() end)
end

function love.update(dt)
  if loadingScreen then
    love.timer.sleep(1)

    local status = love.thread.getChannel("status")
    local latest = nil
    repeat
      local msg = status:pop()
      if msg then latest = msg end
    until not msg
    if latest then loadingScreen.subtext = latest end

    if activeThread then
      local err = activeThread:getError()
      if err then
        stopBackground()
      elseif not activeThread:isRunning() then
        if threadStreaming then
          processScanResults()
        else
          local ch = love.thread.getChannel("result")
          local result = ch:pop() or ""
          local cb = threadCallback
          stopBackground()
          if cb then cb(result) end
        end
      end
    end
    return
  end

  backgroundComponent:update(dt)
  currentMenu:update(dt)
end

function love.draw()
  if not loadingScreen then
    backgroundComponent:draw()
  end
  local w, h = love.graphics.getDimensions()

  if loadingScreen then
    loadingScreen:draw()
  else

  currentMenu:draw(0, 0, w, h)

  local title = state.path[#state.path] or "tiny media center"
  if title:sub(1, 1) == ":" then title = title:sub(2) end
  love.graphics.setColor(config.style.dim_color)
  love.graphics.print(stripExtension(title), 0, 0)

  local videoNode = getVideoContext(state.path)
  if videoNode and videoNode.type == "video" then
    local meta = videoNode --[[@as VideoNode]].meta
    local pct = watchPct(videoNode)
    local barHeight = 4

    love.graphics.setColor(config.style.text_color[1], config.style.text_color[2], config.style.text_color[3], 0.2)
    love.graphics.rectangle("fill", 0, h - barHeight, w, barHeight)
    if pct > 0 then
      love.graphics.setColor(config.style.accent_color)
      love.graphics.rectangle("fill", 0, h - barHeight, w * pct / 100, barHeight)
    end

    if meta.duration then
      local remaining = tonumber(meta.duration) - tonumber(meta.position or 0)
      local now = os.date("%H:%M")
      local endTime = os.date("%H:%M", os.time() + remaining)
      local mins = math.floor(remaining / 60)
      local hours = math.floor(mins / 60)
      mins = mins % 60
      local remStr = hours > 0 and string.format("%dh%02dmin", hours, mins) or string.format("%dmin", mins)
      local timeText = string.format("%s  --  %s  ->  %s", now, remStr, endTime)

      local font = love.graphics.getFont()
      local textW = font:getWidth(timeText)
      love.graphics.setColor(config.style.dim_color)
      love.graphics.print(timeText, (w - textW) / 2, h - barHeight - config.style.font_size * 1.2)
    end
  end
  end
end

function love.keypressed(key)
  if loadingScreen then return end
  if key == "up" then
    currentMenu:navigateUp()
  elseif key == "down" then
    currentMenu:navigateDown()
  elseif key == "return" then
    currentMenu:navigateIn()
  elseif key == "escape" or key == "appback" or key == "sleep" then
    if currentMenu.navigateOut then currentMenu.navigateOut() end
  elseif key == "r" then
    love.event.quit("restart")
  elseif #key == 1 then
    currentMenu:jumpToLetter(key)
  end
end

function love.mousepressed(x, y, button)
  if loadingScreen then return end
  if button == 1 then
    currentMenu:navigateIn()
  elseif button == 2 then
    if currentMenu.navigateOut then currentMenu.navigateOut() end
  end
end

local scrollBuffer = 0
function love.wheelmoved(x, y)
  if loadingScreen then return end
  if (y > 0) ~= (scrollBuffer > 0) then scrollBuffer = 0 end
  scrollBuffer = scrollBuffer + y
  while scrollBuffer > 30 do
    currentMenu:navigateUp()
    scrollBuffer = scrollBuffer - 30
  end
  while scrollBuffer < -30 do
    currentMenu:navigateDown()
    scrollBuffer = scrollBuffer + 30
  end
end

function love.gamepadpressed(joystick, button)
  if loadingScreen then return end
  if button == "dpup" then
    currentMenu:navigateUp()
  elseif button == "dpdown" then
    currentMenu:navigateDown()
  elseif button == "a" then
    currentMenu:navigateIn()
  elseif button == "b" then
    if currentMenu.navigateOut then currentMenu.navigateOut() end
  end
end
