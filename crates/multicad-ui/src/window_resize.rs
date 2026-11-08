//! A client grip for the native window and resize borders on undecorated hosts.
use crate::egui::{
    self, CursorIcon, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, ViewportCommand,
};

#[cfg(not(target_os = "macos"))]
use egui::viewport::ResizeDirection;

pub(crate) const MIN_CLIENT_SIZE: [f32; 2] = [1024., 600.];
const GRIP: f32 = 22.;

pub(crate) fn show(ui: &Ui) {
    let ctx = ui.ctx();
    let unavailable = ctx.input(|input| {
        let window = input.viewport();
        window.maximized == Some(true) || window.fullscreen == Some(true)
    });
    if unavailable {
        return;
    }

    let window = ctx.viewport_rect();
    let grip = Rect::from_min_max(window.max - Vec2::splat(GRIP), window.max);
    let response = region(ctx, "bottom-right", grip, CursorIcon::ResizeNwSe, true);
    #[cfg(target_os = "macos")]
    resize_macos(&response);
    #[cfg(not(target_os = "macos"))]
    {
        resize_native(&response, ResizeDirection::SouthEast);
        resize_borders(ctx, window);
    }
}

fn region(
    ctx: &egui::Context,
    id: &str,
    rect: Rect,
    cursor: CursorIcon,
    draw_grip: bool,
) -> Response {
    egui::Area::new(Id::new(("window-resize", id)))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .default_size(rect.size())
        .constrain(false)
        .movable(false)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(rect.size(), Sense::drag());
            if draw_grip {
                let corner = rect.max - egui::vec2(4., 4.);
                let color = if response.hovered() || response.dragged() {
                    ui.visuals().text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                for length in [7., 13.] {
                    ui.painter().line_segment(
                        [
                            corner - egui::vec2(length, 0.),
                            corner - egui::vec2(0., length),
                        ],
                        Stroke::new(1.25, color),
                    );
                }
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Resize window")
                });
                response.on_hover_text("Resize window from the edges or corners")
            } else {
                response
            }
            .on_hover_and_drag_cursor(cursor)
        })
        .inner
}

#[cfg(not(target_os = "macos"))]
fn resize_native(response: &Response, direction: ResizeDirection) {
    if response.drag_started_by(egui::PointerButton::Primary) {
        response
            .ctx
            .send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

#[cfg(not(target_os = "macos"))]
fn resize_borders(ctx: &egui::Context, window: Rect) {
    // macOS has native decorated borders. On Windows and Linux the client
    // supplies these zones because the native window is undecorated.
    use CursorIcon as Cursor;
    use ResizeDirection as Direction;
    const EDGE: f32 = 5.;
    const CORNER: f32 = 12.;
    let [l, t, r, b] = [window.left(), window.top(), window.right(), window.bottom()];
    let rect = |x0, y0, x1, y1| Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
    for (id, bounds, cursor, direction) in [
        (
            "top",
            rect(l + CORNER, t, r - CORNER, t + EDGE),
            Cursor::ResizeNorth,
            Direction::North,
        ),
        (
            "bottom",
            rect(l + CORNER, b - EDGE, r - GRIP, b),
            Cursor::ResizeSouth,
            Direction::South,
        ),
        (
            "left",
            rect(l, t + CORNER, l + EDGE, b - CORNER),
            Cursor::ResizeWest,
            Direction::West,
        ),
        (
            "right",
            rect(r - EDGE, t + CORNER, r, b - GRIP),
            Cursor::ResizeEast,
            Direction::East,
        ),
        (
            "top-left",
            rect(l, t, l + CORNER, t + CORNER),
            Cursor::ResizeNwSe,
            Direction::NorthWest,
        ),
        (
            "top-right",
            rect(r - CORNER, t, r, t + CORNER),
            Cursor::ResizeNeSw,
            Direction::NorthEast,
        ),
        (
            "bottom-left",
            rect(l, b - CORNER, l + CORNER, b),
            Cursor::ResizeNeSw,
            Direction::SouthWest,
        ),
    ] {
        resize_native(&region(ctx, id, bounds, cursor, false), direction);
    }
}

#[cfg(target_os = "macos")]
fn resize_macos(response: &Response) {
    use egui::PointerButton::Primary;

    #[derive(Clone)]
    struct Drag {
        pointer: Pos2,
        size: Vec2,
        anchor: Pos2,
    }

    let ctx = &response.ctx;
    let Some((inner, outer)) = ctx.input(|input| {
        let window = input.viewport();
        window.inner_rect.zip(window.outer_rect)
    }) else {
        return;
    };
    let id = response.id;
    if response.drag_started_by(Primary)
        && let Some(press) = ctx.input(|input| input.pointer.press_origin())
    {
        ctx.data_mut(|data| {
            data.insert_temp(
                id,
                Drag {
                    pointer: inner.min + press.to_vec2(),
                    size: inner.size(),
                    anchor: outer.min,
                },
            );
        });
    }
    if (response.dragged_by(Primary) || response.drag_stopped_by(Primary))
        && let Some(drag) = ctx.data(|data| data.get_temp::<Drag>(id))
        && let Some(pointer) = ctx.input(|input| input.pointer.interact_pos())
    {
        let delta = inner.min + pointer.to_vec2() - drag.pointer;
        let minimum = Vec2::from(MIN_CLIENT_SIZE) / ctx.zoom_factor();
        let size = (drag.size + delta).max(minimum);
        if (size - inner.size()).abs().max_elem() >= 0.5 {
            // Winit does not implement BeginResize on macOS. Keep the
            // top-left corner fixed when AppKit applies the requested size.
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(drag.anchor));
        }
    }
    if !response.dragged_by(Primary) {
        ctx.data_mut(|data| data.remove::<Drag>(id));
    }
}
