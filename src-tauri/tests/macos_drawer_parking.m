// Run on a logged-in Mac. The AX target is this executable's own disposable window.
#import "../src/platform/window_drawer/macos.m"
#import <assert.h>

// Simulate a window owner that enforces its own layout minimum for AX requests.
@interface MinimumWindow : NSWindow
@property NSSize enforcedMinimum;
@property CGFloat maximumX;
@property BOOL testModal;
@end
@implementation MinimumWindow
// Qt 6 frameless QMainWindow reports this same subrole on macOS.
- (NSAccessibilitySubrole)accessibilitySubrole { return NSAccessibilityDialogSubrole; }
- (BOOL)isAccessibilityModal { return self.testModal; }
- (void)setFrame:(NSRect)frame display:(BOOL)display {
    frame.size.width = MAX(frame.size.width, self.enforcedMinimum.width);
    frame.size.height = MAX(frame.size.height, self.enforcedMinimum.height);
    if (self.maximumX && frame.origin.x > self.maximumX) frame.origin.x = self.maximumX;
    [super setFrame:frame display:display];
}
@end

static void checkDisplayGeometry(void) {
    CGRect screens[] = {CGRectMake(0, 0, 1000, 800), CGRectMake(1000, 0, 800, 800)};
    assert(isParked(CGRectMake(1810, 810, 400, 300), screens, 2));
    assert(isParked(CGRectMake(1799, 768, 400, 300), screens, 2));
    assert(isParked(CGRectMake(-399, 768, 400, 300), screens, 2));
    assert(!isParked(CGRectMake(999, 768, 400, 300), screens, 2)); // adjacent display
    assert(!isParked(CGRectMake(1798, 768, 400, 300), screens, 2)); // too wide
    assert(!isParked(CGRectMake(1799, 710, 400, 300), screens, 2)); // too tall
    assert(!isParked(CGRectMake(1799, 10, 400, 300), screens, 2));
    screens[1] = CGRectMake(500, 800, 800, 600);
    assert(!isParked(CGRectMake(999, 768, 400, 300), screens, 2)); // display below
}

static NSDictionary *surface(pid_t pid, CGRect frame) {
    return @{(__bridge id)kCGWindowNumber:@(pid + 321), (__bridge id)kCGWindowOwnerPID:@(pid), (__bridge id)kCGWindowLayer:@0,
             (__bridge id)kCGWindowAlpha:@1, (__bridge id)kCGWindowBounds:
             CFBridgingRelease(CGRectCreateDictionaryRepresentation(frame))};
}
static void checkOcclusion(void) {
    CGRect slot = CGRectMake(240, 80, 800, 560);
    CGRect clamped = CGRectMake(180,80,920,600);
    assert(!acceptsMeasurement(clamped,slot,NO,NO));
    assert(acceptsMeasurement(clamped,slot,NO,YES));
    assert(!acceptsMeasurement(CGRectOffset(slot,40,0),slot,YES,YES));
    NSDictionary *target = surface(1, slot);
    NSDictionary *finder = surface(2, CGRectOffset(slot, -200, 200));
    NSMutableDictionary *sibling = [target mutableCopy];
    sibling[(__bridge id)kCGWindowNumber] = @12345;
    assert(identifyWindow(@[target,sibling],1,slot) == kCGNullWindowID);
    CPDrawerEntry *entry = [CPDrawerEntry new];
    entry.pid = 1;
    NSDictionary *negotiation = presentation(entry,clamped,slot,NO);
    assert([negotiation[@"width"] intValue] == 920 && [negotiation[@"visible"] isEqual:@[]]);
    NSArray *all = visibleParts(@[target, finder], entry, slot, slot);
    assert(all.count == 1 && [all[0][@"width"] intValue] == 800);
    NSArray *parts = visibleParts(@[finder, target], entry, slot, slot);
    double area = 0;
    for (NSDictionary *part in parts) area += [part[@"width"] doubleValue] * [part[@"height"] doubleValue];
    assert(area == 800*560 - 600*360);
    assert(visibleParts(@[finder], entry, slot, slot) == nil);
    assert(visibleParts(@[surface(1,CGRectOffset(slot,900,0))], entry, slot, slot) == nil);
    assert([visibleParts(@[surface(2, slot), target], entry, slot, slot) isEqual:@[]]);
    assert(isCursorOverlay(NSFloatingWindowLevel, @"com.openai.sky.CUAService"));
    assert(!isCursorOverlay(NSNormalWindowLevel, @"com.openai.sky.CUAService"));
    assert(!isCursorOverlay(NSFloatingWindowLevel, @"example.opaque-palette"));
    NSMutableDictionary *palette = [surface(2, slot) mutableCopy];
    palette[(__bridge id)kCGWindowLayer] = @(NSFloatingWindowLevel);
    assert(visibleParts(@[palette, target], entry, slot, slot).count == 0);
    // Exercise the identified cursor helper against the same overlapping stack
    // when it is installed and running. Only its public process identity is used.
    NSRunningApplication *helper = [NSRunningApplication runningApplicationsWithBundleIdentifier:@"com.openai.sky.CUAService"].firstObject;
    if (helper) {
        palette[(__bridge id)kCGWindowOwnerPID] = @(helper.processIdentifier);
        assert([visibleParts(@[palette, target], entry, slot, slot) isEqual:all]);
    }
    CGRect snapped = CGRectMake(slot.origin.x, slot.origin.y, 800, 559);
    NSArray *grid = visibleParts(@[surface(1,snapped)], entry, snapped, slot);
    assert(grid.count == 1 && [grid[0][@"height"] intValue] == 559);
}

