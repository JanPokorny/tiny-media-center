local config = require("config")

---@class (exact) MenuItemComponentInit
---@field label string
---@field select function
---@field focused? boolean
---@field dim? boolean

---@class (exact) MenuItemComponent : MenuItemComponentInit
---@field private __index self
---@field private _squishScale number
---@field private _dt number
local MenuItemComponent = {}

---@param o MenuItemComponentInit
---@return MenuItemComponent
function MenuItemComponent:new(o)
  o = o or {}
  ---@cast o MenuItemComponent
  o.focused = o.focused or false
  assert(o.label, "MenuItemComponent requires a label")
  assert(o.select, "MenuItemComponent requires a select function")
  o._squishScale = 1
  o._dt = 0
  setmetatable(o, self)
  self.__index = self
  return o
end

---@param dt number
function MenuItemComponent:update(dt)
  self._dt = dt
end

---@param x integer
---@param y integer
---@param w integer
---@param h integer
function MenuItemComponent:draw(x, y, w, h)
  local font = love.graphics.getFont()
  local prefix = self.focused and "> " or "  "
  local prefixWidth = font:getWidth(prefix)
  local color = self.focused and config.style.accent_color or config.style.text_color
  if self.dim and not self.focused then
    color = { color[1], color[2], color[3], 0.3 }
  end

  love.graphics.setColor(color)
  love.graphics.print(prefix, x, y)

  local textX = x + prefixWidth
  local maxWidth = w - textX
  local textWidth = font:getWidth(self.label)
  local targetScale = (self.focused and textWidth > maxWidth) and maxWidth / textWidth or 1
  self._squishScale = self._squishScale + (targetScale - self._squishScale) * math.min(1, 8 * self._dt)
  self._dt = 0

  love.graphics.setColor(color)
  love.graphics.print(self.label, textX, y, 0, self._squishScale, 1)
end

return MenuItemComponent
