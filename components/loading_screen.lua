local config = require("config")

local SPINNER_CHARS = { "/", "-", "\\", "|" }
local SPINNER_SPEED = 8

---@class (exact) LoadingScreen
---@field private __index self
---@field text string
---@field private _time number
local LoadingScreen = {}

---@param text string
---@return LoadingScreen
function LoadingScreen:new(text)
  local o = { text = text, _time = 0 }
  setmetatable(o, self)
  self.__index = self
  return o
end

---@param dt number
function LoadingScreen:update(dt)
  self._time = self._time + dt
end

function LoadingScreen:draw()
  local font = love.graphics.getFont()
  local w, h = love.graphics.getDimensions()

  local textW = font:getWidth(self.text)
  love.graphics.setColor(config.style.text_color)
  love.graphics.print(self.text, (w - textW) / 2, h / 2 - config.style.font_size * 0.8)

  local frame = math.floor(self._time * SPINNER_SPEED) % #SPINNER_CHARS + 1
  local spinner = "[" .. SPINNER_CHARS[frame] .. "]"
  local spinnerW = font:getWidth(spinner)
  love.graphics.setColor(config.style.dim_color)
  love.graphics.print(spinner, (w - spinnerW) / 2, h / 2 + config.style.font_size * 0.3)
end

return LoadingScreen
