# FOUNDRY Plan

A native production-chain planner for **FOUNDRY**, built with Rust and `egui`.

The app reads the installed game's YAML templates at startup, including recipes,
blast furnace modes, item names, compatible production buildings, crafting
speeds, and machine power consumption. No recipe database is copied into this
repository.

## Run

```bash
cargo run --release
```

The app automatically checks common Steam installations on Linux, macOS, and
Windows. This includes native, legacy, and Flatpak Steam layouts on Linux.

For a custom Steam library or another installation layout, set the template
directory explicitly:

```bash
FOUNDRY_TEMPLATE_ROOT="/path/to/FOUNDRY/foundry_Data/StreamingAssets/Templates" \
  cargo run --release
```

`FOUNDRY_TEMPLATE_ROOT` takes precedence over automatic discovery. If the app
cannot find the templates, it starts without recipe data and displays an error
explaining how to configure the path.

## Use

- Search the recipe library and press `+` (or double-click a recipe) to add it.
- Drag a node by its header. Middle-drag the workspace to pan and scroll to zoom.
- Drag a red input port onto empty space to choose a recipe that produces it.
- Drag a green output port onto empty space to choose a recipe that consumes it.
- Ports can also be dragged directly onto a matching port on another node.
- Select a node to change its building or clock speed. Blast furnaces instead
  expose their 1–5 tower configuration and operating temperature; their solid,
  elemental, hot-air, slag, and waste-gas rates update from the game template
  values. Enter either a machine count or primary output rate to pin that output;
  editing either value updates the other. Use `Unpin` to let connected pinned
  nodes calculate it again.
- New nodes start unpinned. A connected component shows production values only
  after at least one node is pinned. Balanced links are cyan, partial links are
  orange, and unresolved links are gray.
- Unconnected inputs and outputs, total machine count, and power are summarized on
  the right. Use `Fit plan` to frame the entire graph.
- Select a node and press Delete/Backspace, or use the `×` on the node, to remove it.

## Verify

```bash
cargo test
```

Path-resolution tests run on every machine. When FOUNDRY templates are available
through automatic discovery or `FOUNDRY_TEMPLATE_ROOT`, the tests also parse the
installed data and exercise both ordinary production-chain accounting and the
Blast Furnace → Molten Xenoferrite → Xenoferrite Plates chain.
