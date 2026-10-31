//! Pointer guidance for the sketch drawing tools (CAD-158).
//!
//! The guide is derived from the tool's real gesture state so the Inspector
//! and status line describe the next viewport click, when geometry is
//! committed and what Escape discards. Escape runs `SketchEditor::cancel`,
//! which clears only uncommitted progress and ends the tool; geometry already
//! committed stays in the sketch and Undo removes it. Only Spline has an
//! explicit completion action; every other tool commits from its final click.
use super::*;

/// What the Inspector shows for the active drawing tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolGuide {
    /// The next pointer step, e.g. "Step 2 of 2 · Click the opposite corner".
    pub step: String,
    /// Uncommitted input, or already committed chain state, if any.
    pub pending: Option<String>,
    /// How the gesture is committed to the sketch.
    pub commit: String,
    /// What Escape discards; never claims to remove committed geometry.
    pub escape: String,
    /// Short Escape hint for the status line.
    pub escape_short: &'static str,
    /// Whether the tool has an explicit completion action (Spline only).
    pub finish_action: bool,
}

impl ToolGuide {
    /// One-line status text for the viewport message.
    pub fn status(&self) -> String {
        format!("{} · {}", self.step, self.escape_short)
    }
}

impl DrawTool {
    /// The names of the points the click gesture asks for, in order. Empty
    /// for tools without a fixed point sequence.
    fn pointer_points(self) -> &'static [&'static str] {
        match self {
            Self::Line => &["start point", "end point"],
            Self::Rectangle => &["first corner", "opposite corner"],
            Self::Circle => &["center", "point on the circle that sets the radius"],
            Self::Arc => &["center", "start point (sets the radius)", "end point"],
            Self::RectangleCenter => &["rectangle center", "corner"],
            Self::RectangleCenterTwo => &[
                "rectangle center",
                "first corner",
                "point toward the neighbouring corner (same distance from the center)",
            ],
            Self::RectangleThree => &[
                "first corner",
                "second corner of the first edge",
                "point that sets the rectangle height",
            ],
            Self::CircleTwoRadius => &[
                "first point on the circle",
                "second point on the circle",
                "point that chooses the center side and radius",
            ],
            Self::CircleThree => &[
                "first point on the circle",
                "second point on the circle",
                "third point on the circle",
            ],
            Self::Polyline | Self::Spline | Self::Fillet | Self::Chamfer | Self::Text => &[],
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Rectangle
            | Self::RectangleCenter
            | Self::RectangleCenterTwo
            | Self::RectangleThree => "rectangle",
            Self::Circle | Self::CircleTwoRadius | Self::CircleThree => "circle",
            Self::Arc => "arc",
            Self::Polyline => "polyline segment",
            Self::Spline => "spline",
            Self::Fillet => "fillet",
            Self::Chamfer => "chamfer",
            Self::Text => "text",
        }
    }
    /// Guidance for `pending` uncommitted pointer inputs. For Polyline,
    /// `segments` counts the segments of the open chain that are already
    /// committed; Spline's `pending` is its control-pole count.
    pub(crate) fn guide(self, pending: usize, segments: usize) -> ToolGuide {
        let label = self.label();
        let ends = |what: &str| format!("Esc ends the {label} tool. {what}");
        match self {
            Self::Spline => ToolGuide {
                step: if pending < 4 {
                    format!(
                        "Click control poles · {pending} placed, at least 4 needed to finish"
                    )
                } else {
                    format!(
                        "Click more control poles, or finish · {pending} placed · click the first pole to close"
                    )
                },
                pending: (pending > 0).then(|| {
                    format!("Pending: {pending} control poles · not in the sketch yet")
                }),
                commit: "Nothing is committed while you place poles. Apply in the action bar \
                         (or Enter) commits an open spline once four poles are placed; clicking \
                         the first pole commits a closed spline."
                    .into(),
                escape: if pending > 0 {
                    format!(
                        "Esc discards the {pending} pending poles and ends the tool. Existing geometry stays."
                    )
                } else {
                    ends("Existing geometry stays.")
                },
                escape_short: if pending > 0 {
                    "Esc discards the pending poles"
                } else {
                    "Esc ends the tool"
                },
                finish_action: true,
            },
            Self::Polyline => ToolGuide {
                step: if pending == 0 {
                    "Step 1 · Click the first vertex".into()
                } else {
                    "Click the next vertex · click the first vertex to close".into()
                },
                pending: if segments > 0 {
                    Some(format!(
                        "{segments} segment{} already committed · the chain continues from its last vertex",
                        if segments == 1 { "" } else { "s" }
                    ))
                } else {
                    (pending > 0).then(|| "Pending: first vertex · not in the sketch yet".into())
                },
                commit: "Each click after the first commits one segment immediately; no Apply is \
                         needed. Clicking the first vertex closes the chain (three vertices or more)."
                    .into(),
                escape: if segments > 0 {
                    "Esc stops the chain and ends the tool. Committed segments stay in the sketch; \
                     Undo removes them."
                        .into()
                } else if pending > 0 {
                    "Esc discards the pending first vertex and ends the tool. Existing geometry stays."
                        .into()
                } else {
                    ends("Existing geometry stays.")
                },
                escape_short: if segments > 0 {
                    "Esc stops the chain (committed segments stay)"
                } else if pending > 0 {
                    "Esc discards the pending vertex"
                } else {
                    "Esc ends the tool"
                },
                finish_action: false,
            },
            Self::Fillet | Self::Chamfer => ToolGuide {
                step: "Click a corner shared by two lines or arcs".into(),
                pending: None,
                commit: format!(
                    "Each corner click applies the {} {} from this Inspector immediately; the tool \
                     stays active for the next corner.",
                    self.noun(),
                    if self == Self::Fillet { "radius" } else { "setback" }
                ),
                escape: ends("Corners already treated stay in the sketch; Undo removes them."),
                escape_short: "Esc ends the tool",
                finish_action: false,
            },
            Self::Text => ToolGuide {
                step: "Click the anchor position · click a letter to edit existing text".into(),
                pending: None,
                commit: "Each click places the Inspector text immediately; the tool stays active."
                    .into(),
                escape: ends("Text already placed stays in the sketch; Undo removes it."),
                escape_short: "Esc ends the tool",
                finish_action: false,
            },
            _ => {
                let points = self.pointer_points();
                let total = points.len();
                let index = pending.min(total - 1);
                let noun = self.noun();
                let placed = points[..pending.min(total)].join(", ");
                ToolGuide {
                    step: format!("Step {} of {total} · Click the {}", index + 1, points[index]),
                    pending: (pending > 0)
                        .then(|| format!("Pending: {placed} · not in the sketch yet")),
                    commit: format!(
                        "The {} click commits the {noun} immediately; no Apply is needed. \
                         The tool stays active for the next {noun}.",
                        points[total - 1]
                            .split_once(" (")
                            .map_or(points[total - 1], |(name, _)| name)
                    ),
                    escape: if pending > 0 {
                        format!(
                            "Esc discards the pending {placed} and ends the tool. Committed geometry \
                             stays in the sketch; Undo removes it."
                        )
                    } else {
                        ends(&format!(
                            "Each {noun} already committed stays in the sketch; Undo removes it."
                        ))
                    },
                    escape_short: if pending > 0 {
                        "Esc discards the pending points"
                    } else {
                        "Esc ends the tool"
                    },
                    finish_action: false,
                }
            }
        }
    }
}

