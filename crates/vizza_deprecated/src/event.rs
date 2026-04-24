use crate::renderer::Renderer;
use std::time::{Duration, Instant};
use winit::{event::WindowEvent, event_loop::ActiveEventLoop};

const DRAG_REDRAW_THROTTLE: Duration = Duration::from_millis(30);

/// Mouse interaction state
#[derive(Debug, Clone, Copy)]
pub struct MouseState {
    pub position: (f64, f64),
    pub dragging: bool,
    pub last_drag_pos: (f64, f64),
    pub drag_viewport: Option<usize>,
    pub rescaling_y: bool,
    pub last_drag_redraw: Option<Instant>,
    pub redraw_pending: bool,
    /// Currently hovered viewport index
    pub hover_viewport: Option<usize>,
    /// Currently hovered bar index within the viewport's visible range
    pub hover_bar_index: Option<usize>,
    /// Middle mouse button is pressed (shows tooltip)
    pub middle_button_pressed: bool,
}

impl MouseState {
    pub fn new() -> Self {
        Self {
            position: (0.0, 0.0),
            dragging: false,
            last_drag_pos: (0.0, 0.0),
            drag_viewport: None,
            rescaling_y: false,
            last_drag_redraw: None,
            redraw_pending: false,
            hover_viewport: None,
            hover_bar_index: None,
            middle_button_pressed: false,
        }
    }
}

/// Handles window close request
pub fn handle_close_requested(event_loop: &ActiveEventLoop) {
    event_loop.exit();
}

/// Handles window resize
pub fn handle_resize(
    renderer: &mut Renderer,
    physical_size: winit::dpi::PhysicalSize<u32>,
) -> bool {
    renderer.resize(physical_size);
    true
}

/// Handles cursor movement and dragging
pub fn handle_cursor_moved(
    renderer: &mut Renderer,
    mouse_state: &mut MouseState,
    position: winit::dpi::PhysicalPosition<f64>,
) -> bool {
    mouse_state.position = (position.x, position.y);

    if mouse_state.dragging {
        // Clear hover state while dragging
        mouse_state.hover_viewport = None;
        mouse_state.hover_bar_index = None;

        // Only pan if we have a valid drag viewport
        if let Some(viewport_idx) = mouse_state.drag_viewport {
            let dx = position.x - mouse_state.last_drag_pos.0;
            let dy = position.y - mouse_state.last_drag_pos.1;
            // Use the rescaling_y flag that was set when drag started
            renderer.handle_pan_viewport(viewport_idx, dx, dy, mouse_state.rescaling_y);
            mouse_state.last_drag_pos = (position.x, position.y);

            if mouse_state.redraw_pending {
                return false;
            }

            let now = Instant::now();
            let should_render = match mouse_state.last_drag_redraw {
                Some(last) => now.duration_since(last) >= DRAG_REDRAW_THROTTLE,
                None => true,
            };

            if should_render {
                mouse_state.last_drag_redraw = Some(now);
                return true;
            }
        }
        return false;
    }

    // Update hover state when not dragging
    let old_hover_viewport = mouse_state.hover_viewport;
    let old_hover_bar = mouse_state.hover_bar_index;

    // Find viewport under cursor and calculate bar index
    if let Some(viewport_idx) = renderer.find_viewport_index_at(position.x, position.y) {
        mouse_state.hover_viewport = Some(viewport_idx);

        // Calculate which bar is under the cursor
        if let Some(bar_idx) = renderer.get_bar_index_at(viewport_idx, position.x) {
            mouse_state.hover_bar_index = Some(bar_idx);
        } else {
            mouse_state.hover_bar_index = None;
        }
    } else {
        mouse_state.hover_viewport = None;
        mouse_state.hover_bar_index = None;
    }

    // Request redraw if hover state changed
    let hover_changed = old_hover_viewport != mouse_state.hover_viewport
        || old_hover_bar != mouse_state.hover_bar_index;

    // Update tooltip with current hover state (only show if middle button pressed)
    renderer.update_hover(
        mouse_state.hover_viewport,
        mouse_state.hover_bar_index,
        position.x as f32,
        position.y as f32,
        mouse_state.middle_button_pressed,
    );

    if hover_changed {
        if let Some(idx) = old_hover_viewport {
            renderer.mark_viewport_dirty(idx);
        }
        if let Some(idx) = mouse_state.hover_viewport {
            renderer.mark_viewport_dirty(idx);
        }
    }

    hover_changed
}

