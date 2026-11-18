// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#import "../../crates/trames-mac/src/manager-ui.h"
#import "../../crates/trames-mac/src/appearance.h"

@interface TramesManager (LayoutTests)
- (void)fieldChanged:(NSControl *)sender;
- (void)comboBoxSelectionDidChange:(NSNotification *)note;
@end
@interface TMLayoutFixture : TramesManager
@property NSArray *lastArguments;
@property NSDictionary *lastBody;
@end
@implementation TMLayoutFixture
- (void)refresh:(id)sender {(void)sender;}
- (void)watchRegistry {}
- (BOOL)confirm:(NSString *)title message:(NSString *)message {(void)title;(void)message;return YES;}
- (void)run:(NSArray *)args owner:(NSString *)key name:(NSString *)name title:(NSString *)title quiet:(BOOL)quiet input:(NSData *)input completion:(void(^)(NSString *,int))done {
    (void)key;(void)name;(void)title;(void)quiet;self.lastArguments=args;self.lastBody=input?[NSJSONSerialization JSONObjectWithData:input options:0 error:nil]:nil;
    if(done && [args containsObject:@"permission"])done(@"{\"revision\":\"permission-saved\"}",0);
}
@end
static NSArray<NSView *> *descendants(NSView *root) {
    NSMutableArray *views=[NSMutableArray arrayWithObject:root];for(NSView *child in root.subviews)[views addObjectsFromArray:descendants(child)];return views;
}
static id identified(NSView *root,NSString *identifier) {
    for(NSView *v in descendants(root))if([v.identifier isEqual:identifier])return v;return nil;
}
static NSButton *button(NSView *root,NSString *title) {
    for(NSView *v in descendants(root))if([v isKindOfClass:NSButton.class] && [((NSButton *)v).title isEqual:title])return (NSButton *)v;return nil;
}
static void layout(NSWindow *window) {[window.contentView layoutSubtreeIfNeeded];[NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.25]];[window displayIfNeeded];}
static void clickable(NSButton *b,NSWindow *window) {
    NSCAssert(b && b.enabled,@"missing or disabled button");NSRect r=[b convertRect:b.bounds toView:window.contentView];
    NSCAssert(r.size.width>30 && r.size.height>15 && NSContainsRect(window.contentView.bounds,r),@"button outside sheet: %@ %@",b.title,NSStringFromRect(r));
    NSView *hit=[window.contentView hitTest:NSMakePoint(NSMidX(r),NSMidY(r))];
    NSCAssert(hit==b || [hit isDescendantOf:b],@"button is drawn outside its actual hit area: %@ hit %@",b.title,hit);
}
static void mouseClick(NSButton *button,NSWindow *window) {
    NSPoint p=[button convertPoint:NSMakePoint(NSMidX(button.bounds),NSMidY(button.bounds)) toView:nil];
    NSTimeInterval t=NSProcessInfo.processInfo.systemUptime;
    NSEvent *up=[NSEvent mouseEventWithType:NSEventTypeLeftMouseUp location:p modifierFlags:0 timestamp:t+0.01 windowNumber:window.windowNumber context:nil eventNumber:2 clickCount:1 pressure:0];
    [NSApp postEvent:up atStart:YES];
    [window sendEvent:[NSEvent mouseEventWithType:NSEventTypeLeftMouseDown location:p modifierFlags:0 timestamp:t windowNumber:window.windowNumber context:nil eventNumber:1 clickCount:1 pressure:1]];
}
static void snapshot(NSWindow *window,NSString *name) {
    NSString *folder=NSProcessInfo.processInfo.environment[@"TRAMES_MANAGER_UI_OUTPUT"];if(!folder)return;
    [NSFileManager.defaultManager createDirectoryAtPath:folder withIntermediateDirectories:YES attributes:nil error:nil];
    NSView *view=window.sheetParent?window.contentView.subviews.firstObject:window.contentView;NSBitmapImageRep *bitmap=[view bitmapImageRepForCachingDisplayInRect:view.bounds];[view cacheDisplayInRect:view.bounds toBitmapImageRep:bitmap];
    [[bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:[folder stringByAppendingPathComponent:[name stringByAppendingString:@".png"]] atomically:YES];
}
static void booleanValue(id value,BOOL expected) {
    NSCAssert([value isKindOfClass:NSNumber.class] && CFGetTypeID((__bridge CFTypeRef)value)==CFBooleanGetTypeID() && [value boolValue]==expected,@"boolean became an integer: %@ (%@)",value,[value class]);
}
static void runTests(void) {@autoreleasepool {
    TMLayoutFixture *m=[TMLayoutFixture new];[m initializeModel];
    NSDictionary *config=@{@"environment":@{@"uuid":@"fixture",@"type":@"linux"},@"image":@{@"distribution":@"debian"},@"boot":@{@"mode":@"shell"},@"resources":@{@"cpus":@"all",@"memory":@"auto",@"memory_limit":NSNull.null},@"display":@{@"windows":@YES},@"components":@{@"audio":@YES,@"audio_input":@NO,@"vulkan":@YES,@"vaapi":@YES},@"interop":@{@"clipboard":@{@"enabled":@YES},@"host_cli":@{@"enabled":@NO}}};
    NSDictionary *row=@{@"name":@"debian",@"path":@"fixture",@"revision":@"base",@"profile":@"env.toml",@"source":@"host",@"state":@"stopped",@"autostart":@NO,@"config":config,@"grants":@{@"clipboard":@NO,@"host_cli":@NO}};
    NSMutableArray *fields=[NSMutableArray new];for(NSArray *entry in @[@[@"resources.cpus",@"CPU cores",@"resource"],@[@"resources.memory",@"Memory",@"resource"],@[@"resources.memory_limit",@"Optional hard limit",@"resource"],@[@"display.windows",@"Application windows",@"bool"],@[@"components.audio",@"Audio output",@"bool"],@[@"components.audio_input",@"Microphone input",@"bool"],@[@"components.vulkan",@"Vulkan",@"switch"],@[@"components.vaapi",@"Video acceleration",@"switch"]])
        [fields addObject:@{@"path":entry[0],@"title":entry[1],@"kind":entry[2],@"section":@"Compute",@"help":@"",@"options":@[@"true",@"false",@"auto"]}];
    m.snapshot=@{@"global":@{@"default_environment":@"debian"},@"global_path":@"config.toml",@"fields":fields,@"capabilities":@[@{@"key":@"clipboard",@"title":@"Clipboard",@"help":@"Exchange text",@"command":@"clipboard"},@{@"key":@"host_cli",@"title":@"Trames CLI",@"help":@"Manage this environment",@"command":@"host-cli"}]};
    m.rows=@[row];m.selectedKey=@"fixture";[m applicationDidFinishLaunching:[NSNotification notificationWithName:NSApplicationDidFinishLaunchingNotification object:NSApp]];m.tabs.selectedSegment=1;[m render];layout(m.window);
    NSCAssert(m.table.tableColumns[0].width>=240,@"environment column did not grow with the sidebar");
    for(NSView *view in descendants(m.window.contentView))if([view isKindOfClass:NSScrollView.class]){NSScrollView *scroll=(NSScrollView *)view;NSCAssert(scroll.autohidesScrollers && scroll.scrollerStyle==NSScrollerStyleOverlay,@"oversized permanent scrollbar");}
    NSCAssert(m.status.stringValue.length==0,@"unwanted closing-manager footer");
    for(NSString *path in @[@"display.windows",@"components.audio",@"components.audio_input"]){NSButton *b=identified(m.content,path);for(NSNumber *state in @[@YES,@NO]){b.state=[state boolValue]?NSControlStateValueOn:NSControlStateValueOff;[m fieldChanged:b];booleanValue([m draft][path],[state boolValue]);}}
    for(NSString *path in @[@"components.vulkan",@"components.vaapi"]){NSPopUpButton *p=identified(m.content,path);[p selectItemAtIndex:1];[m fieldChanged:p];booleanValue([m draft][path],NO);}
    NSComboBox *cpus=identified(m.content,@"resources.cpus");NSCAssert([cpus isKindOfClass:NSComboBox.class] && cpus.numberOfItems>1,@"missing CPU presets");[cpus selectItemAtIndex:2];[m comboBoxSelectionDidChange:[NSNotification notificationWithName:NSComboBoxSelectionDidChangeNotification object:cpus]];NSCAssert([[m draft][@"resources.cpus"] isEqual:@2],@"CPU preset lost its numeric value");
    NSComboBox *memory=identified(m.content,@"resources.memory");memory.stringValue=@"6GiB";NSCAssert([TMResourceValue(memory) isEqual:@"6GiB"],@"custom memory value lost");
    NSComboBox *limit=identified(m.content,@"resources.memory_limit");NSCAssert(TMResourceValue(limit)==NSNull.null,@"no-limit preset is not null");
    [m saveConfiguration:nil];for(NSString *path in @[@"display.windows",@"components.audio",@"components.audio_input",@"components.vulkan",@"components.vaapi"])booleanValue(m.lastBody[@"changes"][path],NO);
    snapshot(m.window,@"configuration");
    [m.drafts removeAllObjects];[m.revisions removeAllObjects];m.page=@"settings";[m render];layout(m.window);NSButton *login=identified(m.content,@"fixture");NSCAssert([login isKindOfClass:NSButton.class],@"login setting is not a checkbox");[login performClick:nil];NSCAssert(([m.lastArguments isEqual:@[@"env",@"enable",@"debian"]]),@"login checkbox did not enable");[login performClick:nil];NSCAssert(([m.lastArguments isEqual:@[@"env",@"disable",@"debian"]]),@"login checkbox did not disable");snapshot(m.window,@"settings");
    id previousStyle=[NSUserDefaults.standardUserDefaults objectForKey:@"interfaceStyle"];
    NSPopUpButton *appearance=identified(m.content,@"interface-style");NSCAssert(appearance.numberOfItems==3,@"missing appearance choices");
    for(NSString *style in @[@"light",@"dark",@"system"]){
        for(NSMenuItem *item in appearance.itemArray)if([item.representedObject isEqual:style])[appearance selectItem:item];
        [NSApp sendAction:appearance.action to:appearance.target from:appearance];layout(m.window);
        NSCAssert([TMInterfaceStyle() isEqual:style],@"appearance choice not persisted");
        if([style isEqual:@"system"])NSCAssert(NSApp.appearance==nil,@"system appearance should inherit macOS");
        else {NSString *resolved=[m.window.effectiveAppearance bestMatchFromAppearancesWithNames:@[NSAppearanceNameAqua,NSAppearanceNameDarkAqua]];NSString *expected=[style isEqual:@"dark"]?NSAppearanceNameDarkAqua:NSAppearanceNameAqua;NSCAssert([resolved isEqual:expected],@"window appearance did not update live");}
        snapshot(m.window,[@"settings-" stringByAppendingString:style]);
    }
    if(previousStyle)[NSUserDefaults.standardUserDefaults setObject:previousStyle forKey:@"interfaceStyle"];else[NSUserDefaults.standardUserDefaults removeObjectForKey:@"interfaceStyle"];TMApplyInterfaceStyle();

    m.page=@"env";m.tabs.selectedSegment=3;[m render];layout(m.window);NSCAssert(!button(m.content,@"Requested"),@"redundant request control remains");NSPopUpButton *permission=identified(m.content,@"clipboard");NSCAssert(permission.indexOfSelectedItem==0,@"grant without enabled permission displayed as allowed");[m change:@"components.audio" value:@NO];[permission selectItemAtIndex:1];[m permission:permission];NSCAssert([m.revisions[@"fixture"] isEqual:@"permission-saved"] && [[m draft][@"components.audio"] isEqual:@NO],@"permission action lost or invalidated an unrelated draft");NSCAssert(([m.lastArguments isEqual:@[@"_manager",@"permission",@"fixture",@"clipboard",@"on",@"base"]]),@"permission action did not update the combined setting");snapshot(m.window,@"permissions");
    for(NSNumber *width in @[@1120,@960]){[m.window setContentSize:NSMakeSize(width.doubleValue,720)];layout(m.window);[m create:nil];id controller=m.createController;NSWindow *sheet=[controller window];[sheet makeKeyWindow];layout(sheet);NSCAssert(button(sheet.contentView,@"Cancel").frame.size.width<160,@"wizard action button stretched across the panel");NSStackView *body=[controller valueForKey:@"body"];NSView *heading=body.arrangedSubviews.firstObject;NSCAssert(NSMaxY(heading.frame)>body.bounds.size.height-64,@"sheet content starts halfway down the panel");
        NSButton *source=button(sheet.contentView,@"Debian — daily tools and optional systemd");clickable(source,sheet);if(NSApp.active && sheet.keyWindow)mouseClick(source,sheet);else{puts("SKIP: OS mouse dispatch requires an active desktop; validating native hit targets and control actions");[source performClick:nil];}layout(sheet);NSCAssert([[controller valueForKey:@"kind"] isEqual:@"debian"],@"source choice did not respond");snapshot(sheet,@"create-source");
        NSButton *next=button(sheet.contentView,@"Continue");clickable(next,sheet);[next performClick:nil];layout(sheet);[[controller valueForKey:@"name"] setStringValue:@"layout-test"];
        NSCAssert([[controller valueForKey:@"cpus"] isKindOfClass:NSComboBox.class] && [[controller valueForKey:@"memory"] isKindOfClass:NSComboBox.class],@"wizard has raw resource fields");snapshot(sheet,@"create-configure");next=button(sheet.contentView,@"Continue");clickable(next,sheet);[next performClick:nil];layout(sheet);clickable(button(sheet.contentView,@"Create Environment"),sheet);snapshot(sheet,@"create-review");[button(sheet.contentView,@"Cancel") performClick:nil];NSCAssert(!m.createController,@"wizard did not close");
    }
    [m.window orderOut:nil];puts("PASS: manager layout, sheet hit targets, boolean JSON, resource presets/custom values, login checkbox and combined permissions");
}}
int main(void) {@autoreleasepool {
    [NSApplication sharedApplication];
    dispatch_async(dispatch_get_main_queue(),^{runTests();[NSApp stop:nil];[NSApp postEvent:[NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:0 data1:0 data2:0] atStart:NO];});
    [NSApp run];return 0;
}}
