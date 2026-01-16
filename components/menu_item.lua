---@class (exact) MenuItemComponentInit
---@field label string
---@field select function
---@field focused? boolean
---@field target string|nil
---@field node table|nil

---@class (exact) MenuItemComponent : MenuItemComponentInit
---@field private __index self
local MenuItemComponent = {}

---@param o MenuItemComponentInit
---@return MenuItemComponent
function MenuItemComponent:new(o)
  o = o or {} --[[@type MenuItemComponent]]
  assert(o.label, "MenuItemComponent requires a label")
  assert(o.select, "MenuItemComponent requires a select function")
  o.focused = o.focused or false
  setmetatable(o, self)
  self.__index = self
  return o
end

---@param fontSize integer
---@param x integer
---@param y integer
---@param w integer
---@param h integer
function MenuItemComponent:draw(fontSize, x, y, w, h)
  love.graphics.print((self.focused and "> " or "  ") .. self.label, x, y)
end

return MenuItemComponent
