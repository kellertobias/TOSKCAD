#include "native_window.hpp"
#include "multicad-ui/src/native_window.rs.h"
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <cmath>
#include <algorithm>

namespace {
char quit_guard_state_key;
}

namespace {
char touch_state_key;
char touch_originals_key;
}
@interface MultiCADTouchState : NSObject
@property(nonatomic, copy) NSSet* identities;
@property(nonatomic) NSPoint centroid;
@property(nonatomic) NSPoint origin;
@property(nonatomic) NSPoint pending;
@property(nonatomic) std::uint64_t sequence;
@property(nonatomic) BOOL engaged;
@end
@implementation MultiCADTouchState
@end

namespace {
void observe_contacts(MultiCADTouchState* state, NSSet* identities, NSPoint centroid,
                      NSPoint cursor) {
    if (identities.count != 3 || !std::isfinite(centroid.x) || !std::isfinite(centroid.y)) {
        state.identities = nil;
        return;
    }
    if (![identities isEqualToSet:state.identities]) {
        state.sequence += 1;
        state.origin = cursor;
        state.pending = NSZeroPoint;
    } else {
        const NSPoint delta = NSMakePoint(centroid.x-state.centroid.x,
                                         state.centroid.y-centroid.y);
        state.pending = NSMakePoint(std::clamp(state.pending.x+delta.x, -2000., 2000.),
                                    std::clamp(state.pending.y+delta.y, -2000., 2000.));
    }
    state.identities = identities;
    state.centroid = centroid;
    state.engaged = YES;
}
}

@interface MultiCADTouchObserver : NSObject
- (void)touchesBeganWithEvent:(NSEvent*)event;
- (void)touchesMovedWithEvent:(NSEvent*)event;
- (void)touchesEndedWithEvent:(NSEvent*)event;
- (void)touchesCancelledWithEvent:(NSEvent*)event;
@end
@implementation MultiCADTouchObserver
- (void)touchesBeganWithEvent:(NSEvent*)event {
    // This IMP runs on the unchanged WinitView. Access associated state only,
    // never helper ivars; retain the host's inherited responder callback.
    NSView* view = (NSView*)self;
    MultiCADTouchState* state = objc_getAssociatedObject(self, &touch_state_key);
    if (state && [NSThread isMainThread]) {
        NSMutableSet* identities = [NSMutableSet new];
        NSPoint centroid = NSZeroPoint;
        for (NSTouch* touch in [event touchesMatchingPhase:NSTouchPhaseTouching inView:view]) {
            if (touch.type != NSTouchTypeIndirect || touch.isResting) continue;
            [identities addObject:touch.identity];
            centroid.x += touch.normalizedPosition.x * touch.deviceSize.width;
            centroid.y += touch.normalizedPosition.y * touch.deviceSize.height;
        }
        if (identities.count) {
            centroid.x /= identities.count;
            centroid.y /= identities.count;
        }
        NSPoint cursor = [view convertPoint:event.locationInWindow fromView:nil];
        if (!view.isFlipped) cursor.y = NSMaxY(view.bounds) - cursor.y;
        observe_contacts(state, identities, centroid, cursor);
        // Winit's drawRect queues its ordinary redraw; no event loop replacement.
        [view setNeedsDisplay:YES];
    }
    NSDictionary* originals = objc_getAssociatedObject(object_getClass(self), &touch_originals_key);
    NSValue* original = originals[NSStringFromSelector(_cmd)];
    if (original) {
        using Callback = void (*)(id, SEL, NSEvent*);
        reinterpret_cast<Callback>(original.pointerValue)(self, _cmd, event);
    }
}
- (void)touchesMovedWithEvent:(NSEvent*)event { [self touchesBeganWithEvent:event]; }
- (void)touchesEndedWithEvent:(NSEvent*)event { [self touchesBeganWithEvent:event]; }
- (void)touchesCancelledWithEvent:(NSEvent*)event { [self touchesBeganWithEvent:event]; }
@end

