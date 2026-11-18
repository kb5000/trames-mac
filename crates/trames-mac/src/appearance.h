// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#import <AppKit/AppKit.h>

// Local native-UI preference. Never enters environment TOML or guest rendering.
static inline NSString *TMInterfaceStyle(void) {
    NSString *style=[NSUserDefaults.standardUserDefaults stringForKey:@"interfaceStyle"];
    return [@[@"light",@"dark"] containsObject:style] ? style : @"system";
}
static inline void TMApplyInterfaceStyle(void) {
    NSString *style=TMInterfaceStyle();
    NSApp.appearance=[style isEqual:@"system"] ? nil : [NSAppearance appearanceNamed:[style isEqual:@"dark"] ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua];
}
static inline void TMObserveInterfaceStyle(void) {
    TMApplyInterfaceStyle();
    static dispatch_once_t once;
    dispatch_once(&once,^{
        // The manager and lazy Wayland frontend are separate processes. One
        // event updates existing native chrome; no polling or guest dependency.
        [NSDistributedNotificationCenter.defaultCenter addObserverForName:@"org.trames.interfaceStyleChanged" object:NSBundle.mainBundle.bundleIdentifier queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note){
            (void)note;[NSUserDefaults.standardUserDefaults synchronize];TMApplyInterfaceStyle();
        }];
    });
}
static inline void TMSetInterfaceStyle(NSString *style) {
    [NSUserDefaults.standardUserDefaults setObject:style forKey:@"interfaceStyle"];
    [NSUserDefaults.standardUserDefaults synchronize];TMApplyInterfaceStyle();
    [NSDistributedNotificationCenter.defaultCenter postNotificationName:@"org.trames.interfaceStyleChanged" object:NSBundle.mainBundle.bundleIdentifier userInfo:nil deliverImmediately:YES];
}
