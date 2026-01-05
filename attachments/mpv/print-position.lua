mp.register_event("file-loaded", function()
    io.stdout:write(string.format("duration=%s\n", mp.get_property_number("duration")))
    io.stdout:flush()
end)

mp.add_hook("on_unload", 50, function()
    io.stdout:write(string.format("position=%s\n", mp.get_property_number("time-pos")))
    io.stdout:flush()
end)
