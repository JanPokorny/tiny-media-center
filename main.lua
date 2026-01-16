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

local Conf = require("utils.conf")
local Style = require("style")
local BackgroundComponent = require("components.background")
local MenuItemComponent = require("components.menu_item")
local MenuComponent = require("components.menu")

local MEDIA_ROOT = os.getenv("TMC_MEDIA_PATH") or "./media"
local SAVE_DIR = love.filesystem.getSaveDirectory()
local METADATA_FILE = "metadata/media.conf"

local state = { path = {} }
---@type DirectoryNode
local mediaTree = { name = "", path = {}, type = "directory", children = {} }
local currentMenu

local function stripExtension(filename)
  return filename:match("(.+)%.[^.]+$") or filename
end

---@param videoPath string[]
---@param data table<string, any>
local function appendMetadata(videoPath, data)
  love.filesystem.append(METADATA_FILE, "\n" .. Conf.stringify({ [table.concat(videoPath, "/")] = data }))
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
        appendMetadata(videoFromCtx.path, { [key] = raw_item.trackId })

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
  local playLabel = "Play" .. (pct > 0 and " [" .. pct .. "%" ..
      (meta.duration and ", ends at " .. os.date("%H:%M", os.time() + tonumber(meta.duration) - tonumber(meta.position or 0)) or "") ..
      "]" or "")
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

        local h = io.popen(string.format('mpv %s "%s"', table.concat(args, " "),
          MEDIA_ROOT .. "/" .. table.concat(node.path, "/")))
        local updatedData = Conf.parse(h:read("*a"))
        h:close()
        for k, v in pairs(updatedData) do node.meta[k] = v end
        appendMetadata(node.path, updatedData)
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
    local displayName = stripExtension(name)
    local item = { label = displayName, target = name, node = child }
    if child.type == "video" then
      local pct = watchPct(child)
      if pct > 0 then item.label = displayName .. " [" .. pct .. "%]" end
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
    return aCat ~= bCat and aCat < bCat or a.label < b.label
  end)

  local items = {}
  for _, raw_item in ipairs(raw_items) do
    local action = raw_item.action or "browse"
    if action == "play_wii_game" or action == "run_script" then
      raw_item.select = function()
        local cmd = action == "play_wii_game" and 'dolphin-emu --batch --exec="%s"' or 'bash "%s"'
        io.popen(string.format(cmd, MEDIA_ROOT .. "/" .. table.concat(path, "/") .. "/" .. raw_item.target))
      end
    else -- browse
      raw_item.select = function()
        local parentMenu = currentMenu
        local oldPath = state.path
        state.path = { unpack(oldPath) }
        table.insert(state.path, raw_item.target)

        local newItems
        if raw_item.node.type == "video" then
          newItems = getVideoMenuItems(raw_item.node)
        else -- directory
          newItems = getDirectoryMenuItems(state.path)
        end
        currentMenu = MenuComponent:new({ items = newItems, selectedIndex = 1 })
        currentMenu:resetScroll()
        currentMenu.navigateOut = function()
          currentMenu = parentMenu
          state.path = oldPath
          currentMenu:resetScroll()
        end
      end
    end
    table.insert(items, MenuItemComponent:new(raw_item))
  end
  return items
end

local backgroundComponent = BackgroundComponent:new()

function love.load()
  love.filesystem.createDirectory("metadata")
  love.filesystem.createDirectory("mpv")
  for _, f in ipairs({ "preflight.lua", "runtime.lua", "visualiser.lua", "input.conf", "subfont.ttf" }) do
    love.filesystem.write("mpv/" .. f, love.filesystem.read("attachments/mpv/" .. f))
  end

  local meta = Conf.parse(love.filesystem.read(METADATA_FILE)) or {}
  local children = {}
  local typeByExt = { mp4 = "video", mkv = "video", avi = "video", mp3 = "video", rvz = "wii_game", sh = "script" }

  local h = io.popen('cd "' .. MEDIA_ROOT .. '" && find . -type f 2>/dev/null')
  for line in h:lines() do
    local rel = line:match("^%./(.+)")
    if rel then
      local parts = {}
      for p in rel:gmatch("[^/]+") do
        if p:sub(1, 1) == "." then
          parts = nil; break
        end
        parts[#parts + 1] = p
      end
      if parts then
        local nodeType = typeByExt[parts[#parts]:match("%.([^%.]+)$")]
        if nodeType then
          local cur, curPath = children, {}
          for i = 1, #parts - 1 do
            curPath[#curPath + 1] = parts[i]
            cur[parts[i]] = cur[parts[i]] or {
              name = parts[i], path = { unpack(curPath) }, type = "directory", children = {}
            }
            cur = cur[parts[i]].children
          end

          local name = parts[#parts]
          curPath[#curPath + 1] = name
          local key = table.concat(curPath, "/")
          local node = { name = name, path = curPath, type = nodeType }

          if nodeType == "video" then
            node.meta = meta[key] or {}
            if not node.meta.duration then
              local cmd = string.format('mpv --script="%s/mpv/preflight.lua" --msg-level=all=no "%s" 2>/dev/null',
                SAVE_DIR, MEDIA_ROOT .. "/" .. key)
              local ph = io.popen(cmd)
              local extracted = Conf.parse(ph:read("*a"))
              ph:close()
              for k, v in pairs(extracted) do node.meta[k] = v end
            end
            meta[key] = node.meta
          end
          cur[name] = node
        end
      end
    end
  end
  h:close()

  mediaTree.children = children
  love.filesystem.write(METADATA_FILE, Conf.stringify(meta))
  love.graphics.setFont(love.graphics.newFont("attachments/mpv/subfont.ttf", Style.FONT_SIZE))
  love.graphics.setBackgroundColor(Style.BG_COLOR)
  love.mouse.setVisible(false)
  love.math.setRandomSeed(os.time())
  currentMenu = MenuComponent:new({
    items = getDirectoryMenuItems(state.path)
  })
  currentMenu:resetScroll()
end

function love.update(dt)
  currentMenu:update(dt)
  backgroundComponent:update(dt)
end

function love.draw()
  backgroundComponent:draw()
  local w, h = love.graphics.getDimensions()
  currentMenu:draw(0, 0, w, h)

  local title = state.path[#state.path] or "tiny media center"
  if title:sub(1, 1) == ":" then title = title:sub(2) end
  love.graphics.setColor(Style.DIM_COLOR)
  love.graphics.print(stripExtension(title), 0, 0)
end

function love.keypressed(key)
  if key == "up" then
    currentMenu:navigateUp()
  elseif key == "down" then
    currentMenu:navigateDown()
  elseif key == "return" then
    currentMenu:navigateIn()
  elseif key == "escape" or key == "appback" or key == "sleep" then
    if currentMenu.navigateOut then currentMenu.navigateOut() end
  elseif #key == 1 then
    currentMenu:jumpToLetter(key)
  end
end

function love.mousepressed(x, y, button)
  if button == 1 then
    currentMenu:navigateIn()
  elseif button == 2 then
    if currentMenu.navigateOut then currentMenu.navigateOut() end
  end
end

local scrollBuffer = 0
function love.wheelmoved(x, y)
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
