#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <ImageIO/ImageIO.h>
#import "watch.h"

// Called on the serialized Rust worker. Each entry retains the AX identity, not a title or list index.
@interface CPDrawerEntry : NSObject
@property(nonatomic,strong) id element;
@property(nonatomic,copy) NSString *name, *title;
@property CGRect original;
@property BOOL attached;
@property BOOL followingResize;
@property NSTimeInterval lastOrderAttempt;
@property CGSize requestedSize, acceptedSize;
@property BOOL hasParkingPoint;
@property CGPoint parkingPoint;
@property pid_t pid;
@property NSUInteger lastUse;
@property CGWindowID windowID;
@property(nonatomic,copy) NSString *presentationState;
@end
@implementation CPDrawerEntry
@end

static NSMutableDictionary<NSString *, CPDrawerEntry *> *entries;
static NSString *current;
static CGRect lastFrame, lastActual;

// AX calls to our own Tauri window enter AppKit directly. Remote AX remains
// on the worker. A bounded wait lets shutdown finish if the UI loop has exited.
static BOOL onAXOwner(AXUIElementRef el, void (^work)(void)) {
    pid_t pid = 0;
    if (NSThread.isMainThread || AXUIElementGetPid(el,&pid) || pid != getpid()) { work(); return YES; }
    dispatch_block_t task = dispatch_block_create(0,work);
    dispatch_async(dispatch_get_main_queue(),task);
    if (!dispatch_block_wait(task,dispatch_time(DISPATCH_TIME_NOW,250*NSEC_PER_MSEC))) return YES;
    dispatch_block_cancel(task);
    return NO;
}

static id attribute(AXUIElementRef element, CFStringRef key) {
    id owner = (__bridge id)element;
    __block id result = nil;
    BOOL ok = onAXOwner(element,^{
        CFTypeRef value = NULL;
        AXUIElementCopyAttributeValue((__bridge AXUIElementRef)owner,key,&value);
        result = CFBridgingRelease(value);
    });
    return ok ? result : nil;
}
static AXUIElementRef elementOf(CPDrawerEntry *entry) { return (__bridge AXUIElementRef)entry.element; }
static BOOL getFrame(CPDrawerEntry *entry, CGRect *frame) {
    id p = attribute(elementOf(entry), kAXPositionAttribute);
    id s = attribute(elementOf(entry), kAXSizeAttribute);
    return p && s && CFGetTypeID((__bridge CFTypeRef)p) == AXValueGetTypeID()
        && CFGetTypeID((__bridge CFTypeRef)s) == AXValueGetTypeID()
        && AXValueGetValue((__bridge AXValueRef)p,kAXValueCGPointType,&frame->origin)
        && AXValueGetValue((__bridge AXValueRef)s,kAXValueCGSizeType,&frame->size);
}
static BOOL writeValue(CPDrawerEntry *entry, CFStringRef key, AXValueType type, const void *data) {
    AXValueRef value = AXValueCreate(type,data);
    if (!value) return NO;
    id payload = CFBridgingRelease(value);
    __block AXError result;
    return onAXOwner(elementOf(entry),^{
        result = AXUIElementSetAttributeValue(elementOf(entry),key,(__bridge CFTypeRef)payload);
    }) && result == kAXErrorSuccess;
}
static BOOL place(CPDrawerEntry *entry, CGRect rect) {
    CGRect old;
    if (!getFrame(entry,&old)) return NO;
    if (!CGSizeEqualToSize(old.size,rect.size))
        writeValue(entry,kAXSizeAttribute,kAXValueCGSizeType,&rect.size);
    writeValue(entry,kAXPositionAttribute,kAXValueCGPointType,&rect.origin);
    CGRect actual;
    return getFrame(entry,&actual) && fabs(actual.origin.x-rect.origin.x)<=1 && fabs(actual.origin.y-rect.origin.y)<=1
        && fabs(actual.size.width-rect.size.width)<=1 && fabs(actual.size.height-rect.size.height)<=1;
}
static BOOL needsMoreSpace(CGSize measured, CGSize slot) {
    return measured.width > slot.width + 1 || measured.height > slot.height + 1;
}
static BOOL alignedWithSlot(CGRect measured, CGRect slot) {
    return fabs(measured.origin.x-slot.origin.x)<=2 && fabs(measured.origin.y-slot.origin.y)<=2;
}
static BOOL acceptsMeasurement(CGRect measured, CGRect slot, BOOL knownSize, BOOL finalRead) {
    // A minimum-size window can be clamped to the work area before the
    // pet has recentered its larger panel. Negotiate size before alignment.
    if (finalRead && needsMoreSpace(measured.size,slot.size)) return YES;
    return alignedWithSlot(measured,slot) && (knownSize || CGSizeEqualToSize(measured.size,slot.size) || finalRead);
}
// AX success acknowledges the request. The owner may apply it later, snap
// to a character grid, or enforce an intrinsic minimum.
static BOOL fitWindow(CPDrawerEntry *entry, CGRect slot, CGRect *actual) {
    CGRect before;
    if (!getFrame(entry, &before)) return NO;
    BOOL negotiated = CGSizeEqualToSize(slot.size, entry.requestedSize) &&
                      CGSizeEqualToSize(before.size, entry.acceptedSize);
    BOOL resize = !negotiated && !CGSizeEqualToSize(before.size,slot.size);
    BOOL wroteSize = !resize || writeValue(entry,kAXSizeAttribute,kAXValueCGSizeType,&slot.size);
    if (resize || !CGPointEqualToPoint(before.origin,slot.origin))
        writeValue(entry,kAXPositionAttribute,kAXValueCGPointType,&slot.origin);
    for (int attempt = 0; attempt < 4; attempt++) {
        if (attempt) [NSThread sleepForTimeInterval:0.035];
        if (!getFrame(entry, actual) || CGRectIsEmpty(*actual)) continue;
        if (acceptsMeasurement(*actual,slot,negotiated,attempt == 3)) {
            if (wroteSize) { entry.requestedSize = slot.size; entry.acceptedSize = actual->size; }
            return YES;
        }
        if (attempt == 1 && !alignedWithSlot(*actual,slot) && !needsMoreSpace(actual->size,slot.size))
            writeValue(entry,kAXPositionAttribute,kAXValueCGPointType,&slot.origin);
    }
    return NO;
}
static BOOL gone(CPDrawerEntry *entry) {
    __block AXError error;
    return onAXOwner(elementOf(entry), ^{
        CFTypeRef role = NULL;
        error = AXUIElementCopyAttributeValue(elementOf(entry), kAXRoleAttribute, &role);
        if (role) CFRelease(role);
    }) && error == kAXErrorInvalidUIElement;
}
static BOOL restore(CPDrawerEntry *entry) {
    if (!entry.attached || place(entry,entry.original) || gone(entry)) {
        entry.attached = NO;
        return YES;
    }
    return NO;
}
// A normal AppKit window may retain a strip of its title bar even after AX
// reports success. Accept one point at a bottom corner of just one display.
static BOOL isParked(CGRect window, const CGRect *screens, uint32_t count) {
    if (!count || CGRectIsEmpty(window)) return NO;
    BOOL hasStrip = NO;
    for (uint32_t i = 0; i < count; i++) {
        CGRect screen = screens[i];
        CGRect visible = CGRectIntersection(window, screen);
        if (CGRectIsEmpty(visible)) continue;
        BOOL side = CGRectGetMinX(visible) == CGRectGetMinX(screen) ||
                    CGRectGetMaxX(visible) == CGRectGetMaxX(screen);
        if (hasStrip || !side || CGRectGetMaxY(visible) != CGRectGetMaxY(screen) ||
            visible.size.width > 1 || visible.size.height > 64) return NO;
        hasStrip = YES;
    }
    return YES;
}

