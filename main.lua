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

local conf = require("conf_parser")

local MEDIA_ROOT = os.getenv("TMC_MEDIA_PATH") or "./media"
local SAVE_DIR = love.filesystem.getSaveDirectory()
local METADATA_FILE = "metadata/media.conf"

local state = {path = {}, selectedIndex = 1, scrollOffset = 0}
---@type DirectoryNode
local mediaTree = {name = "", path = {}, type = "directory", children = {}}
local UI = {
  bgColor = {0, 0, 0}, textColor = {1, 1, 1},
  accentColor = {1, 0.8, 0}, dimColor = {0.5, 0.5, 0.5},
  fontSize = 72, itemHeight = 81
}

local function stripExtension(filename)
  return filename:match("(.+)%.[^.]+$") or filename
end

---@param videoPath string[]
---@param data table<string, any>
local function appendMetadata(videoPath, data)
  love.filesystem.append(METADATA_FILE, "\n" .. conf.serialize({[table.concat(videoPath, "/")] = data}))
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

---@return Node, string
local function getVideoContext()
  for i, seg in ipairs(state.path) do
    if seg:sub(1, 1) == ":" then return getNode({unpack(state.path, 1, i - 1)}), seg end
    local node = getNode({unpack(state.path, 1, i)})
    if node and node.type == "video" then return node, state.path[i + 1] end
  end
end

---@param node Node
local function watchPct(node)
  if node.type ~= "video" or not node.meta.duration then return 0 end
  local videoNode = node --[[@as VideoNode]]
  local pct = math.floor((tonumber(videoNode.meta.position or 0) / tonumber(videoNode.meta.duration)) * 100 + 0.5)
  return pct >= 90 and 100 or pct
end

local function getMenuItems()
  local video, menu = getVideoContext()
  
  if video and (menu == ":audio" or menu == ":sub") then
    local items = menu == ":sub" and {{label = "none", target = "none", action = "select_sub", trackId = ""}} or {}
    local videoNode = video --[[@as VideoNode]]
    for key, value in pairs(videoNode.meta) do
      local trackType, trackId = key:match("^track_([^_]+)_(.*)$")
      if trackType and ":" .. trackType == menu then
        table.insert(items, {label = value, target = value, action = "select_" .. trackType, trackId = trackId})
      end
    end
    table.sort(items, function(a, b)
      return a.trackId == "" or (b.trackId ~= "" and tonumber(a.trackId) < tonumber(b.trackId))
    end)
    return items
  end
  
  if video and video.type == "video" then
    local videoNode = video --[[@as VideoNode]]
    local meta, pct = videoNode.meta, watchPct(videoNode)
    local playLabel = "Play" .. (pct > 0 and " [" .. pct .. "%" .. 
      (meta.duration and ", ends at " .. os.date("%H:%M", os.time() + tonumber(meta.duration) - tonumber(meta.position or 0)) or "") .. "]" or "")
    local audioLabel = meta.aid and meta["track_audio_" .. meta.aid] and 
      "Audio [" .. meta["track_audio_" .. meta.aid] .. "]" or "Audio"
    local subLabel = meta.sid == "" and "Subtitles [none]" or 
      (meta.sid and meta["track_sub_" .. meta.sid] and "Subtitles [" .. meta["track_sub_" .. meta.sid] .. "]" or "Subtitles")
    return {
      {label = playLabel, target = "play", action = "play", node = video},
      {label = audioLabel, target = ":audio", action = "audio_menu", node = video},
      {label = subLabel, target = ":sub", action = "sub_menu", node = video}
    }
  end
  
  local dir = getNode(state.path)
  if not dir or dir.type ~= "directory" then return {} end
  
  local items = {}
  for name, child in pairs(dir --[[@as DirectoryNode]].children) do
    local displayName = stripExtension(name)
    local item = {label = displayName, target = name, node = child}
    if child.type == "video" then
      local pct = watchPct(child)
      if pct > 0 then item.label = displayName .. " [" .. pct .. "%]" end
    elseif child.type == "wii_game" then
      item.action = "play_wii_game"
    elseif child.type == "script" then
      item.action = "run_script"
    end
    table.insert(items, item)
  end
  
  table.sort(items, function(a, b)
    local aPct, bPct = watchPct(a.node), watchPct(b.node)
    local aCat = aPct >= 1 and aPct <= 89 and 1 or (aPct == 0 and 2 or 3)
    local bCat = bPct >= 1 and bPct <= 89 and 1 or (bPct == 0 and 2 or 3)
    return aCat ~= bCat and aCat < bCat or a.label < b.label
  end)
  return items
end

local function targetOffset()
  local w, h = love.graphics.getDimensions()
  local itemCount, listHeight = #getMenuItems(), #getMenuItems() * UI.itemHeight
  return listHeight <= h * 0.8 and (listHeight - UI.itemHeight) / 2 or (state.selectedIndex - 1) * UI.itemHeight
end

local function resetScroll()
  state.scrollOffset = targetOffset() - UI.itemHeight * 0.5
end

