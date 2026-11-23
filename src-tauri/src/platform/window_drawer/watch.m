#import "watch.h"
#import <stdatomic.h>

// Only the presented window owns observers. AppKit/workspace notifications
// arrive on the main run loop; AX is best effort (some owners omit events).
static __weak NSWindow *drawerHost;
static _Atomic(CGWindowID) hostNumber;
static void (*notifyDrawer)(CPDrawerChange,double,double);
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
static BOOL grouped;
static NSInteger surfaceLevel;

void cp_drawer_set_level(void *host, NSInteger level) {
    NSWindow *panel = (__bridge NSWindow *)host;
    if (panel == drawerHost && grouped) surfaceLevel = level;
    else if (panel.level != level) panel.level = level;
}
static BOOL wakeQueued;
static CPDrawerChange pendingChange;

void cp_drawer_connect(void *host, void (*notify)(CPDrawerChange,double,double)) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        [NSNotificationCenter.defaultCenter addObserverForName:NSWindowWillMoveNotification object:nil
            queue:nil usingBlock:^(NSNotification *note) {
                if (note.object != drawerHost) cp_drawer_will_move(note.object);
            }];
    });
    drawerHost = (__bridge NSWindow *)host;
    atomic_store(&hostNumber,(CGWindowID)drawerHost.windowNumber);
    notifyDrawer = notify;
}

CGWindowID cp_drawer_host_number(void) { return atomic_load(&hostNumber); }

static void wakeDrawer(CPDrawerChange change) {
    if (!watchedWindow || !notifyDrawer) return;
    pendingChange = MAX(change,pendingChange);
    if (wakeQueued) return;
    wakeQueued = YES;
    NSUInteger generation = subscription;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (subscription != generation) return;
        CPDrawerChange change = pendingChange;
        NSPoint delta = displacement; displacement = NSZeroPoint;
        wakeQueued = NO; pendingChange = CPDrawerGeometry;
        if (watchedWindow) notifyDrawer(change,delta.x,delta.y);
    });
}

// Keep ordinary windows on one side of the entire presentation. Only move
// our own panel in z-order, never raise or activate the third-party app here.
BOOL cp_drawer_order_with_target(CGWindowID number, NSInteger level) {
    __block BOOL ordered = NO;
    dispatch_block_t order = dispatch_block_create(0, ^{
        if (!watchedWindow || watchedNumber != number || !drawerHost.visible ||
            number == drawerHost.windowNumber || !number) return;
        NSInteger previous = drawerHost.level;
        if (!grouped) surfaceLevel = previous;
        @try {
            if (previous != level) drawerHost.level = level;
            [drawerHost orderWindow:NSWindowAbove relativeTo:number];
            grouped = YES;
            ordered = YES;
        } @catch (NSException *exception) {
            // The target may have closed after the WindowServer sample.
            if (![exception.name isEqual:NSInvalidArgumentException]) @throw;
            drawerHost.level = previous;
        }
    });
    if (NSThread.isMainThread) order();
    else {
        dispatch_async(dispatch_get_main_queue(),order);
        if (dispatch_block_wait(order,dispatch_time(DISPATCH_TIME_NOW,80*NSEC_PER_MSEC))) {
            dispatch_block_cancel(order);
            return NO;
        }
    }
    return ordered;
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
    wakeDrawer(resized ? CPDrawerSurfaceInvalid : CPDrawerGeometry);
}

static void axChanged(AXObserverRef __unused observer, AXUIElementRef element, CFStringRef name, void *__unused context) {
    // A focused sibling, newly opened window or another app may cover the slot.
    BOOL targetGeometry = watchedWindow && CFEqual((__bridge CFTypeRef)watchedWindow, element) &&
        (CFEqual(name,kAXMovedNotification) || CFEqual(name,kAXResizedNotification));
    BOOL lost = watchedWindow && CFEqual((__bridge CFTypeRef)watchedWindow,element) &&
        (CFEqual(name,kAXUIElementDestroyedNotification) || CFEqual(name,kAXWindowMiniaturizedNotification));
    wakeDrawer(lost ? CPDrawerSurfaceInvalid : targetGeometry ? CPDrawerGeometry : CPDrawerStackChanged);
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
        AXObserverAddNotification(observer,el,kAXUIElementDestroyedNotification,NULL);
        AXObserverAddNotification(observer,el,kAXWindowMiniaturizedNotification,NULL);
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
    if (number != watchedNumber) wakeDrawer(CPDrawerStackChanged);
}

void cp_drawer_watch(id window, pid_t pid, CGWindowID number) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (watchedWindow == window) { watchedNumber = number; return; }
        if (grouped) {
            drawerHost.level = surfaceLevel;
            grouped = NO;
        }
        subscription++;
        wakeQueued = NO;
        pendingChange = CPDrawerGeometry;
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
            id token = [workspace addObserverForName:name object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *n) {
                NSRunningApplication *app = n.userInfo[NSWorkspaceApplicationKey];
                BOOL lost = [n.name isEqual:NSWorkspaceActiveSpaceDidChangeNotification] ||
                    ([n.name isEqual:NSWorkspaceDidHideApplicationNotification] && app.processIdentifier == watchedPid);
                wakeDrawer(lost ? CPDrawerSurfaceInvalid : CPDrawerStackChanged); observeFront();
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
            if (other != drawerHost && other.windowNumber != watchedNumber && other.level <= NSFloatingWindowLevel) wakeDrawer(CPDrawerStackChanged);
        }];
        [watchTokens addObject:@[center,occlusion]];
        // Covers apps that omit AX geometry notifications during live dragging.
        dragMonitor = [NSEvent addGlobalMonitorForEventsMatchingMask:NSEventMaskLeftMouseDragged handler:^(NSEvent *e) {
            if (e.timestamp-lastDragWake >= 1.0/60) { lastDragWake = e.timestamp; wakeDrawer(CPDrawerGeometry); }
        }];
        installObserver(pid,window,YES);
        observeFront();
    });
}
