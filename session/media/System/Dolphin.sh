# Dolphin's own UI. Through systemd like tmc's Wii launches: avoids a GL-init crash and
# keeps running if tmc restarts.
exec systemd-run --user --wait --collect dolphin-emu
