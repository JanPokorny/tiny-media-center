mp.register_event("file-loaded", function()
  io.stdout:write(string.format("duration=%s\n", mp.get_property_number("duration")))

  for _, track in ipairs(mp.get_property_native("track-list")) do
    if track.type == "audio" or track.type == "sub" then
      io.stdout:write(
        string.format("track_%s_%d=%s%s%s%s\n",
        track.type,
        track.id,
        track.lang or "?",
        track.title and string.format(" (%s)", track.title) or "",
        track["audio-channels"] and string.format(" (%dch)", track["audio-channels"]) or "",
        track.codec and string.format(" (%s)", track.codec) or "")
      )
    end
  end

  io.stdout:flush()
  mp.command("quit")
end)
