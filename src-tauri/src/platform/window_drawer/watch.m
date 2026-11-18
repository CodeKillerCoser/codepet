#import "watch.h"

// Only the presented window owns observers. AppKit/workspace notifications
// arrive on the main run loop; AX is best effort (some owners omit events).
static __weak NSWindow *drawerHost;
static void (*notifyDrawer)(bool,double,double);
static NSRect hostFrame;
static NSPoint displacement;
static id watchedWindow;
static AXObserverRef targetObserver, frontObserver;
static pid_t watchedPid, frontPid;
static CGWindowID watchedNumber;
static NSUInteger targetGeneration, frontGeneration, subscription;
static id dragMonitor;
static NSTimeInterval lastDragWake;
static NSMutableArray *watchTokens;
static BOOL wakeQueued, mustCover;

void cp_drawer_connect(void *host, void (*notify)(bool,double,double)) {
    drawerHost = (__bridge NSWindow *)host;
    notifyDrawer = notify;
}

static void wakeDrawer(BOOL cover) {
    if (!watchedWindow || !notifyDrawer) return;
    mustCover |= cover;
    if (wakeQueued) return;
    wakeQueued = YES;
    NSUInteger generation = subscription;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (subscription != generation) return;
        BOOL seal = mustCover;
        NSPoint delta = displacement; displacement = NSZeroPoint;
        wakeQueued = NO; mustCover = NO;
        if (watchedWindow) notifyDrawer(seal,delta.x,delta.y);
    });
}

// Programmatic moves during panel dragging can precede NSWindowDidMove.
void cp_drawer_host_changed(void) {
    if (!watchedWindow || !drawerHost) return;
    NSRect next = drawerHost.frame;
    if (NSEqualRects(next,hostFrame)) return;
    displacement.x += next.origin.x - hostFrame.origin.x;
    displacement.y += NSMaxY(hostFrame) - NSMaxY(next);
    BOOL resized = !NSEqualSizes(hostFrame.size,next.size);
    hostFrame = next;
    wakeDrawer(resized);
}

static void axChanged(AXObserverRef __unused observer, AXUIElementRef element, CFStringRef name, void *__unused context) {
    // A focused sibling, newly opened window or another app may cover the slot.
    BOOL targetGeometry = watchedWindow && CFEqual((__bridge CFTypeRef)watchedWindow, element) &&
        (CFEqual(name,kAXMovedNotification) || CFEqual(name,kAXResizedNotification));
    wakeDrawer(!targetGeometry);
}

static void removeObserver(AXObserverRef *observer) {
    if (!*observer) return;
    CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(*observer), kCFRunLoopCommonModes);
    CFRelease(*observer); *observer = NULL;
}

static AXObserverRef observeOwner(pid_t pid, id window) {
    AXObserverRef observer = NULL;
    if (!pid || AXObserverCreate(pid,axChanged,&observer) != kAXErrorSuccess) return NULL;
    AXUIElementRef app = AXUIElementCreateApplication(pid);
    AXUIElementSetMessagingTimeout(app,0.12);
    CFStringRef changes[] = {kAXFocusedWindowChangedNotification,kAXWindowCreatedNotification,
        kAXMovedNotification,kAXResizedNotification,kAXUIElementDestroyedNotification,
        kAXWindowMiniaturizedNotification,kAXWindowDeminiaturizedNotification};
    for (size_t i=0;i<sizeof(changes)/sizeof(*changes);i++)
        AXObserverAddNotification(observer,app,changes[i],NULL);
    if (window) {
        AXUIElementRef el = (__bridge AXUIElementRef)window;
        AXObserverAddNotification(observer,el,kAXMovedNotification,NULL);
        AXObserverAddNotification(observer,el,kAXResizedNotification,NULL);
    }
    CFRelease(app);
    return observer;
}

// Registration can involve a slow remote AX server. Install its source only
// if the selected/frontmost owner still matches when registration finishes.
static void installObserver(pid_t pid, id window, BOOL target) {
    NSUInteger generation = target ? ++targetGeneration : ++frontGeneration;
    if (!pid) return;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0), ^{
        AXObserverRef observer = observeOwner(pid,window);
        if (!observer) return;
        dispatch_async(dispatch_get_main_queue(), ^{
            if (generation != (target ? targetGeneration : frontGeneration)) { CFRelease(observer); return; }
            if (target) targetObserver = observer;
            else frontObserver = observer;
            CFRunLoopAddSource(CFRunLoopGetMain(),AXObserverGetRunLoopSource(observer),kCFRunLoopCommonModes);
        });
    });
}

static void observeFront(void) {
    pid_t pid = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
    if (pid == watchedPid) pid = 0;
    if (frontPid == pid) return;
    removeObserver(&frontObserver);
    frontPid = pid;
    installObserver(pid,nil,NO);
}

void cp_drawer_other_window_pressed(CGWindowID number) {
    if (number != watchedNumber) wakeDrawer(YES);
}

void cp_drawer_watch(id window, pid_t pid, CGWindowID number) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (watchedWindow == window) return;
        subscription++;
        wakeQueued = NO;
        mustCover = NO;
        displacement = NSZeroPoint;
        for (NSArray *pair in watchTokens) [pair[0] removeObserver:pair[1]];
        watchTokens = nil; targetGeneration++; frontGeneration++;
        if (dragMonitor) { [NSEvent removeMonitor:dragMonitor]; dragMonitor = nil; }
        removeObserver(&targetObserver); removeObserver(&frontObserver);
        watchedWindow = window; watchedPid = pid; watchedNumber = number; frontPid = 0;
        if (!window || !notifyDrawer) return;
        watchTokens = [NSMutableArray new];
        NSNotificationCenter *workspace = NSWorkspace.sharedWorkspace.notificationCenter;
        for (NSString *name in @[NSWorkspaceDidActivateApplicationNotification,NSWorkspaceActiveSpaceDidChangeNotification,
            NSWorkspaceDidHideApplicationNotification,NSWorkspaceDidUnhideApplicationNotification]) {
            id token = [workspace addObserverForName:name object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *__unused n) {
                wakeDrawer(YES); observeFront();
            }];
            [watchTokens addObject:@[workspace,token]];
        }
        hostFrame = drawerHost.frame;
        NSNotificationCenter *center = NSNotificationCenter.defaultCenter;
        for (NSString *name in @[NSWindowDidMoveNotification,NSWindowDidResizeNotification]) {
            id token = [center addObserverForName:name object:drawerHost queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *__unused n) {
                cp_drawer_host_changed();
            }];
            [watchTokens addObject:@[center,token]];
        }
        id occlusion = [center addObserverForName:NSWindowDidChangeOcclusionStateNotification object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *n) {
            NSWindow *other = n.object;
            if (other != drawerHost && other.windowNumber != watchedNumber && other.level <= NSFloatingWindowLevel) wakeDrawer(YES);
        }];
        [watchTokens addObject:@[center,occlusion]];
        // Covers apps that omit AX geometry notifications during live dragging.
        dragMonitor = [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskLeftMouseDragged handler:^(NSEvent *e) {
            if (e.timestamp-lastDragWake >= 1.0/60) { lastDragWake = e.timestamp; wakeDrawer(NO); }
        }];
        installObserver(pid,window,YES);
        observeFront();
    });
}