static BOOL tryParking(CPDrawerEntry *entry, CGPoint target, const CGRect *screens, uint32_t count) {
    if (!writeValue(entry, kAXPositionAttribute, kAXValueCGPointType, &target)) return NO;
    CGRect actual;
    if (getFrame(entry, &actual) && isParked(actual, screens, count)) return YES;
    // Some apps apply the position on their next event loop iteration.
    [NSThread sleepForTimeInterval:0.025];
    return getFrame(entry, &actual) && isParked(actual, screens, count);
}

static BOOL park(CPDrawerEntry *entry) {
    CGRect before;
    uint32_t count = 0;
    if (!getFrame(entry, &before) || CGGetActiveDisplayList(0, NULL, &count) || !count) return NO;
    NSMutableData *storage = [NSMutableData dataWithLength:sizeof(CGDirectDisplayID) * count];
    CGDirectDisplayID *ids = storage.mutableBytes;
    if (CGGetActiveDisplayList(count, ids, &count) || !count) return NO;
    NSMutableData *frames = [NSMutableData dataWithLength:sizeof(CGRect) * count];
    CGRect *screens = frames.mutableBytes;
    CGRect bounds = CGRectNull;
    for (uint32_t i = 0; i < count; i++) {
        screens[i] = CGDisplayBounds(ids[i]);
        bounds = CGRectUnion(bounds, screens[i]);
    }
    if (entry.hasParkingPoint && isParked((CGRect){entry.parkingPoint,before.size}, screens, count) &&
        tryParking(entry,entry.parkingPoint,screens,count)) return YES;
    entry.hasParkingPoint = NO;
    // Prefer a completely invisible window on apps that permit it.
    CGPoint outside = CGPointMake(CGRectGetMaxX(bounds) + 16, CGRectGetMaxY(bounds) + 16);
    if (tryParking(entry, outside, screens, count)) return YES;
    for (uint32_t i = 0; i < count; i++) {
        CGRect screen = screens[i];
        CGFloat y = CGRectGetMaxY(screen) - 1;
        CGPoint corners[] = {
            CGPointMake(CGRectGetMaxX(screen) - 1, y),
            CGPointMake(CGRectGetMinX(screen) - before.size.width + 1, y)
        };
        for (size_t j = 0; j < 2; j++) {
            CGRect requested = (CGRect){corners[j], before.size};
            if (isParked(requested, screens, count) &&
                tryParking(entry, corners[j], screens, count)) {
                entry.hasParkingPoint = YES;
                entry.parkingPoint = corners[j];
                return YES;
            }
        }
    }
    place(entry, before);
    return NO;
}
// Return a small PNG, shared across the windows of this app.
static NSString *iconURL(NSRunningApplication *app) {
    NSData *data = app.icon.TIFFRepresentation;
    if (!data) return @"";
    CGImageSourceRef source = CGImageSourceCreateWithData((__bridge CFDataRef)data, NULL);
    if (!source) return @"";
    NSDictionary *options = @{(__bridge NSString *)kCGImageSourceCreateThumbnailFromImageAlways:@YES,
                              (__bridge NSString *)kCGImageSourceThumbnailMaxPixelSize:@48};
    CGImageRef thumb = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    if (!thumb) return @"";
    NSBitmapImageRep *image = [[NSBitmapImageRep alloc] initWithCGImage:thumb];
    CGImageRelease(thumb);
    NSData *png = [image representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    return png ? [@"data:image/png;base64," stringByAppendingString:[png base64EncodedStringWithOptions:0]] : @"";
}

// Captured by AppKit at mouse-down, before IPC or an AX query can lose the
// source to the moving window or the pet panel. Accessed by the drawer worker.
typedef struct {
    CGWindowID number;
    pid_t pid;
    CGRect original;
    CGPoint origin;
    CGPoint release;
    BOOL began, released, pet, detached;
    uint64_t sequence;
} CPDrawerPress;
static CPDrawerPress lastPress;
static uint64_t pressSequence; // All writers run on the main thread.
// Both AppKit event monitors and physical sampling run on the main thread.
// Keep the release available even after the worker consumes lastPress.
static uint32_t releaseSerial;
static CGPoint releasePoint;
static NSUInteger useCounter;

static CGPoint mousePoint(void) {
    CGEventRef event = CGEventCreate(NULL);
    CGPoint point = CGEventGetLocation(event);
    CFRelease(event);
    return point;
}

static id hitAt(CGPoint point) {
    AXUIElementRef system = AXUIElementCreateSystemWide(), hit = NULL;
    AXUIElementSetMessagingTimeout(system, 0.15);
    AXUIElementCopyElementAtPosition(system, point.x, point.y, &hit);
    CFRelease(system);
    return CFBridgingRelease(hit);
}

static NSString *dockItemBundle(AXUIElementRef hit) {
    if (![attribute(hit, kAXSubroleAttribute) isEqual:@"AXApplicationDockItem"]) return nil;
    NSURL *url = attribute(hit, kAXURLAttribute);
    return [url isKindOfClass:NSURL.class] ? [NSBundle bundleWithURL:url].bundleIdentifier : nil;
}

static NSArray *windowStack(void) {
    return CFBridgingRelease(CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
}
static BOOL boundsOf(NSDictionary *info, CGRect *bounds) {
    return CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)info[(__bridge id)kCGWindowBounds], bounds);
}
static BOOL nearFrame(CGRect a, CGRect b) {
    return fabs(a.origin.x-b.origin.x)<=2 && fabs(a.origin.y-b.origin.y)<=2 &&
           fabs(a.size.width-b.size.width)<=2 && fabs(a.size.height-b.size.height)<=2;
}
static CGWindowID identifyWindow(NSArray *stack, pid_t pid, CGRect frame) {
    CGWindowID candidate = kCGNullWindowID;
    for (NSDictionary *info in stack) {
        CGRect bounds;
        if ([info[(__bridge id)kCGWindowOwnerPID] intValue] != pid ||
            !boundsOf(info, &bounds) || !nearFrame(bounds, frame)) continue;
        if (candidate) return kCGNullWindowID;
        candidate = [info[(__bridge id)kCGWindowNumber] unsignedIntValue];
    }
    return candidate;
}

