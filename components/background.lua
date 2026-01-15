---@class Star
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field size number

---@class Background
---@field width integer
---@field height integer
---@field speed integer
---@field numStars integer
---@field connectionDist integer
---@field private _stars Star[]?
local Background = {}

---Creates a new Background instance
---@param o Background?
---@return Background
function Background:new(o)
  o = o or {}
  setmetatable(o, self)
  self.__index = self
  o._stars = {}
  for _ = 1, o.numStars do
    table.insert(o._stars, {
      x = love.math.random(0, o.width),
      y = love.math.random(0, o.height),
      vx = love.math.random(-100, 100) / 100 * o.speed,
      vy = love.math.random(-100, 100) / 100 * o.speed,
      size = love.math.random(1, 3)
    })
  end
  return o
end

---Updates star positions based on delta time
---@param dt number
function Background:update(dt)
  for _, star in ipairs(self._stars) do
    star.x = (50 + star.x + star.vx * dt) % (self.width + 100) - 50
    star.y = (50 + star.y + star.vy * dt) % (self.height + 100) - 50
  end
end

---Draws the starfield with connections
function Background:draw()
  love.graphics.setBlendMode("add")
  for i, s1 in ipairs(self._stars) do
    love.graphics.setColor(0.5, 0.6, 0.7, 0.4)
    love.graphics.circle("fill", s1.x, s1.y, s1.size)
    for j = i + 1, #self._stars do
      local s2 = self._stars[j]
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

return Background