function navigateIn()
  local items, item = getMenuItems(), getMenuItems()[state.selectedIndex]
  if not item then return end
  local action = item.action or "browse"
  
  if action == "play" then
    local node = item.node --[[@as VideoNode]]
    local args = {'--fullscreen', '--msg-level=all=no', '--save-position-on-quit=no', 
      '--input-default-bindings=no', '--osd-on-seek=msg-bar', '--sub-font-provider=none', 
      '--sub-font-size=60', '--osd-font-provider=none', '--osd-font-size=60',
      string.format('--config-dir="%s/mpv"', SAVE_DIR),
      string.format('--script="%s/mpv/runtime.lua"', SAVE_DIR),
      string.format('--script="%s/mpv/visualiser.lua"', SAVE_DIR)}
    
    if node.meta.position and tonumber(node.meta.position) < tonumber(node.meta.duration) - 3 then
      table.insert(args, "--start=" .. node.meta.position)
    end
    if node.meta.aid then table.insert(args, "--aid=" .. node.meta.aid) end
    if node.meta.sid and #node.meta.sid > 0 then table.insert(args, "--sid=" .. node.meta.sid) end
    
    local h = io.popen(string.format('mpv %s "%s"', table.concat(args, " "), MEDIA_ROOT .. "/" .. table.concat(node.path, "/")))
    local updatedData = conf.parse(h:read("*a"))
    h:close()
    for k, v in pairs(updatedData) do node.meta[k] = v end
    appendMetadata(node.path, updatedData)
    
  elseif action == "play_wii_game" or action == "run_script" then
    local cmd = action == "play_wii_game" and 'dolphin-emu --batch --exec="%s"' or 'bash "%s"'
    io.popen(string.format(cmd, MEDIA_ROOT .. "/" .. table.concat(state.path, "/") .. "/" .. item.target))
    
  elseif action == "audio_menu" or action == "sub_menu" then
    table.insert(state.path, item.target)
    state.selectedIndex = 1
    resetScroll()
    
  elseif action == "select_audio" or action == "select_sub" then
    local video = getVideoContext() --[[@as VideoNode]]
    local key = action == "select_audio" and "aid" or "sid"
    video.meta[key] = item.trackId
    appendMetadata(video.path, {[key] = item.trackId})
    table.remove(state.path)
    state.selectedIndex = action == "select_audio" and 2 or 3
    resetScroll()
  else
    table.insert(state.path, item.target)
    state.selectedIndex = 1
    resetScroll()
  end
end

function navigateOut()
  if #state.path < 1 then return end
  local last = table.remove(state.path)
  state.selectedIndex = 1
  for i, item in ipairs(getMenuItems()) do
    if item.target == last then state.selectedIndex = i; break end
  end
  resetScroll()
end

function createLiveBackground(width, height)
  local bg = {width = width, height = height, stars = {}, speed = 15, connectionDist = 120}
  for i = 1, 150 do
    table.insert(bg.stars, {
      x = love.math.random(0, width), y = love.math.random(0, height),
      vx = love.math.random(-100, 100) / 100 * bg.speed,
      vy = love.math.random(-100, 100) / 100 * bg.speed,
      size = love.math.random(1, 3)
    })
  end
  
  function bg:update(dt)
    for _, star in ipairs(self.stars) do
      star.x = (50 + star.x + star.vx * dt) % (bg.width + 100) - 50
      star.y = (50 + star.y + star.vy * dt) % (bg.height + 100) - 50
    end
  end
  
  function bg:draw()
    love.graphics.setBlendMode("add")
    for i, s1 in ipairs(self.stars) do
      love.graphics.setColor(0.5, 0.6, 0.7, 0.4)
      love.graphics.circle("fill", s1.x, s1.y, s1.size)
      for j = i + 1, #self.stars do
        local s2 = self.stars[j]
        local dx, dy = s1.x - s2.x, s1.y - s2.y
        local distSq = dx * dx + dy * dy
        if distSq < self.connectionDist^2 then
          love.graphics.setLineWidth(1)
          love.graphics.setColor(0.2, 0.5, 0.6, (1 - distSq / self.connectionDist^2) * 0.2)
          love.graphics.line(s1.x, s1.y, s2.x, s2.y)
        end
      end
    end
    love.graphics.setBlendMode("alpha")
  end
  return bg
end

local background = createLiveBackground(love.graphics.getWidth(), love.graphics.getHeight())