/// Handles mouse wheel scrolling
pub fn handle_mouse_wheel(
    renderer: &mut Renderer,
    mouse_state: &MouseState,
    ctrl_pressed: bool,
    delta: winit::event::MouseScrollDelta,
) -> bool {
    let scroll_delta = match delta {
        winit::event::MouseScrollDelta::LineDelta(_x, y) => y as f64,
        winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
    };

    let (mouse_x, mouse_y) = mouse_state.position;
    renderer.handle_scroll(scroll_delta, mouse_x, mouse_y, ctrl_pressed)
}

/// Handles mouse button input
pub fn handle_mouse_input(
    renderer: &mut Renderer,
    mouse_state: &mut MouseState,
    ctrl_pressed: bool,
    state: winit::event::ElementState,
    button: winit::event::MouseButton,
) -> bool {
    let mut dirty = false;

    match button {
        winit::event::MouseButton::Left => {
            match state {
                winit::event::ElementState::Pressed => {
                    mouse_state.dragging = true;
                    mouse_state.last_drag_pos = mouse_state.position;
                    // Record which viewport the drag started in
                    mouse_state.drag_viewport =
                        renderer.find_viewport_index_at(mouse_state.position.0, mouse_state.position.1);
                    mouse_state.last_drag_redraw = None;

                    // Set focus to the viewport being clicked
                    if let Some(idx) = mouse_state.drag_viewport {
                        dirty |= renderer.set_focused_viewport(idx);

                        // Check if we should start y-rescaling mode
                        if let Some(vp) = renderer.viewport_states.get(idx) {
                            let relative_x = mouse_state.position.0 as f32 - vp.x;
                            let in_last_15_percent = relative_x >= vp.width * 0.85;
                            mouse_state.rescaling_y = ctrl_pressed && in_last_15_percent;
                        }
                    }
                }
                winit::event::ElementState::Released => {
                    let was_dragging = mouse_state.dragging;
                    let released_viewport = mouse_state.drag_viewport;
                    mouse_state.dragging = false;
                    mouse_state.drag_viewport = None;
                    mouse_state.rescaling_y = false;
                    mouse_state.last_drag_redraw = None;

                    if was_dragging {
                        if let Some(idx) = released_viewport {
                            renderer.mark_viewport_dirty(idx);
                        }
                        dirty = true;
                    }
                }
            }
        }
        winit::event::MouseButton::Middle => {
            match state {
                winit::event::ElementState::Pressed => {
                    mouse_state.middle_button_pressed = true;
                    // Update tooltip immediately when middle button pressed
                    renderer.update_hover(
                        mouse_state.hover_viewport,
                        mouse_state.hover_bar_index,
                        mouse_state.position.0 as f32,
                        mouse_state.position.1 as f32,
                        true, // show tooltip
                    );
                    if mouse_state.hover_viewport.is_some() {
                        dirty = true;
                    }
                }
                winit::event::ElementState::Released => {
                    mouse_state.middle_button_pressed = false;
                    // Hide tooltip when middle button released
                    renderer.update_hover(
                        mouse_state.hover_viewport,
                        mouse_state.hover_bar_index,
                        mouse_state.position.0 as f32,
                        mouse_state.position.1 as f32,
                        false, // hide tooltip
                    );
                    if let Some(idx) = mouse_state.hover_viewport {
                        renderer.mark_viewport_dirty(idx);
                    }
                    dirty = true;
                }
            }
        }
        _ => {}
    }

    dirty
}

