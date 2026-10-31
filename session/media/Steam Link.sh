# Wake the gaming PC first; Steam Link waits for it to appear on the network.
wol 2c:f0:5d:83:a7:93 || true
exec flatpak run com.valvesoftware.SteamLink
