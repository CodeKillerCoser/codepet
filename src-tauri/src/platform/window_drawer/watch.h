#pragma once
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>

// Main thread. A pure host move reports its displacement in top-down points.
typedef NS_ENUM(uint8_t, CPDrawerChange) {
    CPDrawerGeometry, CPDrawerStackChanged, CPDrawerSurfaceInvalid
};
void cp_drawer_connect(void *host, void (*notify)(CPDrawerChange change, double dx, double dy));
CGWindowID cp_drawer_host_number(void);
void cp_drawer_host_changed(void);
void cp_drawer_set_level(void *host, NSInteger level);
BOOL cp_drawer_order_with_target(CGWindowID number, NSInteger level);
void cp_drawer_other_window_pressed(CGWindowID number);

// May be called from the drawer worker. Passing nil removes the subscription.
void cp_drawer_watch(id window, pid_t pid, CGWindowID number);

void cp_drawer_will_move(NSWindow *window);