// A frameless Qt QMainWindow can report AXDialog, even when it is the app's
// normal, movable, non-modal main window. Sheets and modal dialogs stay out.
static BOOL isDraggableWindow(id window) {
    AXUIElementRef w = (__bridge AXUIElementRef)window;
    AXUIElementSetMessagingTimeout(w, 0.15);
    if (![attribute(w, kAXRoleAttribute) isEqual:@"AXWindow"] ||
        [attribute(w, CFSTR("AXFullScreen")) boolValue] ||
        [attribute(w, kAXModalAttribute) boolValue] ||
        [attribute(w, kAXMinimizedAttribute) boolValue]) return NO;
    NSString *subrole = attribute(w, kAXSubroleAttribute);
    if ([subrole isEqual:@"AXStandardWindow"]) return YES;
    if (subrole && ![subrole isEqual:@"AXDialog"] && ![subrole isEqual:@"AXUnknown"]) return NO;
    __block BOOL movable = NO;
    return onAXOwner(w, ^{
        Boolean settable = false;
        movable = AXUIElementIsAttributeSettable((__bridge AXUIElementRef)window,
            kAXPositionAttribute, &settable) == kAXErrorSuccess && settable;
    }) && movable;
}

static NSDictionary *windowInfo(CGWindowID number) {
    if (!number) return nil;
    NSArray *rows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, number));
    return rows.firstObject;
}

static CPDrawerPress sourceAt(CGWindowID number, CGPoint point, BOOL pet) {
    CPDrawerPress press = {.origin = point, .pet = pet, .began = YES, .sequence = ++pressSequence};
    if (pet) return press;
    NSDictionary *info = windowInfo(number);
    if (!info) {
        // Some native drag loops omit a target number. Inspect compositor
        // metadata at the start of this gesture, never AX trees on hover.
        for (NSDictionary *row in windowStack()) {
            CGRect bounds;
            if ([row[(__bridge id)kCGWindowLayer] integerValue] == NSNormalWindowLevel &&
                boundsOf(row, &bounds) && CGRectContainsPoint(bounds, point)) {
                info = row;
                break;
            }
        }
    }
    if (info && boundsOf(info, &press.original) && CGRectContainsPoint(press.original, point)) {
        press.number = [info[(__bridge id)kCGWindowNumber] unsignedIntValue];
        press.pid = [info[(__bridge id)kCGWindowOwnerPID] intValue];
    }
    return press;
}

