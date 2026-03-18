// This module has been replaced by `window::Window::dispatch_event`.
//
// Event routing is now a method on the `Window` GObject so it can access all
// widget state directly via `self.imp()`, without threading widget references
// through a long parameter list. See `src/window/mod.rs`.
