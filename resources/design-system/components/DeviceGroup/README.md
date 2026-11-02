# DeviceGroup

Devices grouped by what they need, each group a react-aria Disclosure, its cards a react-aria GridList.

- Server groups, in order: Waiting for the server, In use, Shared and free, Not shared or offline. Client groups: Waiting for the server, Attached here, Free to attach, Not available. Empty groups are not shown.
- Cards are `kp-device` GridListItems with `data-state`. Arrow keys move between cards, Enter or a click selects one and opens it in the detail drawer, and the Switch or Button inside a card still works on its own.
- A pending card gets the amber stripe along its top edge; a selected card a canopy edge.
