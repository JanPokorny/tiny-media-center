local Style = require("style")

local listFadeShader = love.graphics.newShader([[
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
]])

---@class MenuItemComponent
---@class (exact) MenuComponentInit
---@field items MenuItemComponent[]
---@field selectedIndex? integer

---@class (exact) MenuComponent : MenuComponentInit
---@field private __index self
---@field scrollOffset number
---@field navigateOut function|nil
local MenuComponent = {}

---@param o MenuComponentInit?
---@return MenuComponent
function MenuComponent:new(o)
  o = o or {}
  ---@cast o MenuComponent
  o.items = o.items or {}
  o.selectedIndex = o.selectedIndex or 1
  o.scrollOffset = 0
  o.navigateOut = o.navigateOut
  setmetatable(o, self)
  self.__index = self
  return o
end

function MenuComponent:getTargetOffset()
  local listHeight = #self.items * Style.ITEM_HEIGHT
  return listHeight <= love.graphics.getHeight() * 0.8 and (listHeight - Style.ITEM_HEIGHT) / 2 or (self.selectedIndex - 1) * Style.ITEM_HEIGHT
end

function MenuComponent:resetScroll()
  local target = self:getTargetOffset()
  self.scrollOffset = target - Style.ITEM_HEIGHT * 0.5
end

function MenuComponent:update(dt)
  self.scrollOffset = self.scrollOffset + (self:getTargetOffset() - self.scrollOffset) * 10 * dt
end

function MenuComponent:draw(x, y, w, h)
  local headerHeight = Style.ITEM_HEIGHT * 1.2
  local fadeTopSize, fadeBotSize = h * 0.3 - headerHeight, h - h * 0.7

  listFadeShader:send("screen_height", h)
  listFadeShader:send("header_height", headerHeight)
  listFadeShader:send("fade_top_size", fadeTopSize)
  listFadeShader:send("fade_bot_size", fadeBotSize)
  love.graphics.setShader(listFadeShader)

  for i, item in ipairs(self.items) do
    local itemY = y + h / 2 - self.scrollOffset + (i - 1) * Style.ITEM_HEIGHT
    if itemY > y - Style.ITEM_HEIGHT and itemY < y + h then
      item.focused = i == self.selectedIndex
      item:draw(x + 50, itemY, w, Style.ITEM_HEIGHT)
    end
  end
  love.graphics.setShader()
end

function MenuComponent:navigateUp()
  if #self.items == 0 then return end
  self.selectedIndex = math.max(1, self.selectedIndex - 1)
end

function MenuComponent:navigateDown()
  if #self.items == 0 then return end
  self.selectedIndex = math.min(#self.items, self.selectedIndex + 1)
end

function MenuComponent:jumpToLetter(key)
  if #self.items == 0 or #key ~= 1 then return end
  for i = 0, #self.items - 1 do
    local index = 1 + (self.selectedIndex + i) % #self.items
    if self.items[index].label:lower():sub(1, 1) == key then
      self.selectedIndex = index
      break
    end
  end
end

function MenuComponent:navigateIn()
  local item = self.items[self.selectedIndex]
  if not item then return end
  item:select()
end

function MenuComponent:setItems(items, selectedIndex)
  self.items = items
  self.selectedIndex = selectedIndex or 1
  self:resetScroll()
end

return MenuComponent
