# Freya devtools

A companion app for inspecting the running UI in real time: browse the node tree, inspect an
element's style, layout and text style, highlight/hover an element to locate it on screen, and
control animation speed.

## Setup in this repo

Devtools are **opt-in**. Our own `devtools` feature forwards to freya's, so it never ends up in a
default or release build:

```toml
[features]
devtools = ["freya/devtools"]
```

Run the app with it enabled:

```sh
cargo run --features devtools
```

No code changes are needed — `freya::launch` registers `DevtoolsPlugin` itself when the feature is
active (`freya-0.4.3/src/lib.rs:132`). A plain `cargo run` starts no devtools server.

## Running it

The UI is a separate standalone binary, installed once:

```sh
cargo install freya-devtools-app
freya-devtools-app
```

Start it alongside a running `cargo run`. Order does not matter — if the app is not up yet the
devtools app keeps retrying until it connects.

There is no in-app keyboard shortcut to open devtools, and no window of its own inside the app.

## How it works

The plugin runs a WebSocket server on **`[::1]:7354`** — the IPv6 loopback specifically, not
`127.0.0.1` (`freya-devtools-0.4.3/src/server.rs:109`). It prints
`Running the Devtools Server in [::1]:7354` on startup, so that line in the app's output confirms
devtools is live.

Traffic is JSON over that socket:

- app -> devtools: `Update { window_id, nodes: Vec<NodeInfo> }`, a full snapshot of the node tree,
  where each `NodeInfo` carries `node_id`, `parent_id`, `children_len`, `height`, `layer`, the
  computed `NodeState` (style, layout, text) and the resolved `area` / `inner_area`.
- devtools -> app: `HighlightNode { window_id, node_id }`, `HoverNode { window_id, node_id }` and
  `SetSpeedTo { speed }` (the animation-speed slider).

## Limitations

- **Only one** devtools-enabled freya app can run at a time. The port is fixed, so a second instance
  fails to bind it — close the previous one first. Only instances built with `--features devtools`
  take the port, so an ordinary `cargo run` alongside one is fine.
- The devtools app shows what the plugin sends; it cannot edit styles or the tree.

## Related

In debug builds freya also registers `PerformanceOverlayPlugin` on its own, independent of the
devtools feature (`freya-0.4.3/src/lib.rs:134`).
