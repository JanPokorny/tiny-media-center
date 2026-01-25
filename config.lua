local tinytoml = require("vendor.tinytoml")

local success, c = pcall(tinytoml.parse, (os.getenv("XDG_CONFIG_HOME") or os.getenv("HOME") .. "/.config") .. "/tiny-media-center/config.toml")
if not success then c = {} end
c.style = c.style or {}

return {
  media_path = c.media_path or ".",
  style = {
    background_color = c.style.background_color or { 0, 0, 0 },
    text_color = c.style.text_color or { 1, 1, 1 },
    accent_color = c.style.accent_color or { 1, 0.8, 0 },
    dim_color = c.style.dim_color or { 0.5, 0.5, 0.5 },
    font_size = c.style.font_size or 72,
  }
}