static void checkClampedMinimum(MinimumWindow *fixture, CPDrawerEntry *entry, CGRect original) {
    fixture.enforcedMinimum = NSMakeSize(880,610);
    fixture.maximumX = original.origin.x + 10;
    CGRect slot = CGRectMake(fixture.maximumX + 40, original.origin.y, 800, 560);
    CGRect measured;
    assert(fitWindow(entry,slot,&measured));
    assert(needsMoreSpace(measured.size,slot.size) && measured.origin.x < slot.origin.x - 2);
    NSDictionary *pending = presentation(entry,measured,slot,NO);
    assert([pending[@"visible"] isEqual:@[]] && [pending[@"width"] doubleValue] >= 880);
    CGRect larger = (CGRect){.origin=original.origin, .size=measured.size};
    assert(fitWindow(entry,larger,&measured) && nearFrame(larger,measured));
    fixture.maximumX = 0;
    fixture.enforcedMinimum = NSZeroSize;
    assert(place(entry,original));
}
static void checkHostedResize(MinimumWindow *fixture, CPDrawerEntry *entry, CGRect original) {
    CGSize available = CGSizeMake(900,620);
    CGRect slot = CGRectMake(120,100,800,560);
    assert(place(entry,slot));
    entry.original = original; entry.attached = YES;
    entry.windowID = (CGWindowID)fixture.windowNumber;
    lastFrame = lastActual = slot;
    CPDrawerPress press = {.pid=entry.pid, .number=entry.windowID};
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.08]];
    assert(boundsOf(windowInfo(entry.windowID),&press.original));
    press.origin = CGPointMake(CGRectGetMaxX(press.original)-1,CGRectGetMaxY(press.original)-1);
    // Resize from the left/top as well as right/bottom. During tracking
    // both the new size and the owner's changing origin must survive a sync.
    CGRect samples[] = {CGRectMake(80,80,860,600),CGRectMake(150,130,650,420),CGRectMake(75,75,940,680)};
    for (int i = 0; i < 3; i++) {
        CGRect measured = samples[i], after;
        assert(place(entry,measured));
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.08]];
        NSDictionary *layout = followPresentedWindow(entry,slot,available,YES,press);
        assert([layout[@"resizing"] boolValue]);
        // The compositor can include a focus border while tracking. AX
        // below must converge to exact coordinates after release.
        assert(fabs([layout[@"width"] doubleValue]-measured.size.width)<=6 &&
               fabs([layout[@"height"] doubleValue]-measured.size.height)<=6);
        assert(getFrame(entry,&after) && CGRectEqualToRect(after,measured));
        slot.size = CGSizeMake(MIN(available.width,measured.size.width),MIN(available.height,measured.size.height));
    }
    assert(!followPresentedWindow(entry,slot,available,NO,press));
    CGRect fitted;
    assert(fitWindow(entry,slot,&fitted) && nearFrame(fitted,slot));
    assert(restore(entry) && getFrame(entry,&fitted) && nearFrame(fitted,original));
}

