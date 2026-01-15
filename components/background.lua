---@class (exact) Star
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field size number

---@class (exact) BackgroundComponentInit
---@field speed integer

---@class (exact) BackgroundComponent : BackgroundComponentInit
---@field private __index self
---@field private _stars Star[]
local BackgroundComponent = {}

---@param o BackgroundComponentInit?
---@return BackgroundComponent
function BackgroundComponent:new(o)
  o = o or {} --[[@type BackgroundComponent]]
  setmetatable(o, self)
  self.__index = self
  local w, h = love.graphics.getDimensions()
  o.speed = o.speed or h / 70
  o._stars = {}
  for _ = 1, w * h / 10000 do
    table.insert(o._stars, {
      x = love.math.random(0, w),
      y = love.math.random(0, h),
      vx = 2 * love.math.random() - 1,
      vy = 2 * love.math.random() - 1,
      size = love.math.random(1, 3)
    })
  end
  return o
end

---@param dt number
function BackgroundComponent:update(dt)
  local w, h = love.graphics.getDimensions()
  for _, star in ipairs(self._stars) do
    star.x = (50 + star.x + star.vx * dt * self.speed) % (w + 100) - 50
    star.y = (50 + star.y + star.vy * dt * self.speed) % (h + 100) - 50
  end
end

function BackgroundComponent:draw()
  love.graphics.setBlendMode("add")
  local connectionDist = love.graphics.getHeight() / 8
  for i, s1 in ipairs(self._stars) do
    love.graphics.setColor(0.5, 0.6, 0.7, 0.4)
    love.graphics.circle("fill", s1.x, s1.y, s1.size)
    for j = i + 1, #self._stars do
      local s2 = self._stars[j]
      local dx, dy = s1.x - s2.x, s1.y - s2.y
      local distSq = dx * dx + dy * dy
      if distSq < connectionDist^2 then
        love.graphics.setLineWidth(1)
        love.graphics.setColor(0.2, 0.5, 0.6, (1 - distSq / connectionDist^2) * 0.2)
        love.graphics.line(s1.x, s1.y, s2.x, s2.y)
      end
    end
  end
  love.graphics.setBlendMode("alpha")
end

return BackgroundComponent