impl SketchEditor {
    /// Uncommitted pointer inputs of the active drawing tool.
    pub(super) fn drawing_pending_points(&self) -> usize {
        match self.tool {
            Some(DrawTool::Spline) => self.spline_poles.len(),
            Some(tool) if tool.corner() || tool == DrawTool::Text => 0,
            Some(_) => usize::from(self.first.is_some()) + usize::from(self.arc_start.is_some()),
            None => 0,
        }
    }
    /// Guidance for the active drawing tool, if any.
    pub(crate) fn tool_guide(&self) -> Option<ToolGuide> {
        let tool = self.tool?;
        let segments = if tool == DrawTool::Polyline {
            self.polyline_segments
        } else {
            0
        };
        Some(tool.guide(self.drawing_pending_points(), segments))
    }
    /// Status line for the current pointer step of the active tool.
    pub(super) fn set_tool_status(&mut self) {
        if let Some(guide) = self.tool_guide() {
            self.message = guide.status();
        }
    }
    /// Status line after a pointer gesture committed geometry.
    pub(super) fn set_committed_status(&mut self, tool: DrawTool) {
        if let Some(guide) = self.tool_guide() {
            let noun = tool.noun();
            let mut noun = noun.to_owned();
            noun[..1].make_ascii_uppercase();
            self.message = format!("{noun} committed · Undo removes it · {}", guide.status());
        }
    }
    /// Inspector body for the active drawing tool: the pointer step, pending
    /// input, commit behavior, Escape semantics and optional command syntax.
    /// Tools without a completion action get no Apply footer; their inline
    /// controls only clear pending input or end the tool, exactly like Esc.
    pub(super) fn tool_guide_ui(&mut self, ui: &mut egui::Ui, tool: DrawTool) {
        let Some(guide) = self.tool_guide() else {
            return;
        };
        ui.label(egui::RichText::new(&guide.step).color(Color32::LIGHT_YELLOW));
        match &guide.pending {
            Some(pending) => ui.small(pending),
            None => ui.small("No pending input"),
        };
        ui.small(&guide.commit);
        ui.small(&guide.escape);
        if !guide.finish_action {
            let pending = self.drawing_pending_points();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(pending > 0, egui::Button::new("Clear pending points"))
                    .on_hover_text("Discard uncommitted clicks and keep the tool active; committed geometry stays")
                    .on_disabled_hover_text("No uncommitted clicks")
                    .clicked()
                {
                    self.clear_drawing_progress();
                    self.set_tool_status();
                }
                if ui
                    .button(format!("End {} (Esc)", tool.label()))
                    .on_hover_text("End the tool and discard only uncommitted clicks; committed geometry stays and Undo removes it")
                    .clicked()
                {
                    self.cancel();
                    self.message = format!("{} ended · committed geometry kept", tool.label());
                }
            });
        }
        egui::CollapsingHeader::new("Command entry")
            .id_salt(("sketch-command-entry", tool.id()))
            .default_open(false)
            .show(ui, |ui| {
                ui.small("Optional exact input for the command line:");
                ui.monospace(tool.usage());
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_prompts_for_first_then_opposite_corner_and_commits_without_apply() {
        let start = DrawTool::Rectangle.guide(0, 0);
        assert_eq!(start.step, "Step 1 of 2 · Click the first corner");
        assert_eq!(start.pending, None);
        assert!(!start.finish_action);
        assert!(start.commit.contains("no Apply is needed"));
        assert!(!start.commit.to_lowercase().contains("spline"));
        let second = DrawTool::Rectangle.guide(1, 0);
        assert_eq!(second.step, "Step 2 of 2 · Click the opposite corner");
        assert_eq!(
            second.pending.as_deref(),
            Some("Pending: first corner · not in the sketch yet")
        );
        assert!(
            second
                .commit
                .starts_with("The opposite corner click commits")
        );
    }

    #[test]
    fn every_tool_has_step_commit_and_escape_guidance_that_keeps_committed_geometry() {
        for tool in DrawTool::ALL {
            let max = tool.pointer_points().len().max(5);
            for pending in 0..=max {
                for segments in [0, 2] {
                    let guide = tool.guide(pending, segments);
                    assert!(!guide.step.is_empty(), "{tool:?}");
                    assert!(!guide.commit.is_empty(), "{tool:?}");
                    assert!(guide.escape.starts_with("Esc "), "{tool:?}");
                    assert!(guide.status().contains(guide.escape_short));
                    let escape = guide.escape.to_lowercase();
                    // Escape only discards uncommitted input.
                    assert!(
                        escape.contains("stay"),
                        "{tool:?} escape must say what stays: {escape}"
                    );
                    assert!(!escape.contains("removes the committed"));
                    assert_eq!(guide.finish_action, tool == DrawTool::Spline);
                }
            }
        }
    }

    #[test]
    fn multi_click_steps_follow_the_click_count() {
        for tool in DrawTool::ALL {
            let points = tool.pointer_points();
            for (index, name) in points.iter().enumerate() {
                let guide = tool.guide(index, 0);
                assert_eq!(
                    guide.step,
                    format!("Step {} of {} · Click the {name}", index + 1, points.len()),
                );
                assert_eq!(guide.pending.is_some(), index > 0);
            }
            if !points.is_empty() && tool.multi_point() {
                assert_eq!(points.len(), tool.clicks(), "{tool:?}");
            }
        }
    }

    #[test]
    fn spline_finish_guard_and_polyline_chain_state_are_explicit() {
        let few = DrawTool::Spline.guide(3, 0);
        assert!(few.step.contains("at least 4"));
        assert!(few.escape.contains("3 pending poles"));
        assert!(DrawTool::Spline.guide(4, 0).step.contains("finish"));
        let chain = DrawTool::Polyline.guide(1, 2);
        assert!(
            chain
                .pending
                .unwrap()
                .contains("2 segments already committed")
        );
        assert!(chain.escape.contains("Committed segments stay"));
        assert!(
            DrawTool::Polyline
                .guide(1, 0)
                .pending
                .unwrap()
                .contains("not in the sketch yet")
        );
    }
}
