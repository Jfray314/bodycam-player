# Bodycam look for ReShade

Gives any game the Bodycam camera: fisheye-style lens warp, colour fringing, soft blurry edges, dirt on the lens, dark rounded corners, and a brightness "pump" when the picture changes fast.

## Install
1. Install ReShade (reshade.me) on the game you want, if you haven't.
2. Copy `Bodycam.fx` into that game's `reshade-shaders\Shaders` folder.
3. Start the game, open the ReShade overlay (Home key by default), tick **Bodycam** in the list.
4. Use the sliders to taste. Defaults match the Bodycam player.

## Notes
- ReShade only sees the finished picture (0 to 1 colour), not the game's HDR data. So lens dirt lights up on bright spots using a lower threshold (0.8, not 4.0). Raise or lower "Dirt brightness threshold" per game.
- The exposure pump reacts to sudden brightness changes in the picture, since ReShade cannot see how fast you turn the camera.
- Left out: rolling-shutter wobble, flashbang, blood-on-lens and explosion shake (they need game data).
- The edge blur is heavy (24 samples per pixel). If the framerate drops, lower "Edge blur".
- This file was written to follow ReShade FX syntax but has NOT been compiled here (no ReShade compiler available). If ReShade shows a compile error in the overlay, send the message back and it can be fixed.