// AppKit may start a Tauri title-bar drag in its own tracking loop before
// either the local monitor or the WebView samples the initial press.
void cp_drawer_will_move(NSWindow *window) {
    if (!window || window.level != NSNormalWindowLevel ||
        !CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,kCGMouseButtonLeft)) return;
    CGWindowID number = (CGWindowID)window.windowNumber;
    @synchronized (CPDrawerEntry.class) {
        if (lastPress.number == number && !lastPress.released) return;
        CPDrawerPress press = sourceAt(number, mousePoint(), NO);
        if (press.pid == getpid()) lastPress = press;
    }
}

// Called under the press lock. A delayed mouseUp and its physical fallback
// refer to the same gesture, including when AppKit consumed the local mouseUp.
static void finishPress(CGPoint point) {
    if (!lastPress.began) return;
    if (!lastPress.pet) {
        if (!lastPress.released) releaseSerial++;
        releasePoint = point;
    }
    lastPress.release = point;
    lastPress.released = YES;
}

void cp_drawer_mouse_event(void *raw, bool pet) {
    NSEvent *event = (__bridge NSEvent *)raw;
    CGEventRef cg = event.CGEvent;
    CGPoint point = cg ? CGEventGetLocation(cg) : mousePoint();
    if (event.type == NSEventTypeLeftMouseUp) {
        @synchronized (CPDrawerEntry.class) { finishPress(point); }
    } else {
        // Local events identify the Tauri NSWindow which received the press.
        // Global events may only have WindowServer's hit-test window number.
        CGWindowID number = (CGWindowID)event.windowNumber;
        if ((!number || ![NSApp windowWithWindowNumber:number]) && cg) {
            CGWindowID hit = (CGWindowID)CGEventGetIntegerValueField(cg, kCGMouseEventWindowUnderMousePointerThatCanHandleThisEvent);
            if (hit) number = hit;
        }
        CPDrawerPress press = sourceAt(number, point, pet);
        @synchronized (CPDrawerEntry.class) { lastPress = press; }
        if (!pet) cp_drawer_other_window_pressed(press.number);
    }
}

// Inspect an outstanding monitored press even if the WebView missed the
// entire native tracking loop and never sampled the button while held.
uint32_t cp_drawer_sample_press(bool down, CGPoint *release) {
    static BOOL held;
    @synchronized (CPDrawerEntry.class) {
        if (down && !held && (!lastPress.began || lastPress.released)) {
            lastPress = sourceAt(0, mousePoint(), NO);
        } else if (!down && lastPress.began && !lastPress.released) {
            finishPress(mousePoint());
        }
        held = down;
        *release = releasePoint;
        return releaseSerial;
    }
}

static CPDrawerPress capturedPress(BOOL consume) {
    @synchronized (CPDrawerEntry.class) {
        CPDrawerPress press = lastPress;
        if (consume) lastPress = (CPDrawerPress){0};
        return press;
    }
}

static BOOL movedWindow(CPDrawerPress press, CGPoint end, CGRect *actual) {
    if (press.detached || (current && entries[current].windowID == press.number)) return NO;
    NSDictionary *info = windowInfo(press.number);
    if (!info || [info[(__bridge id)kCGWindowOwnerPID] intValue] != press.pid || !boundsOf(info, actual)) return NO;
    if (!CGSizeEqualToSize(press.original.size, actual->size) &&
        !CGRectContainsPoint(CGRectInset(press.original, 4, 4), press.origin)) return NO;
    return hypot(end.x - press.origin.x, end.y - press.origin.y) >= 6 &&
        hypot(actual->origin.x - press.original.origin.x, actual->origin.y - press.original.origin.y) >= 6 &&
        CGRectContainsPoint(CGRectInset(*actual, -2, -40), end);
}

// Resolve AX after release: a custom title bar can be inside WKWebView or
// Qt, and the owner may not answer AX while its native drag loop is active.
static CPDrawerEntry *entryForPress(CPDrawerPress press, CGRect frame) {
    AXUIElementRef app = AXUIElementCreateApplication(press.pid);
    AXUIElementSetMessagingTimeout(app, 0.25);
    NSArray *windows = attribute(app, kAXWindowsAttribute);
    CFRelease(app);
    CPDrawerEntry *match = nil;
    for (id window in windows) {
        CPDrawerEntry *entry = [CPDrawerEntry new];
        entry.element = window;
        CGRect actual;
        if (!getFrame(entry, &actual) || !nearFrame(actual, frame) || !isDraggableWindow(window)) continue;
        if (match) return nil; // Same owner, indistinguishable overlapping siblings.
        // The compositor may include the focus border outside the AX frame.
        // Restore in AX coordinates, using the measured frame in both spaces.
        entry.original = CGRectMake(press.original.origin.x + actual.origin.x - frame.origin.x,
                                    press.original.origin.y + actual.origin.y - frame.origin.y,
                                    press.original.size.width + actual.size.width - frame.size.width,
                                    press.original.size.height + actual.size.height - frame.size.height);
        match = entry;
    }
    match.pid = press.pid;
    match.windowID = press.number;
    match.name = [NSRunningApplication runningApplicationWithProcessIdentifier:press.pid].localizedName ?: @"App";
    return match;
}

static NSDictionary *itemFor(NSString *key, CPDrawerEntry *entry) {
    NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:entry.pid];
    return @{@"id":key, @"name":entry.name, @"title":entry.title, @"icon":iconURL(app)};
}

