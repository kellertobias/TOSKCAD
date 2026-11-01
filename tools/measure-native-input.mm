// Developer-only, single-window macOS input-to-present probe (CAD-243).
// Compile as a dylib and inject only into an isolated, ad-hoc signed QA copy.
// It preserves event delivery and CAMetalLayer::nextDrawable, recording no
// key contents. Normal application builds neither link nor load this helper.
#import <AppKit/AppKit.h>
#import <QuartzCore/CAMetalLayer.h>
#import <Metal/Metal.h>
#import <objc/runtime.h>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <vector>

namespace {
struct Input {
    unsigned long long sequence;
    unsigned long kind;
    double occurred;
    double delivered;
};
struct Probe {
    std::mutex mutex;
    std::vector<Input> pending;
    unsigned long long sequence = 0;
    unsigned long long frames = 0;
    FILE* file = nullptr;
    dispatch_queue_t writer;
    __strong id monitor;
    IMP original = nullptr;
};
Probe& probe() {
    // Presentation callbacks may outlive AppKit teardown. Keep callback state
    // alive until process exit, rather than destroying it before Metal stops.
    static Probe* value = new Probe;
    return *value;
}
void record_input(Input input) {
    dispatch_async(probe().writer, ^{
        std::fprintf(probe().file,
            "{\"phase\":\"input\",\"sequence\":%llu,\"kind\":%lu,"
            "\"occurred\":%.9f,\"delivered\":%.9f}\n",
            input.sequence, input.kind, input.occurred, input.delivered);
        std::fflush(probe().file);
    });
}
id<CAMetalDrawable> next_drawable(id layer, SEL selector) {
    using Original = id<CAMetalDrawable> (*)(id, SEL);
    id<CAMetalDrawable> drawable =
        reinterpret_cast<Original>(probe().original)(layer, selector);
    if (!drawable) return drawable;
    std::vector<Input> inputs;
    unsigned long long frame;
    {
        std::lock_guard lock(probe().mutex);
        inputs.swap(probe().pending);
        frame = ++probe().frames;
    }
    const double acquired = CACurrentMediaTime();
    if (!inputs.empty()) dispatch_async(probe().writer, ^{
        std::fprintf(probe().file,
            "{\"phase\":\"acquired\",\"frame\":%llu,\"count\":%zu,\"time\":%.9f}\n",
            frame, inputs.size(), acquired);
        std::fflush(probe().file);
    });
    [drawable addPresentedHandler:^(id<MTLDrawable> presented) {
        const double time = presented.presentedTime;
        dispatch_async(probe().writer, ^{
            std::fprintf(probe().file,
                "{\"phase\":\"frame-presented\",\"frame\":%llu,\"acquired\":%.9f,\"presented\":%.9f}\n",
                frame, acquired, time);
            for (const Input& input : inputs) {
                std::fprintf(probe().file,
                    "{\"phase\":\"presented\",\"frame\":%llu,\"sequence\":%llu,"
                    "\"kind\":%lu,\"occurred\":%.9f,\"delivered\":%.9f,"
                    "\"acquired\":%.9f,\"presented\":%.9f}\n",
                    frame, input.sequence, input.kind, input.occurred,
                    input.delivered, acquired, time);
            }
            std::fflush(probe().file);
        });
    }];
    return drawable;
}
bool relevant(NSEventType type) {
    switch (type) {
        case NSEventTypeMouseMoved:
        case NSEventTypeLeftMouseDown:
        case NSEventTypeLeftMouseUp:
        case NSEventTypeLeftMouseDragged:
        case NSEventTypeRightMouseDown:
        case NSEventTypeRightMouseUp:
        case NSEventTypeKeyDown:
        case NSEventTypeKeyUp:
        case NSEventTypeScrollWheel: return true;
        default: return false;
    }
}
void finish() {
    if (!probe().file) return;
    // Flush records already handed to the writer. Unpresented input records
    // remain explicit in the log and must not silently disappear in analysis.
    dispatch_sync(probe().writer, ^{
        std::fprintf(probe().file, "{\"phase\":\"exit\"}\n");
        std::fflush(probe().file);
    });
}
__attribute__((constructor)) void install() {
    const char* path = std::getenv("MULTICAD_INPUT_TRACE");
    if (!path || !*path) return;
    @autoreleasepool {
        probe().file = std::fopen(path, "wx");
        if (!probe().file) return;
        probe().writer = dispatch_queue_create("multicad.input-trace", DISPATCH_QUEUE_SERIAL);
        const double clock_delta = CACurrentMediaTime() - NSProcessInfo.processInfo.systemUptime;
        std::fprintf(probe().file,
            "{\"phase\":\"start\",\"version\":3,\"clock_delta_seconds\":%.9f,"
            "\"scope\":\"single-window test process; first Metal drawable after delivered inputs\"}\n",
            clock_delta);
        std::fflush(probe().file);
        Method method = class_getInstanceMethod([CAMetalLayer class], @selector(nextDrawable));
        if (!method) {
            std::fprintf(probe().file, "{\"phase\":\"unsupported\",\"reason\":\"no nextDrawable method\"}\n");
            std::fflush(probe().file);
            return;
        }
        probe().original = method_setImplementation(method, reinterpret_cast<IMP>(next_drawable));
        std::atexit(finish);
        dispatch_async(dispatch_get_main_queue(), ^{
            // Retain the monitor; every event is returned unchanged. Limit the
            // observation to this process's key window and bounded pending input.
            probe().monitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskAny
                handler:^NSEvent* (NSEvent* event) {
                    if (!relevant(event.type) || !event.window ||
                        event.window != NSApp.keyWindow) return event;
                    Input input;
                    bool overflow;
                    {
                        std::lock_guard lock(probe().mutex);
                        input = {++probe().sequence, static_cast<unsigned long>(event.type),
                            event.timestamp, CACurrentMediaTime()};
                        overflow = probe().pending.size() >= 4096;
                        if (!overflow) probe().pending.push_back(input);
                    }
                    record_input(input);
                    if (overflow) {
                        dispatch_async(probe().writer, ^{
                            std::fprintf(probe().file,
                                "{\"phase\":\"overflow\",\"sequence\":%llu}\n", input.sequence);
                            std::fflush(probe().file);
                        });
                    }
                    return event;
                }];
        });
    }
}
}
