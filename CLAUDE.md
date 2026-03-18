# Rust + GTK4 + Libadwaita Project Rules

## State Management
- Use `glib::subclass` for all custom widgets. Logic lives in `imp.rs`, public API in `mod.rs`.
- Widget state belongs in the `imp` struct as `RefCell<T>` or `std::cell::OnceCell<T>`.
- Avoid `Rc<RefCell<T>>` for state shared across modules; prefer GObject properties and signals.
- `Rc<RefCell<Config>>` is permitted within a single UI component (e.g. preferences dialog) where multiple closures need shared mutable access to config within one call stack.

## Memory Safety / Closures
- Use `glib::clone!(@weak self => ...)` (or `@weak widget`) for **every** signal handler closure that captures a GTK object. This prevents reference cycles.
- Use `@strong value` for non-GObject types that need to be cloned into a closure (e.g. `async_channel::Sender`).
- Never capture a GObject with a plain `move` closure — always use `glib::clone!`.
- For closures that must return a value on upgrade failure, use `@default-return value`.

## UI Construction
- Define window/widget layout in Composite Templates (`.ui` XML files) embedded in GResources.
- Register template children with `#[template_child]` in the `imp` struct.
- Use `klass.bind_template()` in `class_init` and `obj.init_template()` in `instance_init`.
- Signal-to-callback wiring goes in `ObjectImpl::constructed()` using `glib::clone!`.

## Concurrency
- Use `glib::MainContext::default().spawn_local()` for async work on the GTK main loop.
- The tokio network stack runs on a dedicated OS thread; channels (`async_channel`) bridge the two.
- Never call blocking I/O on the GTK main thread.

## Project Structure
```
src/
  main.rs          — minimal: register GResources, install TLS provider, run Application
  application.rs   — AdwApplication subclass (startup: channels + network thread; activate: window)
  window/
    mod.rs         — public Window API (setup, event dispatch)
    imp.rs         — GObject implementation, template children, signal wiring
  widgets/         — reusable GObject widget subclasses (future)
  ui/              — stateless helper functions (peer_list, preferences, dialogs, notifications)
  app/             — network layer (AppEvent, AppCommand, HTTP handlers, run_network)
```

## Error Handling
- Use `anyhow` for fallible functions in the network/service layer.
- No `unwrap()` or `expect()` in production paths; propagate with `?` or log + return.
- GTK signal handlers may use `let Some(x) = ... else { return };` for early exit.