// Deliver an event to the drawer callback, without moving the user's mouse.
static void fixtureMouse(CGEventType type, CGWindowID number, CGPoint point, BOOL pet) {
    CGEventRef cg = CGEventCreateMouseEvent(NULL, type, point, kCGMouseButtonLeft);
    CGEventSetIntegerValueField(cg, kCGMouseEventWindowUnderMousePointerThatCanHandleThisEvent, number);
    NSEvent *event = [NSEvent eventWithCGEvent:cg];
    cp_drawer_mouse_event((__bridge void *)event, pet);
    CFRelease(cg);
}

static void checkDockRelease(void) {
    CGPoint start = CGPointMake(120, 160), end = CGPointMake(121, 161), point;
    fixtureMouse(kCGEventLeftMouseDown, 0, start, YES);
    fixtureMouse(kCGEventLeftMouseUp, 0, end, YES);
    assert(takeDockClick(NO, &point) && CGPointEqualToPoint(point, end));
    assert(!takeDockClick(NO, &point)); // The same release cannot activate twice.

    fixtureMouse(kCGEventLeftMouseDown, 0, start, YES);
    fixtureMouse(kCGEventLeftMouseUp, 0, CGPointMake(150, 160), YES);
    assert(!takeDockClick(YES, &point)); // A tracked drag is not a Dock click.
}

static void checkNativeDrop(CPDrawerEntry *entry, CGRect original) {
    assert(getFrame(entry, &original)); // Read after AppKit applies the test window level.
    CGPoint start = CGPointMake(original.origin.x + 96, original.origin.y + 110);
    CGWindowID number = identifyWindow(windowStack(), entry.pid, original);
    assert(number);
    fixtureMouse(kCGEventLeftMouseDown, number, start, YES);
    assert(!capturedPress(NO).number);
    fixtureMouse(kCGEventLeftMouseDown, number, start, NO);
    assert(capturedPress(NO).number == number);
    CGRect frame;
    CGPoint end = CGPointMake(start.x - 90, start.y - 65);
    assert(!movedWindow(capturedPress(NO), end, &frame)); // Dragging content, window stationary.
    CGRect moved = CGRectOffset(original, -90, -65);
    moved.size.width += 16; // An owner may resize while the title bar is being dragged.
    assert(place(entry, moved));
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
    assert(movedWindow(capturedPress(NO), end, &frame));
    fixtureMouse(kCGEventLeftMouseUp, number, end, NO);
    entries = [NSMutableDictionary new];
    NSDictionary *item = droppedWindow();
    assert([item isKindOfClass:NSDictionary.class] && item[@"id"]);
    CPDrawerEntry *pinned = entries[item[@"id"]];
    NSLog(@"Press %@ restored target %@",NSStringFromRect(original),NSStringFromRect(pinned.original));
    assert(pinned.windowID == number && nearFrame(pinned.original, original));
    assert(restore(pinned));
    assert(getFrame(entry, &frame) && CGRectEqualToRect(frame, pinned.original));
    assert([droppedWindow()[@"ignored"] isEqual:@"no-source-event"]); // Each release can be consumed only once.
}

static int drawerEvents;
static BOOL covered;
static CGFloat hostDX;
static void notice(bool cover, double dx, double dy) {
    drawerEvents++; covered |= cover; hostDX += dx;
}
static void pump(void) {
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.06]];
}
static void checkDrawerEvents(NSWindow *window, CPDrawerEntry *entry) {
    NSPoint origin = window.frame.origin;
    cp_drawer_connect((__bridge void *)window,notice);
    CGRect slot = CGRectMake(240,80,800,560);
    CPDrawerEntry *other = [CPDrawerEntry new]; other.pid = 1;
    NSMutableDictionary *host = [surface(getpid(),slot) mutableCopy];
    host[(__bridge id)kCGWindowNumber] = @(window.windowNumber);
    assert(visibleParts(@[host,surface(1,slot)],other,slot,slot).count == 1);
    host[(__bridge id)kCGWindowNumber] = @(window.windowNumber + 1);
    assert([visibleParts(@[host,surface(1,slot)],other,slot,slot) isEqual:@[]]);
    cp_drawer_watch(entry.element,entry.pid,(CGWindowID)window.windowNumber);
    pump();
    hostDX = 0;
    [window setFrameOrigin:NSMakePoint(origin.x+18,origin.y)];
    cp_drawer_host_changed(); // A native drag and its AppKit notification coalesce.
    pump();
    assert(hostDX == 18);
    covered = NO;
    [NSWorkspace.sharedWorkspace.notificationCenter postNotificationName:NSWorkspaceActiveSpaceDidChangeNotification object:nil];
    pump();
    assert(covered);
    cp_drawer_watch(nil,0,0); pump();
    int stopped = drawerEvents;
    [window setFrameOrigin:origin]; cp_drawer_host_changed(); pump();
    assert(drawerEvents == stopped);
}

