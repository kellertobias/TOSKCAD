# Trackpad navigation — 2026-10-28

CAD-124 adds camera navigation without changing the canonical document.

| Input over the viewport | Action |
| --- | --- |
| Pinch | Zoom by the reported ratio, once per frame |
| Precise two-axis scroll | Pan, including momentum |
| Notched wheel / line scroll | Zoom |
| Option + precise scroll | Orbit fallback |
| Three indirect contacts delivered by AppKit | Orbit from their centroid movement |
| Right drag / middle drag / WASD | Existing orbit / pan / keyboard pan |

The pinned `egui-winit` 0.36.2 adapter already converts native pinch into `Event::Zoom` and preserves pixel-versus-line scroll units. The viewport reads those raw events and does not also apply egui's smoothed scroll or derived zoom. A frame guard prevents a layout retry from applying input again. Invalid input is rejected; accumulated native deltas and scroll deltas are bounded.

On macOS, the audited adapter enables indirect touches on the existing winit view through public AppKit APIs. It observes `touchesBegan/Moved/Ended/CancelledWithEvent:`, tracks exactly three stable contact identities, and converts normalized positions using `NSTouch.deviceSize`, documented by the SDK in points. It preserves the view's class and ivars, forwards inherited callbacks, and refuses to replace callbacks already owned by the host. It drains pending input every application frame, including welcome and modal screens. The Rust viewport checks the gesture's starting location and locks out rejected gestures until a new contact sequence.

macOS may reserve three-finger gestures for Mission Control, App Exposé or dragging. Applications cannot depend on receiving those contacts: Apple's [trackpad event documentation](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/HandlingTouchEvents/HandlingTouchEvents.html) describes system gesture precedence and the distinction between raw touch and recognized gesture events. This implementation changes no system settings and uses no private multitouch APIs. Option plus two-finger scroll and right drag remain available. `NSPanGestureRecognizer.numberOfTouchesRequired` is a Touch Bar API in the SDK; it is not used as a trackpad substitute. Precise-scroll provenance does not identify the physical device, so other devices reporting precise scrolling also pan.

Automated coverage includes raw pinch and scroll classification; nonfinite input; simultaneous derived scroll suppression; an actual egui layout retry; disabled/outside-viewport input; gesture-origin and modal ownership; exactly-three-contact identity changes; native callback forwarding and foreign/inherited callback rejection; and a real SketchEditor source/revision preservation check. Existing middle-drag and shortcut tests remain in the workspace suite.

Required checks use Rust 1.99.1 and the committed lockfile: `cargo fmt --all -- --check`, `CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and `CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline`. Incremental caching was disabled after deleting only this checkout's disposable cache to recover disk space. Physical macOS trackpad checks and Windows/Linux runtime verification remain separate from these synthetic tests.
