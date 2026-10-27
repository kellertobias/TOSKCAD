#include "native_window.hpp"
#import <AppKit/AppKit.h>
#include <cmath>
namespace multicad::window {
bool position_buttons(std::size_t address, double height) noexcept {
    if (![NSThread isMainThread] || !address || !std::isfinite(height)) return false;
    NSView* view = (__bridge NSView*)reinterpret_cast<void*>(address);
    NSWindow* window = view.window;
    NSView* content = window.contentView;
    if (!content) return false;
    NSButton* buttons[] = { [window standardWindowButton:NSWindowCloseButton],
        [window standardWindowButton:NSWindowMiniaturizeButton],
        [window standardWindowButton:NSWindowZoomButton] };
    for (auto* button : buttons) if (!button || !button.superview) return false;
    const double size = buttons[0].frame.size.height;
    const double inset = (height - size) / 2.0;
    for (int i=0;i<3;++i) {
        auto* button=buttons[i];
        NSPoint center=NSMakePoint(inset + button.frame.size.width/2.0 + i*20.0,
            content.isFlipped ? height/2.0 : content.bounds.size.height-height/2.0);
        center=[button.superview convertPoint:center fromView:content];
        NSPoint origin=NSMakePoint(center.x-button.frame.size.width/2.0,center.y-button.frame.size.height/2.0);
        if (std::abs(button.frame.origin.x-origin.x)>0.1 || std::abs(button.frame.origin.y-origin.y)>0.1)
            [button setFrameOrigin:origin];
    }
    return true;
}
}
