# hypr-persist

Session persistence daemon for the [Hyprland](https://hyprland.org) Wayland compositor.

`hypr-persist` tracks open windows and, on logout/reboot, restores them: relaunching each
application and placing it back into its exact workspace, monitor, and tree position —
including BSP (dwindle) split structure and master-layout promotion — rather than just
reopening windows wherever the compositor's default placement puts them.

Originally forked from [hyprresume](https://github.com/IraSkyx/hyprresume) by Adrien Lenoir;
substantially reworked since (see `LICENSE` for attribution).

## Status

Early — actively under development.