static id dockSelection(CGPoint point) {
    id hit = hitAt(point);
    NSString *bundle = hit ? dockItemBundle((__bridge AXUIElementRef)hit) : nil;
    if (!bundle) return NSNull.null;
    NSString *best = nil;
    for (NSString *key in entries) {
        CPDrawerEntry *item = entries[key];
        NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:item.pid];
        if (item.attached && [app.bundleIdentifier isEqual:bundle] &&
            (!best || item.lastUse > entries[best].lastUse)) best = key;
    }
    return best ? itemFor(best, entries[best]) : NSNull.null;
}

// A tracked press can fall back to the sampled physical button if AppKit
// missed mouse-up. A fast Dock click must use its recorded release point.
static BOOL takeDockClick(BOOL tracked, CGPoint *end) {
    CPDrawerPress press = capturedPress(YES);
    if (press.detached || (!press.released && !tracked)) return NO;
    *end = press.released ? press.release : mousePoint();
    return !tracked || hypot(end->x - press.origin.x, end->y - press.origin.y) < 6;
}

static id droppedWindow(void) {
    CPDrawerPress press = capturedPress(YES);
    CGPoint end = press.released ? press.release : mousePoint();
    CGRect actual;
    if (!press.number) return @{@"ignored":@"no-source-event"};
    if (!movedWindow(press, end, &actual)) return @{@"ignored":@"no-window-movement-at-drop"};

    CPDrawerEntry *entry = entryForPress(press, actual);
    if (!entry) return @{@"error":@"已检测到拖入，但无法通过辅助功能匹配该窗口。"};
    for (NSString *key in entries) {
        if (CFEqual(elementOf(entries[key]), elementOf(entry))) return itemFor(key, entries[key]);
    }
    NSString *key = NSUUID.UUID.UUIDString;
    entry.title = attribute(elementOf(entry), kAXTitleAttribute) ?: @"";
    entry.attached = YES; // Remember the frame at the beginning of the drag.
    entries[key] = entry;
    return itemFor(key, entry);
}

// Subtract foreground rectangles from the real window footprint. A utility
// panel covering one corner must not suppress an otherwise visible window.
static void addRect(NSMutableArray *rects, CGFloat left, CGFloat top, CGFloat right, CGFloat bottom) {
    if (right > left && bottom > top)
        [rects addObject:[NSValue valueWithRect:NSMakeRect(left, top, right-left, bottom-top)]];
}
static NSArray *subtractCover(NSArray *parts, CGRect cover) {
    NSMutableArray *next = [NSMutableArray new];
    for (NSValue *value in parts) {
        CGRect r = value.rectValue;
        CGRect cut = CGRectIntersection(r, cover);
        if (CGRectIsEmpty(cut)) { [next addObject:value]; continue; }
        addRect(next, CGRectGetMinX(r), CGRectGetMinY(r), CGRectGetMaxX(r), CGRectGetMinY(cut));
        addRect(next, CGRectGetMinX(r), CGRectGetMaxY(cut), CGRectGetMaxX(r), CGRectGetMaxY(r));
        addRect(next, CGRectGetMinX(r), CGRectGetMinY(cut), CGRectGetMinX(cut), CGRectGetMaxY(cut));
        addRect(next, CGRectGetMaxX(cut), CGRectGetMinY(cut), CGRectGetMaxX(r), CGRectGetMaxY(cut));
    }
    return next;
}

// WindowServer alpha describes the whole surface, including for cursor
// highlights whose backing is transparent almost everywhere. Leave those
// pixels to the compositor. Other applications' floating palettes still mask.
static BOOL isCursorOverlay(NSInteger layer, NSString *bundle) {
    return layer == NSFloatingWindowLevel && [bundle isEqual:@"com.openai.sky.CUAService"];
}

static NSArray *visibleParts(NSArray *stack, CPDrawerEntry *entry, CGRect actual, CGRect slot, NSDictionary **firstCover) {
    if (firstCover) *firstCover = nil;
    if (!entry.windowID) entry.windowID = identifyWindow(stack, entry.pid, actual);
    CGWindowID host = cp_drawer_host_number();
    NSMutableArray *covers = [NSMutableArray new];
    for (NSDictionary *info in stack) {
        pid_t pid = [info[(__bridge id)kCGWindowOwnerPID] intValue];
        CGRect bounds;
        if (!boundsOf(info,&bounds) || CGRectIsEmpty(CGRectIntersection(bounds,slot))) continue;
        NSNumber *alpha = info[(__bridge id)kCGWindowAlpha];
        if (alpha && alpha.doubleValue <= 0) continue;
        CGWindowID number = [info[(__bridge id)kCGWindowNumber] unsignedIntValue];
        if (pid == entry.pid && entry.windowID && number == entry.windowID) {
            CGRect footprint = CGRectIntersection(CGRectIntersection(slot, actual), bounds);
            if (CGRectIsEmpty(footprint)) return nil;
            NSArray *parts = @[[NSValue valueWithRect:footprint]];
            for (NSValue *cover in covers) parts = subtractCover(parts, cover.rectValue);
            NSMutableArray *relative = [NSMutableArray new];
            for (NSValue *part in parts) {
                CGRect r = part.rectValue;
                [relative addObject:@{@"x":@(r.origin.x-slot.origin.x), @"y":@(r.origin.y-slot.origin.y),
                                      @"width":@(r.size.width), @"height":@(r.size.height)}];
            }
            return relative;
        }
        NSInteger layer = [info[(__bridge id)kCGWindowLayer] integerValue];
        if (host && pid == getpid() && number == host) {
            // Surfaces above the pet already overlay its opaque pixels too.
            // Only a surface BETWEEN the pet and target can leak through a hole.
            if (CGRectContainsRect(bounds, CGRectIntersection(actual, slot))) {
                [covers removeAllObjects];
                if (firstCover) *firstCover = nil;
            }
            continue;
        }
        if (layer > 0 && (pid == getpid() || pid == entry.pid)) continue;
        if (layer == NSFloatingWindowLevel &&
            isCursorOverlay(layer, [NSRunningApplication runningApplicationWithProcessIdentifier:pid].bundleIdentifier)) continue;
        if (firstCover && !*firstCover) *firstCover = info;
        [covers addObject:[NSValue valueWithRect:bounds]];
    }
    return nil; // The compositor has not confirmed the target at this location.
}