int main(int argc, const char **argv) {
    @autoreleasepool {
        checkDisplayGeometry();
        checkOcclusion();
        [NSApplication sharedApplication];
        checkDockRelease();
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        MinimumWindow *fixture = [[MinimumWindow alloc] initWithContentRect:NSMakeRect(80, 120, 441, 228)
            styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskResizable
            backing:NSBackingStoreBuffered defer:NO];
        [fixture center];
        [fixture orderFront:nil];
        [NSApp finishLaunching];
        AXUIElementRef app = AXUIElementCreateApplication(getpid());
        if (argc == 2 && !strcmp(argv[1],"--notifications")) {
            CPDrawerEntry *owner = [CPDrawerEntry new];
            owner.element = CFBridgingRelease(app); owner.pid = getpid();
            checkDrawerEvents(fixture,owner);
            [fixture orderOut:nil];
            return 0;
        }
        NSArray *windows = attribute(app, kAXWindowsAttribute);
        CFRelease(app);
        assert(windows.count == 1);
        CPDrawerEntry *entry = [CPDrawerEntry new];
        entry.element = windows.firstObject;
        entry.pid = getpid();
        NSRect originalFrame = fixture.frame;
        CGRect original;
        assert(getFrame(entry, &original));
        checkClampedMinimum(fixture,entry,original);
        checkHostedResize(fixture,entry,original);
        if (argc == 2 && !strcmp(argv[1],"--fit")) { [fixture orderOut:nil]; return 0; }
        assert([attribute(elementOf(entry), kAXSubroleAttribute) isEqual:@"AXDialog"]);
        assert(isDraggableWindow(entry.element));
        fixture.testModal = YES;
        assert(!isDraggableWindow(entry.element));
        fixture.testModal = NO;
        // Use only this disposable window for the live WindowServer identity check.
        fixture.level = NSStatusWindowLevel;
        [fixture orderFrontRegardless];
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.15]];
        NSArray *stack = windowStack();
        NSLog(@"Fixture AX %@, AppKit %@, window number %ld",NSStringFromRect(original),NSStringFromRect(fixture.frame),fixture.windowNumber);
        for (NSDictionary *row in stack) if ([row[(__bridge id)kCGWindowOwnerPID] intValue] == getpid()) {
            CGRect cg; boundsOf(row,&cg); NSLog(@"Fixture CG %@ number %@", NSStringFromRect(cg),row[(__bridge id)kCGWindowNumber]);
        }
        assert(identifyWindow(stack, entry.pid, original) == (CGWindowID)fixture.windowNumber);
        NSArray *onscreen = visibleParts(stack, entry, original, original);
        assert(onscreen.count > 0);
        checkDrawerEvents(fixture,entry);
        checkNativeDrop(entry, original);
        entry.original = original;
        entry.attached = YES;
        CGRect slot = CGRectMake(original.origin.x, original.origin.y, 800, 560);
        assert(place(entry, slot));
        CGRect fitted;
        assert(getFrame(entry, &fitted) && CGRectEqualToRect(fitted, slot));
        fixture.enforcedMinimum = NSMakeSize(880, 610);
        [fixture setFrame:NSMakeRect(fixture.frame.origin.x, fixture.frame.origin.y, 950, 700) display:YES];
        assert(fitWindow(entry, slot, &fitted));
        assert(fitted.size.width >= 880 && fitted.size.height >= 610);
        const CGRect translated = CGRectOffset(slot,-10,12);
        assert(fitWindow(entry, translated, &fitted));
        assert(CGSizeEqualToSize(entry.acceptedSize,fitted.size));
        assert(CGPointEqualToPoint(fitted.origin,translated.origin));
        fixture.enforcedMinimum = NSZeroSize;
        for (int cycle = 0; cycle < 2; cycle++) {
            assert(park(entry));
            assert(!fixture.isMiniaturized && !NSApp.isHidden);
            CGRect parked;
            assert(getFrame(entry, &parked));
            NSLog(@"Parked window %@, cycle %d", NSStringFromRect(parked), cycle + 1);
            assert(place(entry, original));
            assert(NSEqualRects(fixture.frame, originalFrame));
        }
        assert(park(entry) && restore(entry) && !entry.attached);
        [fixture orderOut:nil];
    }
    return 0;
}