function love.load()
  love.filesystem.createDirectory("metadata")
  love.filesystem.createDirectory("mpv")
  for _, f in ipairs({"preflight.lua", "runtime.lua", "visualiser.lua", "input.conf", "subfont.ttf"}) do
    love.filesystem.write("mpv/" .. f, love.filesystem.read("attachments/mpv/" .. f))
  end
  
  local meta = conf.parse(love.filesystem.read(METADATA_FILE)) or {}
  local children = {}
  local typeByExt = {mp4 = "video", mkv = "video", avi = "video", mp3 = "video", rvz = "wii_game", sh = "script"}
  
  local h = io.popen('cd "' .. MEDIA_ROOT .. '" && find . -type f 2>/dev/null')
  for line in h:lines() do
    local rel = line:match("^%./(.+)")
    if rel then
      local parts = {}
      for p in rel:gmatch("[^/]+") do
        if p:sub(1, 1) == "." then parts = nil; break end
        parts[#parts + 1] = p
      end
      if parts then
        local nodeType = typeByExt[parts[#parts]:match("%.([^%.]+)$")]
        if nodeType then
          local cur, curPath = children, {}
          for i = 1, #parts - 1 do
            curPath[#curPath + 1] = parts[i]
            cur[parts[i]] = cur[parts[i]] or {
              name = parts[i], path = {unpack(curPath)}, type = "directory", children = {}
            }
            cur = cur[parts[i]].children
          end
          
          local name = parts[#parts]
          curPath[#curPath + 1] = name
          local key = table.concat(curPath, "/")
          local node = {name = name, path = curPath, type = nodeType}
          
          if nodeType == "video" then
            node.meta = meta[key] or {}
            if not node.meta.duration then
              local cmd = string.format('mpv --script="%s/mpv/preflight.lua" --msg-level=all=no "%s" 2>/dev/null', SAVE_DIR, MEDIA_ROOT .. "/" .. key)
              local ph = io.popen(cmd)
              local extracted = conf.parse(ph:read("*a"))
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
  love.filesystem.write(METADATA_FILE, conf.serialize(meta))
  love.graphics.setFont(love.graphics.newFont("attachments/mpv/subfont.ttf", UI.fontSize))
  love.graphics.setBackgroundColor(UI.bgColor)
  love.mouse.setVisible(false)
  love.math.setRandomSeed(os.time())
  resetScroll()
end

function love.update(dt)
  state.scrollOffset = state.scrollOffset + (targetOffset() - state.scrollOffset) * 10 * dt
  background:update(dt)
end

local listFadeShader = love.graphics.newShader[[
  extern number screen_height; extern number header_height; extern number fade_top_size; extern number fade_bot_size;
  vec4 effect(vec4 color, Image texture, vec2 texture_coords, vec2 screen_coords) {
    vec4 pixel = Texel(texture, texture_coords) * color;
    number alpha = screen_coords.y < header_height ? 0.0 : 
      (screen_coords.y < header_height + fade_top_size ? (screen_coords.y - header_height) / fade_top_size : 1.0);
    if (screen_coords.y > screen_height - fade_bot_size) 
      alpha = min(alpha, (screen_height - screen_coords.y) / fade_bot_size);
    pixel.a *= alpha;
    return pixel;
  }
]]

function love.draw()
  background:draw()
  local w, h = love.graphics.getDimensions()
  local headerHeight = UI.itemHeight * 1.2
  local fadeTopSize, fadeBotSize = h * 0.3 - headerHeight, h - h * 0.7
  
  listFadeShader:send("screen_height", h)
  listFadeShader:send("header_height", headerHeight)
  listFadeShader:send("fade_top_size", fadeTopSize)
  listFadeShader:send("fade_bot_size", fadeBotSize)
  love.graphics.setShader(listFadeShader)
  
  for i, item in ipairs(getMenuItems()) do
    local y = h / 2 - state.scrollOffset + (i - 1) * UI.itemHeight
    if y > -UI.itemHeight and y < h then
      love.graphics.setColor(i == state.selectedIndex and UI.accentColor or UI.textColor)
      love.graphics.print((i == state.selectedIndex and "> " or "  ") .. item.label, 50, y)
    end
  end
  love.graphics.setShader()
  
  local title = state.path[#state.path] or "tiny media center"
  if title:sub(1, 1) == ":" then title = title:sub(2) end
  love.graphics.setColor(UI.dimColor)
  love.graphics.print(stripExtension(title), 0, 0)
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
      if items[index].label:lower():sub(1, 1) == key then
        state.selectedIndex = index
        break
      end
    end
  end
end

function love.mousepressed(x, y, button)
  if button == 1 then navigateIn() elseif button == 2 then navigateOut() end
end

local scrollBuffer = 0
function love.wheelmoved(x, y)
  if (y > 0) ~= (scrollBuffer > 0) then scrollBuffer = 0 end
  scrollBuffer = scrollBuffer + y
  while scrollBuffer > 30 do
    state.selectedIndex = math.max(1, state.selectedIndex - 1)
    scrollBuffer = scrollBuffer - 30
  end
  while scrollBuffer < -30 do
    state.selectedIndex = math.min(#getMenuItems(), state.selectedIndex + 1)
    scrollBuffer = scrollBuffer + 30
  end
end

function love.gamepadpressed(joystick, button)
  if button == "dpup" then
    state.selectedIndex = math.max(1, state.selectedIndex - 1)
  elseif button == "dpdown" then
    state.selectedIndex = math.min(#getMenuItems(), state.selectedIndex + 1)
  elseif button == "a" then
    navigateIn()
  elseif button == "b" then
    navigateOut()
  end
end
