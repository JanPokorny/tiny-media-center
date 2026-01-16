local Style = require("style")

---@class (exact) MenuItemComponentInit
---@field label string
---@field select function
---@field focused? boolean

---@class (exact) MenuItemComponent : MenuItemComponentInit
---@field private __index self
local MenuItemComponent = {}

---@param o MenuItemComponentInit
---@return MenuItemComponent
function MenuItemComponent:new(o)
  o = o or {}
  ---@cast o MenuItemComponent
  o.focused = o.focused or false
  assert(o.label, "MenuItemComponent requires a label")
  assert(o.select, "MenuItemComponent requires a select function")
  setmetatable(o, self)
  self.__index = self
  return o
end

---@param x integer
---@param y integer
---@param w integer
---@param h integer
function MenuItemComponent:draw(x, y, w, h)
  love.graphics.setColor(self.focused and Style.ACCENT_COLOR or Style.TEXT_COLOR)
  love.graphics.print((self.focused and "> " or "  ") .. self.label, x, y)
end

return MenuItemComponent