// Some owners recreate the backing surface while retaining their AXWindow.
// Never adopt an overlapping sibling, or rebind a window parked on another Space.
static BOOL rebindRetiredSurface(NSArray *stack, CPDrawerEntry *entry, CGRect actual) {
    if (!entry.windowID || windowInfo(entry.windowID)) return NO;
    CGWindowID candidate = identifyWindow(stack,entry.pid,actual);
    if (!candidate) return NO;
    CPDrawerPress identity = {.number=candidate, .pid=entry.pid, .original=actual};
    CPDrawerEntry *match = entryForPress(identity,actual);
    if (!match || !CFEqual(elementOf(match),elementOf(entry))) return NO;
    entry.windowID = candidate;
    cp_drawer_watch(entry.element,entry.pid,candidate);
    return YES;
}

// A clip path can hide a foreign window, but that paints an opaque patch
// over the target. Put the host immediately above the target at its level
// and verify against the next compositor sample before opening the mask.
static BOOL maintainWindowOrder(NSArray *stack, CPDrawerEntry *entry, BOOL intervening) {
    CGWindowID host = cp_drawer_host_number();
    if (!host || !entry.windowID || host == entry.windowID) return NO;
    NSInteger hostIndex = -1, targetIndex = -1, targetLevel = 0;
    for (NSUInteger i=0; i<stack.count; i++) {
        NSDictionary *row = stack[i];
        CGWindowID number = [row[(__bridge id)kCGWindowNumber] unsignedIntValue];
        if (number == host) {
            hostIndex = i;
        } else if (number == entry.windowID) {
            targetIndex = i;
            targetLevel = [row[(__bridge id)kCGWindowLayer] integerValue];
        }
        if (hostIndex >= 0 && targetIndex >= 0) break;
    }
    if (hostIndex < 0 || targetIndex < 0 ||
        (hostIndex < targetIndex && !intervening)) return NO;
    NSTimeInterval now = NSProcessInfo.processInfo.systemUptime;
    if (now - entry.lastOrderAttempt < 0.15) return NO;
    entry.lastOrderAttempt = now;
    return cp_drawer_order_with_target(entry.windowID,targetLevel);
}

static NSDictionary *presentation(CPDrawerEntry *entry, CGRect actual, CGRect slot, BOOL resizing) {
    BOOL grow = !resizing && needsMoreSpace(actual.size,slot.size);
    NSArray *parts = nil;
    NSDictionary *firstCover = nil;
    if (!grow) {
        NSArray *stack = windowStack();
        parts = visibleParts(stack,entry,actual,slot,&firstCover);
        if (!parts && rebindRetiredSurface(stack,entry,actual))
            parts = visibleParts(stack,entry,actual,slot,&firstCover);
        if (parts && !resizing && maintainWindowOrder(stack,entry,firstCover != nil))
            parts = visibleParts(windowStack(),entry,actual,slot,&firstCover);
    }
    NSString *state = resizing ? @"native-resize" : grow ? @"resize-panel" : !parts ? @"awaiting-surface" : parts.count ? @"visible" : @"covered";
    NSMutableDictionary *result = [@{@"width":@(actual.size.width), @"height":@(actual.size.height),
                                     @"visible":parts ?: @[], @"awaitingSurface":@(!grow && !parts),
                                     @"resizing":@(resizing)} mutableCopy];
    if (![state isEqual:entry.presentationState]) {
        entry.presentationState = state;
        result[@"diagnostic"] = [NSString stringWithFormat:@"%@ requested %.0fx%.0f measured %.0fx%.0f",
            state,slot.size.width,slot.size.height,actual.size.width,actual.size.height];
        if ([state isEqual:@"covered"] && firstCover) {
            pid_t pid = [firstCover[(__bridge id)kCGWindowOwnerPID] intValue];
            NSString *owner = [NSRunningApplication runningApplicationWithProcessIdentifier:pid].bundleIdentifier;
            result[@"diagnostic"] = [result[@"diagnostic"] stringByAppendingFormat:@" coverOwner=%@ layer=%@ window=%@ host=%u target=%u",
                owner ?: @"system", firstCover[(__bridge id)kCGWindowLayer], firstCover[(__bridge id)kCGWindowNumber],
                cp_drawer_host_number(), entry.windowID];
        }
    }
    return result;
}

