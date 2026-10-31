// Audited AppKit adapter for "open document" requests (Finder double-click,
// Open With, dropping a .tcd on the Dock icon). AppKit's default handler is
// replaced at NSApplicationWillFinishLaunching, as Apple documents, so launch
// and running requests both arrive here. Only file-system paths are queued;
// the Rust application opens them through its ordinary checked open path.
#include "native_window.hpp"
#include "multicad-ui/src/native_window.rs.h"
#import <AppKit/AppKit.h>
#include <mutex>
#include <string>
#include <vector>

namespace {
std::mutex open_mutex;
std::vector<std::string> open_queue;
constexpr std::size_t max_queued = 64;
void queue_descriptor(NSAppleEventDescriptor* item) {
    NSAppleEventDescriptor* url = [item coerceToDescriptorType:typeFileURL];
    if (!url) return;
    NSString* text = [[NSString alloc] initWithData:url.data encoding:NSUTF8StringEncoding];
    NSURL* file = text ? [NSURL URLWithString:text] : nil;
    if (!file.isFileURL) return;
    const char* path = file.fileSystemRepresentation;
    if (!path || !*path) return;
    std::lock_guard<std::mutex> lock(open_mutex);
    if (open_queue.size() < max_queued) open_queue.emplace_back(path);
}
}

@interface MultiCADOpenDocuments : NSObject
- (void)willFinishLaunching:(NSNotification*)notification;
- (void)openDocuments:(NSAppleEventDescriptor*)event withReply:(NSAppleEventDescriptor*)reply;
@end
@implementation MultiCADOpenDocuments
- (void)willFinishLaunching:(NSNotification*)notification {
    (void)notification;
    [[NSAppleEventManager sharedAppleEventManager]
        setEventHandler:self
            andSelector:@selector(openDocuments:withReply:)
          forEventClass:kCoreEventClass
             andEventID:kAEOpenDocuments];
}
- (void)openDocuments:(NSAppleEventDescriptor*)event withReply:(NSAppleEventDescriptor*)reply {
    (void)reply;
    NSAppleEventDescriptor* direct = [event paramDescriptorForKeyword:keyDirectObject];
    if (!direct) return;
    if (direct.descriptorType == typeAEList) {
        for (NSInteger i = 1; i <= direct.numberOfItems; ++i)
            queue_descriptor([direct descriptorAtIndex:i]);
    } else {
        queue_descriptor(direct);
    }
    multicad::window::wake_for_opened_documents();
}
@end

namespace {
MultiCADOpenDocuments* open_handler = nil;
}

namespace multicad::window {
void observe_open_documents() noexcept {
    if (![NSThread isMainThread] || open_handler) return;
    open_handler = [MultiCADOpenDocuments new];
    [[NSNotificationCenter defaultCenter] addObserver:open_handler
                                             selector:@selector(willFinishLaunching:)
                                                 name:NSApplicationWillFinishLaunchingNotification
                                               object:nil];
}
rust::Vec<rust::String> take_opened_documents() noexcept {
    std::vector<std::string> taken;
    {
        std::lock_guard<std::mutex> lock(open_mutex);
        taken.swap(open_queue);
    }
    rust::Vec<rust::String> result;
    for (const auto& path : taken) {
        try {
            result.push_back(rust::String(path));
        } catch (...) {
            // Not UTF-8: no ToskCAD path can name it; skip rather than abort.
        }
    }
    return result;
}
// Builds a real kAEOpenDocuments event, as Finder sends it, and runs it through
// the handler. No NSApplication or window is created.
bool open_documents_contract_test() noexcept {
    @autoreleasepool {
        {
            std::lock_guard<std::mutex> lock(open_mutex);
            open_queue.clear();
        }
        NSAppleEventDescriptor* list = [NSAppleEventDescriptor listDescriptor];
        NSArray<NSString*>* paths = @[@"/private/tmp/ToskCAD probe/Upper.TCD",
                                      @"/private/tmp/legacy.multicad"];
        for (NSString* path in paths) {
            NSData* data = [[NSURL fileURLWithPath:path].absoluteString
                dataUsingEncoding:NSUTF8StringEncoding];
            [list insertDescriptor:[NSAppleEventDescriptor descriptorWithDescriptorType:typeFileURL
                                                                                   data:data]
                           atIndex:0];
        }
        // A non-file URL in the list is ignored, never queued.
        NSData* web = [@"https://example.invalid/x.tcd" dataUsingEncoding:NSUTF8StringEncoding];
        [list insertDescriptor:[NSAppleEventDescriptor descriptorWithDescriptorType:typeFileURL
                                                                               data:web]
                       atIndex:0];
        NSAppleEventDescriptor* event = [NSAppleEventDescriptor
            appleEventWithEventClass:kCoreEventClass
                             eventID:kAEOpenDocuments
                    targetDescriptor:nil
                            returnID:kAutoGenerateReturnID
                       transactionID:kAnyTransactionID];
        [event setParamDescriptor:list forKeyword:keyDirectObject];
        MultiCADOpenDocuments* handler = [MultiCADOpenDocuments new];
        [handler openDocuments:event withReply:[NSAppleEventDescriptor nullDescriptor]];
        rust::Vec<rust::String> taken = take_opened_documents();
        if (taken.size() != 2) return false;
        if (std::string(taken[0]) != "/private/tmp/ToskCAD probe/Upper.TCD") return false;
        if (std::string(taken[1]) != "/private/tmp/legacy.multicad") return false;
        return take_opened_documents().empty();
    }
}
}