namespace {
bool owns_selector(Class cls, SEL selector) {
    unsigned int count = 0;
    Method* methods = class_copyMethodList(cls, &count);
    bool found = false;
    for (unsigned int i=0; i<count; ++i) found |= method_getName(methods[i]) == selector;
    free(methods);
    return found;
}
bool install_touch_callbacks(Class cls) {
    SEL selectors[] = { @selector(touchesBeganWithEvent:), @selector(touchesMovedWithEvent:),
        @selector(touchesEndedWithEvent:), @selector(touchesCancelledWithEvent:) };
    // Use one implementation with the actual selector, so forwarding preserves
    // Began/Moved/Ended/Cancelled separately. Never overwrite a host-owned IMP.
    Method helper = class_getInstanceMethod([MultiCADTouchObserver class], selectors[0]);
    IMP implementation = method_getImplementation(helper);
    NSDictionary* previous = objc_getAssociatedObject(cls, &touch_originals_key);
    const bool installed = [previous[@"installed"] boolValue];
    if (previous && !installed) return false;
    for (SEL selector : selectors) {
        Method existing = class_getInstanceMethod(cls, selector);
        if (owns_selector(cls, selector) && method_getImplementation(existing) != implementation)
            return false;
        // A different subclass inheriting our callback must not record that
        // helper as its own original IMP (which would recurse during forwarding).
        if (!installed && existing && method_getImplementation(existing) == implementation)
            return false;
    }
    if (installed) return true;
    NSMutableDictionary* originals = [NSMutableDictionary new];
    for (SEL selector : selectors) {
        Method inherited = class_getInstanceMethod(cls, selector);
        if (inherited) originals[NSStringFromSelector(selector)] =
            [NSValue valueWithPointer:reinterpret_cast<void*>(method_getImplementation(inherited))];
    }
    objc_setAssociatedObject(cls, &touch_originals_key, originals, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    for (SEL selector : selectors) {
        if (!class_addMethod(cls, selector, implementation, method_getTypeEncoding(helper))) return false;
    }
    originals[@"installed"] = @YES;
    return true;
}
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
        NSEvent* event = application.currentEvent;
        if (event.type == NSEventTypeKeyDown &&
            (event.modifierFlags & NSEventModifierFlagCommand) != 0 &&
            [event.charactersIgnoringModifiers.lowercaseString isEqualToString:@"q"] &&
            std::isfinite(event.timestamp) && event.timestamp >= 0) {
            multicad::window::observed_quit_shortcut(
                reinterpret_cast<std::size_t>((__bridge void*)window),
                static_cast<std::uint64_t>(event.timestamp * 1000000.0),
                event.isARepeat);
        }
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

@interface MultiCADTouchProbeBase : NSObject
@property(nonatomic) NSInteger callbacks;
- (void)touchesMovedWithEvent:(NSEvent*)event;
@end
@implementation MultiCADTouchProbeBase
- (void)touchesMovedWithEvent:(NSEvent*)event {
    (void)event;
    self.callbacks += 1;
}
@end
@interface MultiCADTouchProbe : MultiCADTouchProbeBase
@end
@implementation MultiCADTouchProbe
@end
@interface MultiCADDerivedTouchProbe : MultiCADTouchProbe
@end
@implementation MultiCADDerivedTouchProbe
@end
@interface MultiCADForeignTouchProbe : NSObject
- (void)touchesMovedWithEvent:(NSEvent*)event;
@end
@implementation MultiCADForeignTouchProbe
- (void)touchesMovedWithEvent:(NSEvent*)event { (void)event; }
@end

namespace multicad::window {
NativeTouchFrame poll_trackpad(std::size_t address) noexcept {
    NativeTouchFrame result{};
    if (![NSThread isMainThread] || !address) return result;
    NSView* view = (__bridge NSView*)reinterpret_cast<void*>(address);
    if (!view.window || !install_touch_callbacks(object_getClass(view))) return result;
    MultiCADTouchState* state = objc_getAssociatedObject(view, &touch_state_key);
    if (!state) {
        state = [MultiCADTouchState new];
        objc_setAssociatedObject(view, &touch_state_key, state, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        view.allowedTouchTypes |= NSTouchTypeMaskIndirect;
        view.wantsRestingTouches = NO;
    }
    result.available = true;
    if (!view.window.isKeyWindow) {
        state.identities = nil;
        state.pending = NSZeroPoint;
        state.engaged = NO;
        return result;
    }
    result.delta_x = state.pending.x;
    result.delta_y = state.pending.y;
    result.origin_x = state.origin.x;
    result.origin_y = state.origin.y;
    result.sequence = state.sequence;
    result.active = state.engaged || state.identities.count == 3;
    state.pending = NSZeroPoint;
    state.engaged = NO;
    return result;
}
bool trackpad_contract_test() noexcept {
    @autoreleasepool {
        MultiCADTouchState* state = [MultiCADTouchState new];
        NSSet* three = [NSSet setWithArray:@[@1,@2,@3]];
        observe_contacts(state, three, NSMakePoint(10,20), NSMakePoint(100,200));
        if (state.sequence != 1 || state.pending.x != 0 || state.pending.y != 0) return false;
        observe_contacts(state, three, NSMakePoint(15,17), NSZeroPoint);
        if (state.pending.x != 5 || state.pending.y != 3 || state.origin.x != 100) return false;
        observe_contacts(state, [NSSet setWithArray:@[@1,@2]], NSZeroPoint, NSZeroPoint);
        if (state.identities || !state.engaged || state.pending.x != 5) return false;
        observe_contacts(state, [NSSet setWithArray:@[@1,@2,@4]], NSMakePoint(500,500), NSZeroPoint);
        if (state.sequence != 2 || state.pending.x != 0 || state.pending.y != 0) return false;
        observe_contacts(state, three, NSMakePoint(NAN,0), NSZeroPoint);
        if (state.identities) return false;
        MultiCADTouchProbe* probe = [MultiCADTouchProbe new];
        Class cls = object_getClass(probe);
        if (!install_touch_callbacks(cls) || !install_touch_callbacks(cls) || object_getClass(probe) != cls) return false;
        [probe touchesMovedWithEvent:nil];
        if (probe.callbacks != 1) return false;
        if (install_touch_callbacks([MultiCADDerivedTouchProbe class])) return false;
        Class foreign = [MultiCADForeignTouchProbe class];
        IMP before = method_getImplementation(class_getInstanceMethod(foreign, @selector(touchesMovedWithEvent:)));
        if (install_touch_callbacks(foreign) || before != method_getImplementation(class_getInstanceMethod(foreign, @selector(touchesMovedWithEvent:)))) return false;
        return true;
    }
}
std::uint8_t install_quit_guard(std::size_t address) noexcept {
    if (![NSThread isMainThread]) return 1;
    if (!address) return 2;
    NSView* view = (__bridge NSView*)reinterpret_cast<void*>(address);
    NSWindow* window = view.window;
    if (!window) return 2;
    return install_on_delegate(NSApp.delegate, window);
}
std::size_t window_identity(std::size_t address) noexcept {
    if (![NSThread isMainThread] || !address) return 0;
    NSView* view = (__bridge NSView*)reinterpret_cast<void*>(address);
    return reinterpret_cast<std::size_t>((__bridge void*)view.window);
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
