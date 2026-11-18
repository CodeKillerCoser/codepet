#pragma once
#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>

// Main thread. A pure host move reports its displacement in top-down points.
void cp_drawer_connect(void *host, void (*notify)(bool cover, double dx, double dy));
void cp_drawer_host_changed(void);
void cp_drawer_other_window_pressed(CGWindowID number);

// May be called from the drawer worker. Passing nil removes the subscription.
void cp_drawer_watch(id window, pid_t pid, CGWindowID number);
