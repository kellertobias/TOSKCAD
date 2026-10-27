#include "native_window.hpp"
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <cmath>

namespace {
char quit_guard_state_key;
}
@interface MultiCADQuitGuardState : NSObject
@property(nonatomic, weak) NSWindow* window;
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication*)application;
@end
@implementation MultiCADQuitGuardState
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication*)application {
    // The IMP is added to the existing delegate class. Its receiver is that
    // original delegate object, so never access MultiCADQuitGuardState ivars here.
    if (![NSThread isMainThread]) return NSTerminateCancel;
    MultiCADQuitGuardState* state = objc_getAssociatedObject(self, &quit_guard_state_key);
    NSWindow* window = state.window;
    if (window) {
        // Return from AppKit's termination query before sending the ordinary
        // window-close event to winit and the Rust unsaved/pending-work guard.
        [window performSelector:@selector(performClose:) withObject:application afterDelay:0.0];
    }
    return NSTerminateCancel;
}
@end

namespace {
std::uint8_t install_on_delegate(id delegate, NSWindow* window) {
    if (!delegate) return 3;
    SEL selector = @selector(applicationShouldTerminate:);
    Method helper = class_getInstanceMethod([MultiCADQuitGuardState class], selector);
    if (!helper) return 5;
    Class delegate_class = object_getClass(delegate);
    Method existing = class_getInstanceMethod(delegate_class, selector);
    IMP implementation = method_getImplementation(helper);
    // Never replace the delegate object, change its class or overwrite a host
    // callback. Winit downcasts this exact object and accesses its Rust ivars.
    if (existing && method_getImplementation(existing) != implementation) return 4;
    if (!existing && !class_addMethod(delegate_class, selector, implementation,
                                     method_getTypeEncoding(helper))) return 5;
    MultiCADQuitGuardState* state = objc_getAssociatedObject(delegate, &quit_guard_state_key);
    if (!state) {
        state = [MultiCADQuitGuardState new];
        objc_setAssociatedObject(delegate, &quit_guard_state_key, state,
                                 OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    state.window = window;
    return 0;
}
}

// Contract probes allocate only plain NSObject delegates, never an NSApplication
// or NSWindow. They can run in ordinary macOS tests without opening any UI.
@interface MultiCADQuitGuardProbe : NSObject
- (NSInteger)existingCallback;
@end
@implementation MultiCADQuitGuardProbe
- (NSInteger)existingCallback { return 47; }
@end
@interface MultiCADForeignQuitGuardProbe : NSObject
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication*)application;
@end
@implementation MultiCADForeignQuitGuardProbe
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication*)application {
    (void)application;
    return NSTerminateNow;
}
@end

namespace multicad::window {
std::uint8_t install_quit_guard(std::size_t address) noexcept {
    if (![NSThread isMainThread]) return 1;
    if (!address) return 2;
    NSView* view = (__bridge NSView*)reinterpret_cast<void*>(address);
    NSWindow* window = view.window;
    if (!window) return 2;
    return install_on_delegate(NSApp.delegate, window);
}
bool quit_guard_contract_test() noexcept {
    @autoreleasepool {
        MultiCADQuitGuardProbe* delegate = [MultiCADQuitGuardProbe new];
        Class original_class = object_getClass(delegate);
        Method original_callback = class_getInstanceMethod(original_class, @selector(existingCallback));
        IMP original_imp = method_getImplementation(original_callback);
        if (install_on_delegate(delegate, nil) != 0 ||
            install_on_delegate(delegate, nil) != 0 ||
            object_getClass(delegate) != original_class ||
            method_getImplementation(class_getInstanceMethod(original_class, @selector(existingCallback))) != original_imp ||
            [delegate existingCallback] != 47) return false;
        auto guarded = (MultiCADQuitGuardState*)delegate;
        if ([guarded applicationShouldTerminate:nil] != NSTerminateCancel) return false;
        MultiCADForeignQuitGuardProbe* foreign = [MultiCADForeignQuitGuardProbe new];
        IMP foreign_imp = method_getImplementation(class_getInstanceMethod(object_getClass(foreign), @selector(applicationShouldTerminate:)));
        if (install_on_delegate(foreign, nil) != 4 ||
            method_getImplementation(class_getInstanceMethod(object_getClass(foreign), @selector(applicationShouldTerminate:))) != foreign_imp ||
            [foreign applicationShouldTerminate:nil] != NSTerminateNow) return false;
        return true;
    }
}
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