/// Handles keyboard input
pub fn handle_keyboard_input(
    renderer: &mut Renderer,
    ctrl_pressed: &mut bool,
    event: winit::event::KeyEvent,
) -> bool {
    use winit::keyboard::{KeyCode, PhysicalKey};

    let mut dirty = false;

    if let PhysicalKey::Code(key_code) = event.physical_key {
        // Track Ctrl key state
        match key_code {
            KeyCode::ControlLeft | KeyCode::ControlRight => {
                *ctrl_pressed = event.state == winit::event::ElementState::Pressed;
            }
            _ => {}
        }

        if event.state == winit::event::ElementState::Pressed {
            // Apply to focused viewport
            let focused_idx = renderer.focused_viewport_idx();
            if let Some(viewport) = renderer.viewport_states.get_mut(focused_idx) {
                match key_code {
                    KeyCode::KeyE => {
                        viewport.pan_to_end_with_lod();
                        println!("Viewport {}: Panned to end with 1m LOD", focused_idx);
                        renderer.update_viewport_instances(focused_idx);
                        dirty = true;
                    }
                    KeyCode::KeyY => {
                        viewport.view_settings.auto_y_scale = !viewport.view_settings.auto_y_scale;
                        println!(
                            "Viewport {}: Auto Y scale: {}",
                            focused_idx,
                            if viewport.view_settings.auto_y_scale {
                                "ON"
                            } else {
                                "OFF"
                            }
                        );
                        renderer.update_viewport_instances(focused_idx);
                        dirty = true;
                    }
                    KeyCode::KeyG => {
                        renderer.toggle_grid_visibility(focused_idx);
                        dirty = true;
                    }
                    KeyCode::KeyS => {
                        viewport.show_line_overlays = !viewport.show_line_overlays;
                        println!(
                            "Viewport {}: Line overlays: {}",
                            focused_idx,
                            if viewport.show_line_overlays { "ON" } else { "OFF" }
                        );
                        renderer.update_viewport_instances(focused_idx);
                        dirty = true;
                    }
                    KeyCode::Comma => {
                        if viewport.zoom.decrease_bar_width() {
                            println!(
                                "Viewport {}: Bar width {} px",
                                focused_idx, viewport.zoom.bar_width_px
                            );
                            renderer.update_viewport_instances(focused_idx);
                            dirty = true;
                        }
                    }
                    KeyCode::Period => {
                        if viewport.zoom.increase_bar_width() {
                            println!(
                                "Viewport {}: Bar width {} px",
                                focused_idx, viewport.zoom.bar_width_px
                            );
                            renderer.update_viewport_instances(focused_idx);
                            dirty = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    dirty
}

/// Main event dispatcher
pub fn dispatch_window_event(
    event: WindowEvent,
    event_loop: &ActiveEventLoop,
    renderer: &mut Option<Renderer>,
    mouse_state: &mut MouseState,
    ctrl_pressed: &mut bool,
) -> bool {
    match event {
        WindowEvent::CloseRequested => {
            handle_close_requested(event_loop);
            false
        }
        WindowEvent::Resized(physical_size) => {
            if let Some(renderer) = renderer {
                return handle_resize(renderer, physical_size);
            }
            false
        }
        WindowEvent::CursorMoved { position, .. } => {
            if let Some(renderer) = renderer {
                return handle_cursor_moved(renderer, mouse_state, position);
            }
            false
        }
        WindowEvent::MouseWheel { delta, .. } => {
            if let Some(renderer) = renderer {
                return handle_mouse_wheel(renderer, mouse_state, *ctrl_pressed, delta);
            }
            false
        }
        WindowEvent::MouseInput { state, button, .. } => {
            if let Some(renderer) = renderer.as_mut() {
                return handle_mouse_input(renderer, mouse_state, *ctrl_pressed, state, button);
            }
            false
        }
        WindowEvent::KeyboardInput { event, .. } => {
            if let Some(renderer) = renderer {
                return handle_keyboard_input(renderer, ctrl_pressed, event);
            }
            false
        }
        _ => false,
    }
}
