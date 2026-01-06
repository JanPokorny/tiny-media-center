mp.register_event("file-loaded", function()
  io.stdout:write(string.format("duration=%s\n", mp.get_property_number("duration")))
  local tracks = mp.get_property_native("track-list")
  for _, track in ipairs(tracks) do
    if track.type == "audio" or track.type == "sub" then
      io.stdout:write(string.format("[%s:%d]\n", track.type:sub(1, 1), track.id))
      io.stdout:write(string.format("id=%d\n", track.id))
      io.stdout:write(string.format("type=%s\n", track.type))
      io.stdout:write(string.format("selected=%s\n", track.selected and "yes" or "no"))

      if track.lang then
          io.stdout:write(string.format("lang=%s\n", track.lang))
      end

      if track.title then
          io.stdout:write(string.format("title=%s\n", track.title))
      end

      if track["audio-channels"] then
          io.stdout:write(string.format("channels=%d\n", track["audio-channels"]))
      end

      if track.codec then
          io.stdout:write(string.format("codec=%s\n", track.codec))
      end

      io.stdout:write("\n")
    end
  end

  io.stdout:flush()
  mp.command("quit")
end)