static void activateEntry(CPDrawerEntry *entry) {
    NSRunningApplication *app = [NSRunningApplication runningApplicationWithProcessIdentifier:entry.pid];
    if (!app || (app.hidden && ![app unhide])) return;
    if (@available(macOS 14.0, *)) {
        NSRunningApplication *source = NSRunningApplication.currentApplication;
        if (source.active && entry.pid != getpid()) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (source.active) {
                    [NSApp yieldActivationToApplication:app];
                    [app activateFromApplication:source options:0];
                }
            });
        } else {
            [app activateWithOptions:0];
        }
    } else {
        [app activateWithOptions:0];
    }
    AXUIElementRef owner = AXUIElementCreateApplication(entry.pid);
    AXUIElementSetMessagingTimeout(owner, 0.15);
    id ownerElement = CFBridgingRelease(owner);
    onAXOwner(elementOf(entry),^{
        AXUIElementSetAttributeValue((__bridge AXUIElementRef)ownerElement,kAXFrontmostAttribute,kCFBooleanTrue);
        AXUIElementSetAttributeValue(elementOf(entry),kAXMainAttribute,kCFBooleanTrue);
        AXUIElementPerformAction(elementOf(entry),kAXRaiseAction);
    });
}

static NSDictionary *failure(NSString *message) { return @{@"error":message}; }
// Require movement of both the pointer and this exact WindowServer surface.
// A selection drag, a panel move and a top-left resize must retain ownership.
static NSDictionary *detachMovedWindow(CPDrawerPress press, CGPoint point) {
    CPDrawerEntry *entry = current ? entries[current] : nil;
    if (!entry || press.detached || press.pet || !press.number ||
        press.number != entry.windowID || press.pid != entry.pid || entry.followingResize) return nil;
    if (!CGRectContainsPoint(CGRectInset(lastActual,-6,-6),press.origin) ||
        hypot(point.x-press.origin.x,point.y-press.origin.y) < 8) return nil;
    CGRect frame;
    NSDictionary *row = windowInfo(press.number);
    if (!row || [row[(__bridge id)kCGWindowOwnerPID] intValue] != entry.pid || !boundsOf(row,&frame) ||
        !CGSizeEqualToSize(frame.size,press.original.size) ||
        hypot(frame.origin.x-press.original.origin.x,frame.origin.y-press.original.origin.y) < 8 ||
        !CGRectContainsPoint(CGRectInset(frame,-6,-40),point)) return nil;
    // The system may move an edge before publishing its first size change.
    // For edge presses wait for the final geometry before classifying the drag.
    if (!press.released && !CGRectContainsPoint(CGRectInset(press.original,6,6),press.origin)) return nil;
    @synchronized (CPDrawerEntry.class) {
        if (lastPress.sequence == press.sequence) lastPress.detached = YES;
    }
    NSString *token = current;
    [entries removeObjectForKey:token];
    current = nil;
    cp_drawer_watch(nil,0,0);
    return @{@"detached":token};
}
// During an owner's resize loop its AX server can lag the compositor. Read
// only the selected surface and leave both position and size to the owner.
static NSDictionary *followPresentedWindow(CPDrawerEntry *entry, CGRect slot, CGSize limit, BOOL held, CPDrawerPress press) {
    CGRect measured;
    BOOL targetPress = held && !press.pet && entry.windowID && press.number == entry.windowID &&
        press.pid == entry.pid;
    NSDictionary *row = targetPress ? windowInfo(entry.windowID) : nil;
    BOOL liveFrame = row && [row[(__bridge id)kCGWindowOwnerPID] intValue] == entry.pid && boundsOf(row,&measured);
    if (!liveFrame && !getFrame(entry,&measured))
        return held && (targetPress || entry.followingResize) ? @{@"pending":@YES} : nil;
    if (CGRectIsEmpty(measured)) return nil;
    // Some owners report movement before their first resize event on the
    // top/left edge. A position write here would interrupt the native drag.
    if (liveFrame && !entry.followingResize && CGSizeEqualToSize(measured.size,press.original.size))
        return presentation(entry,(CGRect){measured.origin,lastActual.size},slot,NO);
    // A CG focus border may extend beyond the AX frame. Use movement relative
    // to the press so a simple click does not change the preferred size.
    if (liveFrame) {
        CGFloat dx = lastActual.size.width - press.original.size.width;
        CGFloat dy = lastActual.size.height - press.original.size.height;
        if (fabs(dx)<=2) measured.size.width += dx;
        if (fabs(dy)<=2) measured.size.height += dy;
    }
    if (CGSizeEqualToSize(slot.size,lastFrame.size) && !needsMoreSpace(lastActual.size,lastFrame.size) &&
        !CGSizeEqualToSize(measured.size,lastActual.size)) entry.followingResize = YES;
    if (entry.followingResize) {
        CGSize desired = CGSizeMake(MIN(limit.width,MAX(30,ceil(measured.size.width))),
                                    MIN(limit.height,MAX(30,ceil(measured.size.height))));
        if (held || fabs(slot.size.width-desired.width)>0.5 || fabs(slot.size.height-desired.height)>0.5)
            return presentation(entry,measured,slot,YES);
        entry.followingResize = NO;
    }
    if (CGRectEqualToRect(slot,lastFrame) && CGRectEqualToRect(measured,lastActual))
        return presentation(entry,measured,slot,NO);
    return nil;
}
static id run(NSString *operation,NSString *key,CGRect rect,CGSize limit) {
    if (!entries) entries=[NSMutableDictionary new];
    if ([operation isEqual:@"reset"]) {
        CPDrawerPress press = capturedPress(YES);
        if (current && press.number)
            detachMovedWindow(press,press.released ? press.release : mousePoint());
        for (NSString *token in entries.allKeys) if (restore(entries[token])) [entries removeObjectForKey:token];
        if (!entries[current ?: @""]) { current=nil; cp_drawer_watch(nil,0,0); }
        return entries.count ? failure(@"部分窗口尚未恢复，请检查辅助功能权限后重试。") : @YES;
    }
    if ([operation isEqual:@"dragProbe"]) {
        CGRect actual;
        return @(movedWindow(capturedPress(NO), mousePoint(), &actual));
    }
    if (current && ![operation isEqual:@"permission"] && ![operation isEqual:@"release"]) {
        CPDrawerPress press = capturedPress(NO);
        if (press.number == entries[current].windowID && press.number) {
            NSDictionary *detached = detachMovedWindow(press,press.released ? press.release : mousePoint());
            if (detached) return detached;
        }
    }
    // AX can address this process's own windows without global access.
    // External targets, the system Dock and the permission button still require it.
    BOOL own = NO;
    if ([operation isEqual:@"drop"]) own = capturedPress(NO).pid == getpid();
    if ([operation isEqual:@"park"]) own = current && entries[current].pid == getpid();
    if ([operation isEqual:@"show"] || [operation isEqual:@"activate"] || [operation isEqual:@"release"])
        own = entries[key].pid == getpid();
    if (!own && !AXIsProcessTrusted()) return failure(@"请在“系统设置 → 隐私与安全性 → 辅助功能”中允许 Code Pet，然后刷新。");
    if ([operation isEqual:@"dockClick"] || [operation isEqual:@"dragEnd"]) {
        CGPoint end;
        BOOL clicked = takeDockClick([operation isEqual:@"dragEnd"], &end);
        return clicked && entries.count ? dockSelection(end) : NSNull.null;
    }
    if ([operation isEqual:@"permission"]) return @YES;
    if ([operation isEqual:@"drop"]) return droppedWindow();
    CPDrawerEntry *entry=entries[key];
    if ([operation isEqual:@"release"]) {
        if (!entry) return @YES;
        if (!restore(entry)) return failure(@"窗口暂时无法恢复，请重试释放。");
        [entries removeObjectForKey:key];
        if ([current isEqual:key]) { current=nil; cp_drawer_watch(nil,0,0); }
        return @YES;
    }
    if ([operation isEqual:@"park"]) {
        if (!current) return @YES;
        if (!park(entries[current])) return failure(@"此窗口无法停靠在屏幕下角，面板将保持展开。你可以释放窗口。");
        entries[current].presentationState=nil;
        entries[current].followingResize=NO;
        current=nil; cp_drawer_watch(nil,0,0);
        return @YES;
    }
    BOOL activating = [operation isEqual:@"activate"];
    if ((!activating && ![operation isEqual:@"show"]) || !entry) return failure(@"窗口已失效，请重新拖入。");
    if (!entry.attached) {
        CGRect original;
        if (!getFrame(entry,&original)) return failure(@"无法读取窗口位置。");
        entry.original=original; entry.attached=YES;
    }
    if (!activating && [current isEqual:key]) {
        BOOL held = CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState,kCGMouseButtonLeft);
        NSDictionary *layout = followPresentedWindow(entry,rect,limit,held,capturedPress(NO));
        if (layout) return layout;
    }
    if (current && ![current isEqual:key]) return failure(@"请先收起当前窗口。");
    CGRect actual = CGRectZero;
    if (!fitWindow(entry,rect,&actual)) {
        BOOL recovered=restore(entry);
        if ([current isEqual:key] && recovered) { current=nil; cp_drawer_watch(nil,0,0); }
        return @{@"error":recovered ? @"应用未接受展示位置，已恢复原窗口。" : @"窗口调整失败，请重试释放窗口。",
                 @"diagnostic":[NSString stringWithFormat:@"fit failed: requested %@ measured %@",
                                      NSStringFromRect(rect), NSStringFromRect(actual)]};
    }
    BOOL revealing = !needsMoreSpace(actual.size,rect.size) &&
        (activating || ![current isEqual:key] || needsMoreSpace(lastActual.size,lastFrame.size));
    if (revealing) {
        activateEntry(entry);
        entry.lastUse = ++useCounter;
    }
    if (![current isEqual:key]) cp_drawer_watch(entry.element,entry.pid,entry.windowID);
    current=[key copy]; lastFrame=rect; lastActual=actual;
    entry.followingResize = NO;
    NSDictionary *layout = presentation(entry,actual,rect,NO);
    // App activation and window ordering are separate WindowServer updates.
    // Retry the raise once during this reveal, only if focus belongs to the
    // target application. Passive visibility checks never change the order.
    if (revealing && ![layout[@"visible"] count]) {
        [NSThread sleepForTimeInterval:0.05];
        if (NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier == entry.pid) {
            onAXOwner(elementOf(entry),^{ AXUIElementPerformAction(elementOf(entry),kAXRaiseAction); });
            entry.presentationState = nil;
            return presentation(entry,actual,rect,NO);
        }
    }
    return layout;
}
char *cp_drawer_call(const char *op,const char *key,double x,double y,double w,double h,double maxW,double maxH) {
    @autoreleasepool {
        id result=run(@(op),@(key),CGRectMake(x,y,w,h),CGSizeMake(maxW,maxH));
        NSData *json=[NSJSONSerialization dataWithJSONObject:result options:NSJSONWritingFragmentsAllowed error:NULL];
        return strdup([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String ?: "null");
    }
}
