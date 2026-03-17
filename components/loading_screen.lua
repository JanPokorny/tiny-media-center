local config = require("config")

---@class (exact) LoadingScreen
---@field private __index self
---@field text string
local LoadingScreen = {}

---@param text string
---@return LoadingScreen
function LoadingScreen:new(text)
  local o = { text = text }
  setmetatable(o, self)
  self.__index = self
  return o
end

function LoadingScreen:draw()
  local font = love.graphics.getFont()
  local w, h = love.graphics.getDimensions()
  local textW = font:getWidth(self.text)
  love.graphics.setColor(config.style.text_color)
  love.graphics.print(self.text, (w - textW) / 2, (h - config.style.font_size) / 2)
end

return LoadingScreen
