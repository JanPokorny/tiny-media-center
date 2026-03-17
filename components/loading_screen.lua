local config = require("config")

---@class (exact) LoadingScreen
---@field private __index self
---@field text string
---@field subtext string
local LoadingScreen = {}

---@param text string
---@return LoadingScreen
function LoadingScreen:new(text)
  local o = { text = text, subtext = "" }
  setmetatable(o, self)
  self.__index = self
  return o
end

function LoadingScreen:draw()
  local font = love.graphics.getFont()
  local w, h = love.graphics.getDimensions()
  local textW = font:getWidth(self.text)
  love.graphics.setColor(config.style.text_color)
  love.graphics.print(self.text, (w - textW) / 2, (h - config.style.font_size) / 2 - config.style.font_size * 0.7)

  if #self.subtext > 0 then
    local subW = font:getWidth(self.subtext) * 0.5
    love.graphics.setColor(config.style.dim_color)
    love.graphics.print(self.subtext, (w - subW) / 2, (h - config.style.font_size) / 2 + config.style.font_size * 0.7, 0, 0.5, 0.5)
  end
end

return LoadingScreen
