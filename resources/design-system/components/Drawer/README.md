# Drawer

The selected item's details in the aside. A device and a client use the same shape, so the aside reads the same whatever is open (see DeviceDrawer for what a device's holds).

- `kp-drawer__head`: the title block (`kp-drawer__title`: an optional badge above, the name as an h2, a caption under) and Close at the right. Close returns the aside to the activity feed.
- Then the item's parts, 16px apart, each as a `LabeledValue` or a control row.
- `DrawerSection` (`kp-drawer__section`): a part set off by a rule with its own h3, such as History.
- `Holder` (`kp-holder`, `data-state`): who has the device and what that means, in the colour of its state, with its glyph.
- `KeyValues` (`kp-kv`): facts as labels and values. `Chips` (`kp-chips`, `kp-chip`): short names side by side.
