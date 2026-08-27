# FOUNDRY Plan

A native production-chain planner for **FOUNDRY**, built with Rust and `egui`.

The app reads the installed game's YAML templates at startup. In addition to
ordinary recipes and blast-furnace modes, it imports direct fluid processing,
resource extraction, nuclear power loops, and assembly-line production. No
recipe database is copied into this repository.

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
- Select a node to change its building or settings. Ordinary machines expose
  clock speed; blast furnaces expose towers and operating temperature; resource
  converters expose modules and adjacency; endless miners expose power cores;
  reactors expose utilization; and assembly lines can include or omit paint.
  Enter either a machine count or the primary input/output rate to pin the node;
  editing either value updates the other. Use `Unpin` to let connected pinned
  nodes calculate it again.
- New nodes start unpinned. A connected component shows production values only
  after at least one node is pinned. Balanced links are cyan, partial links are
  orange, and unresolved links are gray.
- Unconnected inputs and outputs, total machine count, power consumed, power
  generated, and net power are summarized on the right. Use `Fit plan` to frame
  the entire graph.
- Select a node and press Delete/Backspace, or use the `×` on the node, to remove it.

## Verify

```bash
cargo test
```

Path-resolution tests run on every machine. When FOUNDRY templates are available
through automatic discovery or `FOUNDRY_TEMPLATE_ROOT`, the tests also parse the
installed data and exercise ordinary production accounting, specialized direct
processes, the Blast Furnace → Molten Xenoferrite → Xenoferrite Plates chain,
and a balanced Reactor → Steam Generator → Turbine → Cooling Tower loop.

## Imported process coverage

- Resource converters and boilers, including air intakes and hot-air stoves;
  submerged liquid intakes that source water into pipes and pipelines.
- Pumpjacks joined with reservoirs; ore-vein miners joined with vein and terrain
  templates, including mining-fluid demand and fracking power overhead.
- Endless miners backed by valid `SpecialWorldObjectTemplate` resource nodes.
- Nuclear reactors, steam generators, turbines, and cooling-tower sinks. Turbine
  flow is derived from element fuel value and residual data.
- Assembly lines assembled from object, action, item, and buildable templates.
  Their YAML does not expose cycle timing, so the known game rate of 32 finished
  products per minute is represented explicitly.

Sky-platform passive generation remains outside the planner's current scope.
