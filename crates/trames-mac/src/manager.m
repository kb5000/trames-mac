// SPDX-License-Identifier: GPL-3.0-or-later
#import "manager-ui.h"
#include <fcntl.h>
#include <unistd.h>

@implementation TMJob
@end
@implementation TMButton
@end
@interface TMPanel : NSView
@end
@implementation TMPanel
- (void)drawRect:(NSRect)rect {
    (void)rect;NSBezierPath *p=[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(self.bounds,0.5,0.5) xRadius:9 yRadius:9];
    [NSColor.controlBackgroundColor setFill];[p fill];[NSColor.separatorColor setStroke];p.lineWidth=0.5;[p stroke];
}
- (void)viewDidChangeEffectiveAppearance {[super viewDidChangeEffectiveAppearance];self.needsDisplay=YES;}
@end
// Fill is explicit: NSLayoutAttributeWidth is not a valid StackView alignment.
@interface TMStackView : NSStackView
@property BOOL fillsWidth;
@property NSMutableArray<NSLayoutConstraint *> *fillConstraints;
@end
@implementation TMStackView
- (void)updateFill {
    [NSLayoutConstraint deactivateConstraints:self.fillConstraints?:@[]];
    self.fillConstraints=[NSMutableArray new];if(!self.fillsWidth)return;
    for(NSView *view in self.arrangedSubviews)[self.fillConstraints addObject:[view.widthAnchor constraintEqualToAnchor:self.widthAnchor constant:-(self.edgeInsets.left+self.edgeInsets.right)]];
    [NSLayoutConstraint activateConstraints:self.fillConstraints];
}
- (void)setAlignment:(NSLayoutAttribute)value {self.fillsWidth=value==NSLayoutAttributeWidth;[super setAlignment:self.fillsWidth?NSLayoutAttributeLeading:value];[self updateFill];}
- (void)setEdgeInsets:(NSEdgeInsets)value {[super setEdgeInsets:value];[self updateFill];}
- (void)addArrangedSubview:(NSView *)view {[super addArrangedSubview:view];[self updateFill];}
- (void)removeArrangedSubview:(NSView *)view {[super removeArrangedSubview:view];[self updateFill];}
@end
@interface TMTopClipView : NSClipView
@end
@implementation TMTopClipView
- (NSRect)constrainBoundsRect:(NSRect)proposed {
    NSRect bounds=[super constrainBoundsRect:proposed];NSView *document=self.documentView;
    if(document && !document.isFlipped && document.frame.size.height<bounds.size.height)bounds.origin.y=NSMaxY(document.frame)-bounds.size.height;
    return bounds;
}
@end
@interface TMBackground : NSView
@end
@implementation TMBackground
- (void)drawRect:(NSRect)rect{[NSColor.windowBackgroundColor setFill];NSRectFill(rect);}
- (void)viewDidChangeEffectiveAppearance{[super viewDidChangeEffectiveAppearance];self.needsDisplay=YES;}
@end
NSTextField *TMLabel(NSString *text,CGFloat size,BOOL secondary) {
    NSTextField *v=[NSTextField wrappingLabelWithString:text?:@""];v.font=[NSFont systemFontOfSize:size];
    v.textColor=secondary?NSColor.secondaryLabelColor:NSColor.labelColor;v.selectable=NO;return v;
}
NSStackView *TMStack(NSArray<NSView *> *views,BOOL vertical) {
    NSStackView *v=[TMStackView stackViewWithViews:views];v.orientation=vertical?NSUserInterfaceLayoutOrientationVertical:NSUserInterfaceLayoutOrientationHorizontal;
    v.distribution=NSStackViewDistributionFill;v.spacing=10;v.alignment=vertical?NSLayoutAttributeLeading:NSLayoutAttributeCenterY;return v;
}
TMButton *TMButtonMake(NSString *title,id target,SEL action,id payload) {
    TMButton *b=[TMButton buttonWithTitle:T(title) target:target action:action];b.bezelStyle=NSBezelStyleRounded;b.payload=payload;return b;
}
static NSView *spacer(void) {NSView *v=[NSView new];[v setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationHorizontal];return v;}
void TMPin(NSView *child,NSView *parent,CGFloat inset) {
    child.translatesAutoresizingMaskIntoConstraints=NO;[parent addSubview:child];
    [NSLayoutConstraint activateConstraints:@[[child.leadingAnchor constraintEqualToAnchor:parent.leadingAnchor constant:inset],[child.trailingAnchor constraintEqualToAnchor:parent.trailingAnchor constant:-inset],[child.topAnchor constraintEqualToAnchor:parent.topAnchor constant:inset],[child.bottomAnchor constraintEqualToAnchor:parent.bottomAnchor constant:-inset]]];
}
void TMConfigureScroll(NSScrollView *scroll) {
    scroll.hasVerticalScroller=YES;scroll.hasHorizontalScroller=NO;
    scroll.scrollerStyle=NSScrollerStyleOverlay;scroll.autohidesScrollers=YES;
    scroll.verticalScroller.controlSize=NSControlSizeSmall;scroll.drawsBackground=NO;
}
NSComboBox *TMResourceCombo(NSString *path,id value) {
    NSComboBox *combo=[NSComboBox new];combo.identifier=path;combo.editable=YES;combo.completes=YES;
    NSMutableArray *options=[NSMutableArray new];NSString *display=TMString(value);
    if([path isEqual:@"resources.cpus"]){
        [options addObject:T(@"All cores")];for(NSUInteger i=1;i<=NSProcessInfo.processInfo.activeProcessorCount;i++)[options addObject:[@(i) stringValue]];
        if([display isEqual:@"all"])display=T(@"All cores");
    }else{
        BOOL limit=[path isEqual:@"resources.memory_limit"];
        [options addObject:T(limit?@"No additional limit":@"Automatic (shared with Mac)")];
        [options addObjectsFromArray:@[@"1GiB",@"2GiB",@"4GiB",@"8GiB",@"16GiB",@"32GiB",@"64GiB"]];
        if((limit && value==NSNull.null)||(!limit && [display isEqual:@"auto"]))display=options[0];
    }
    [combo addItemsWithObjectValues:options];combo.stringValue=display;
    combo.numberOfVisibleItems=MIN(10,options.count);[combo.widthAnchor constraintEqualToConstant:210].active=YES;return combo;
}
id TMResourceValue(NSComboBox *combo) {
    NSString *text=[combo.stringValue stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
    if([combo.identifier isEqual:@"resources.cpus"]){
        if([text isEqual:T(@"All cores")] || [text isEqual:@"all"])return @"all";
        NSScanner *scan=[NSScanner scannerWithString:text];long long n;if([scan scanLongLong:&n]&&scan.isAtEnd)return @(n);
    }else{
        if([text isEqual:T(@"Automatic (shared with Mac)")])return @"auto";
        if([combo.identifier isEqual:@"resources.memory_limit"]&&(!text.length || [text isEqual:T(@"No additional limit")]))return NSNull.null;
    }
    return text;
}
NSView *TMGroup(NSArray<NSView *> *rows) {
    TMPanel *panel=[TMPanel new];NSStackView *stack=TMStack(@[],YES);stack.spacing=0;stack.alignment=NSLayoutAttributeWidth;
    for(NSView *row in rows){if(stack.arrangedSubviews.count){NSBox *line=[NSBox new];line.boxType=NSBoxSeparator;[stack addArrangedSubview:line];}[stack addArrangedSubview:row];}
    TMPin(stack,panel,1);return panel;
}
NSView *TMRow(NSString *title,NSString *help,NSView *value) {
    NSTextField *label=TMLabel(T(title),13,NO);NSStackView *left=TMStack(@[label],YES);left.spacing=3;
    if(help.length)[left addArrangedSubview:TMLabel(T(help),11,YES)];
    [left setContentCompressionResistancePriority:250 forOrientation:NSLayoutConstraintOrientationHorizontal];
    NSStackView *row=TMStack(@[left,spacer(),value],NO);row.edgeInsets=NSEdgeInsetsMake(12,14,12,14);row.spacing=12;
    [value setContentCompressionResistancePriority:750 forOrientation:NSLayoutConstraintOrientationHorizontal];return row;
}
id TMValue(NSDictionary *object,NSString *path) {
    id v=object;for(NSString *key in [path componentsSeparatedByString:@"."]){if(![v isKindOfClass:NSDictionary.class])return NSNull.null;v=v[key];}return v?:NSNull.null;
}
NSString *TMString(id value) {
    if(!value || value==NSNull.null)return @"";
    if([value isKindOfClass:NSString.class])return value;
    if([value isKindOfClass:NSNumber.class])return CFGetTypeID((__bridge CFTypeRef)value)==CFBooleanGetTypeID()?([value boolValue]?@"true":@"false"):[value stringValue];
    NSData *data=[NSJSONSerialization dataWithJSONObject:value options:NSJSONWritingPrettyPrinted error:nil];return [[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding]?:@"";
}
static NSString *productName(void){return [NSBundle.mainBundle objectForInfoDictionaryKey:@"CFBundleDisplayName"]?:@"Trames";}
static NSString *executable(void){NSString *p=NSBundle.mainBundle.executablePath?:NSProcessInfo.processInfo.arguments.firstObject;return [p.lastPathComponent isEqual:@"trames-manager"]?[p.stringByDeletingLastPathComponent stringByAppendingPathComponent:@"trames-mac-run"]:p;}
static BOOL isRunning(NSDictionary *row){return [row[@"state"] hasPrefix:@"running"];}
static NSString *systemName(NSDictionary *row){NSDictionary *c=row[@"config"];if(![c isKindOfClass:NSDictionary.class])return [row[@"state"] isEqual:@"Preparing environment…"]?T(@"Preparing environment…"):T(@"Configuration needs attention.");NSString *d=c[@"image"][@"distribution"];return [NSString stringWithFormat:@"%@ · %@",[d capitalizedString],c[@"boot"][@"mode"]];}

@implementation TramesManager
- (void)initializeModel {
    if(self.jobs)return;self.rows=@[];self.snapshot=@{};self.jobs=[NSMutableDictionary new];self.drafts=[NSMutableDictionary new];self.revisions=[NSMutableDictionary new];self.pendingRestart=[NSMutableSet new];self.sessions=[NSMutableDictionary new];self.watches=[NSMutableArray new];self.page=@"env";
}
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)app{(void)app;return YES;}
- (void)applicationDidBecomeActive:(NSNotification *)note{(void)note;if(self.window && !NSApp.modalWindow){[self.sessions removeObjectForKey:self.selectedKey?:@""];[self refresh:nil];}}
- (void)applicationDidFinishLaunching:(NSNotification *)note {
    (void)note;[self initializeModel];[NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSImage *icon=[[NSImage alloc]initWithContentsOfURL:[NSBundle.mainBundle URLForResource:@"Trames" withExtension:@"icns"]];if(icon)NSApp.applicationIconImage=icon;
    NSMenu *menu=[NSMenu new];NSMenuItem *app=[NSMenuItem new];[menu addItem:app];NSMenu *sub=[NSMenu new];[sub addItemWithTitle:[NSString stringWithFormat:T(@"Quit %@"),productName()] action:@selector(terminate:) keyEquivalent:@"q"];app.submenu=sub;
    NSMenuItem *edit=[NSMenuItem new];[menu addItem:edit];edit.submenu=[[NSMenu alloc]initWithTitle:T(@"Edit")];
    NSArray *names=@[@"Undo",@"Redo",@"Cut",@"Copy",@"Paste",@"Select All"];SEL sels[]={@selector(undo:),@selector(redo:),@selector(cut:),@selector(copy:),@selector(paste:),@selector(selectAll:)};NSArray *keys=@[@"z",@"Z",@"x",@"c",@"v",@"a"];
    for(NSUInteger i=0;i<names.count;i++)[edit.submenu addItemWithTitle:T(names[i]) action:sels[i] keyEquivalent:keys[i]];NSApp.mainMenu=menu;
    self.window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,1120,760) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];self.window.title=productName();self.window.minSize=NSMakeSize(960,610);self.window.releasedWhenClosed=NO;[self.window center];
    NSSplitView *split=[[NSSplitView alloc]initWithFrame:self.window.contentView.bounds];split.vertical=YES;split.dividerStyle=NSSplitViewDividerStyleThin;split.delegate=self;split.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;[self.window.contentView addSubview:split];
    NSVisualEffectView *side=[[NSVisualEffectView alloc]initWithFrame:NSMakeRect(0,0,280,760)];side.material=NSVisualEffectMaterialSidebar;side.blendingMode=NSVisualEffectBlendingModeBehindWindow;
    NSStackView *sidebar=TMStack(@[],YES);sidebar.alignment=NSLayoutAttributeWidth;sidebar.edgeInsets=NSEdgeInsetsMake(18,12,14,12);sidebar.spacing=12;TMPin(sidebar,side,0);
    NSStackView *brand=TMStack(@[],NO);if(icon){NSImageView *image=[NSImageView imageViewWithImage:icon];[image.widthAnchor constraintEqualToConstant:28].active=YES;[image.heightAnchor constraintEqualToConstant:28].active=YES;[brand addArrangedSubview:image];}[brand addArrangedSubview:TMLabel(productName(),16,NO)];[sidebar addArrangedSubview:brand];
    [sidebar addArrangedSubview:TMLabel(T(@"Environments"),11,YES)];
    self.table=[NSTableView new];NSTableColumn *column=[[NSTableColumn alloc]initWithIdentifier:@"environment"];column.width=250;column.minWidth=100;column.resizingMask=NSTableColumnAutoresizingMask;[self.table addTableColumn:column];self.table.columnAutoresizingStyle=NSTableViewLastColumnOnlyAutoresizingStyle;self.table.headerView=nil;self.table.rowHeight=60;self.table.backgroundColor=NSColor.clearColor;self.table.style=NSTableViewStyleSourceList;self.table.delegate=self;self.table.dataSource=self;self.table.allowsEmptySelection=NO;self.table.target=self;self.table.action=@selector(selectEnvironment:);
    NSScrollView *list=[NSScrollView new];list.documentView=self.table;TMConfigureScroll(list);[list.heightAnchor constraintGreaterThanOrEqualToConstant:140].active=YES;[sidebar addArrangedSubview:list];
    [sidebar addArrangedSubview:TMButtonMake(@"New Environment…",self,@selector(create:),nil)];
    for(NSArray *item in @[@[@"Networks",@"networks"],@[@"Trames Settings",@"settings"]])[sidebar addArrangedSubview:TMButtonMake(item[0],self,@selector(globalPage:),item[1])];
    [sidebar addArrangedSubview:TMButtonMake(@"Refresh",self,@selector(refresh:),nil)];
    self.sidebarStatus=TMLabel(T(@"Loading environments…"),11,YES);[sidebar addArrangedSubview:self.sidebarStatus];
    NSView *main=[[TMBackground alloc]initWithFrame:NSMakeRect(281,0,839,760)];NSStackView *body=TMStack(@[],YES);body.alignment=NSLayoutAttributeWidth;body.spacing=0;TMPin(body,main,0);
    self.header=TMStack(@[],YES);self.header.alignment=NSLayoutAttributeWidth;self.header.edgeInsets=NSEdgeInsetsMake(20,24,6,24);[body addArrangedSubview:self.header];
    self.tabs=[NSSegmentedControl segmentedControlWithLabels:@[T(@"Overview"),T(@"Configuration"),T(@"Files and Sharing"),T(@"Host Interop"),T(@"Activity")] trackingMode:NSSegmentSwitchTrackingSelectOne target:self action:@selector(tabChanged:)];self.tabs.selectedSegment=0;
    NSView *tabContainer=[NSView new];NSStackView *tabRow=TMStack(@[self.tabs,spacer()],NO);tabRow.edgeInsets=NSEdgeInsetsMake(16,24,12,24);TMPin(tabRow,tabContainer,0);[body addArrangedSubview:tabContainer];
    self.scroll=[NSScrollView new];self.scroll.contentView=[TMTopClipView new];TMConfigureScroll(self.scroll);self.content=TMStack(@[],YES);self.content.spacing=20;self.content.alignment=NSLayoutAttributeWidth;self.content.edgeInsets=NSEdgeInsetsMake(10,24,24,24);self.content.translatesAutoresizingMaskIntoConstraints=NO;self.scroll.documentView=self.content;[self.content.widthAnchor constraintEqualToAnchor:self.scroll.contentView.widthAnchor].active=YES;[body addArrangedSubview:self.scroll];[self.scroll.heightAnchor constraintGreaterThanOrEqualToConstant:300].active=YES;
    self.status=TMLabel(@"",11,YES);self.saveButton=TMButtonMake(@"Check and Save",self,@selector(saveConfiguration:),nil);self.saveButton.keyEquivalent=@"s";self.discardButton=TMButtonMake(@"Discard",self,@selector(discardConfiguration:),nil);
    NSStackView *footer=TMStack(@[self.status,spacer(),self.discardButton,self.saveButton],NO);footer.edgeInsets=NSEdgeInsetsMake(10,20,12,20);[body addArrangedSubview:footer];
    [split addSubview:side];[split addSubview:main];[split setPosition:280 ofDividerAtIndex:0];
    [self.window makeKeyAndOrderFront:nil];[NSApp activate];[self render];[self refresh:nil];
}
- (CGFloat)splitView:(NSSplitView *)v constrainMinCoordinate:(CGFloat)p ofSubviewAt:(NSInteger)i{(void)v;(void)p;(void)i;return 250;}
- (CGFloat)splitView:(NSSplitView *)v constrainMaxCoordinate:(CGFloat)p ofSubviewAt:(NSInteger)i{(void)v;(void)p;(void)i;return 380;}
- (BOOL)splitView:(NSSplitView *)v shouldAdjustSizeOfSubview:(NSView *)s{ return s!=v.subviews.firstObject; }
- (NSInteger)numberOfRowsInTableView:(NSTableView *)table{(void)table;return self.rows.count;}
- (NSView *)tableView:(NSTableView *)table viewForTableColumn:(NSTableColumn *)col row:(NSInteger)index {
    (void)table;(void)col;NSDictionary *row=self.rows[index];NSView *cell=[NSView new];
    NSString *symbol=[TMValue(row,@"config.environment.type") isEqual:@"android"]?@"iphone":@"terminal";NSImageView *image=[NSImageView imageViewWithImage:[NSImage imageWithSystemSymbolName:symbol accessibilityDescription:nil]];
    [image.widthAnchor constraintEqualToConstant:23].active=YES;image.contentTintColor=NSColor.secondaryLabelColor;
    NSString *subtitle=systemName(row);if([row[@"name"] isEqual:TMValue(self.snapshot,@"global.default_environment")])subtitle=[T(@"Default") stringByAppendingFormat:@" · %@",subtitle];
    NSTextField *name=TMLabel(row[@"name"],13,NO),*detail=TMLabel(subtitle,11,YES);
    for(NSTextField *label in @[name,detail]){label.maximumNumberOfLines=1;label.lineBreakMode=NSLineBreakByTruncatingTail;[label setContentCompressionResistancePriority:250 forOrientation:NSLayoutConstraintOrientationHorizontal];}
    NSStackView *names=TMStack(@[name,detail],YES);names.spacing=3;[names setContentCompressionResistancePriority:250 forOrientation:NSLayoutConstraintOrientationHorizontal];
    NSTextField *dot=TMLabel(@"●",10,YES);[dot.widthAnchor constraintEqualToConstant:12].active=YES;dot.textColor=isRunning(row)?NSColor.systemGreenColor:NSColor.tertiaryLabelColor;
    NSStackView *line=TMStack(@[image,names,spacer(),dot],NO);line.spacing=8;line.edgeInsets=NSEdgeInsetsMake(7,6,7,6);TMPin(line,cell,0);return cell;
}
- (NSString *)keyForRow:(NSDictionary *)row {id uuid=TMValue(row,@"config.environment.uuid");return [uuid isKindOfClass:NSString.class]&&[uuid length]?uuid:row[@"path"]?:row[@"name"];}
- (NSDictionary *)selectedRow {for(NSDictionary *r in self.rows)if([[self keyForRow:r] isEqual:self.selectedKey])return r;return nil;}
- (NSString *)selected{return [self selectedRow][@"name"];}
- (NSMutableDictionary *)draft {
    if(!self.selectedKey)return nil;NSMutableDictionary *d=self.drafts[self.selectedKey];if(!d){d=[NSMutableDictionary new];self.drafts[self.selectedKey]=d;}return d;
}
- (void)tableViewSelectionDidChange:(NSNotification *)note {
    (void)note;if(self.rendering)return;NSInteger i=self.table.selectedRow;if(i<0||i>=(NSInteger)self.rows.count)return;
    self.selectedKey=[self keyForRow:self.rows[i]];self.page=@"env";[NSUserDefaults.standardUserDefaults setObject:self.selectedKey forKey:@"ManagerSelectedEnvironment"];[self watchRegistry];[self render];
}
- (void)selectEnvironment:(id)sender{(void)sender;if(![self.page isEqual:@"env"])[self tableViewSelectionDidChange:[NSNotification notificationWithName:NSTableViewSelectionDidChangeNotification object:self.table]];}
- (void)globalPage:(TMButton *)sender{self.page=sender.payload;[self render];}
- (void)tabChanged:(id)sender{(void)sender;self.page=@"env";[self render];}
- (BOOL)hasWork:(NSString *)key {TMJob *job=self.jobs[key];return job && !job.complete;}
- (void)showError:(NSString *)message {if([message containsString:@"changed outside the manager"]){NSAlert *conflict=[NSAlert new];conflict.messageText=T(@"Configuration changed outside the manager");conflict.informativeText=T(@"Reload the latest configuration and review your draft again before saving.");[conflict addButtonWithTitle:T(@"Reload and Review")];[conflict addButtonWithTitle:T(@"Cancel")];NSString *key=self.selectedKey;__weak TramesManager *weakSelf=self;[conflict beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse response){if(response==NSAlertFirstButtonReturn){[weakSelf.revisions removeObjectForKey:key];[weakSelf refresh:nil];}}];return;}self.status.stringValue=message?:T(@"Operation failed.");NSAlert *a=[NSAlert new];a.messageText=T(@"Operation failed");a.informativeText=message?:@"";[a addButtonWithTitle:T(@"OK")];[a beginSheetModalForWindow:self.window completionHandler:nil];}
- (BOOL)confirm:(NSString *)title message:(NSString *)message {NSAlert *a=[NSAlert new];a.messageText=T(title);a.informativeText=T(message);[a addButtonWithTitle:T(@"Continue")];[a addButtonWithTitle:T(@"Cancel")];return [a runModal]==NSAlertFirstButtonReturn;}
- (void)mergeCreationTasks {
    NSMutableArray *rows=[self.rows mutableCopy]?:[NSMutableArray new];
    for(TMJob *job in self.jobs.allValues)if([job.key hasPrefix:@"create:"] && (!job.complete || job.code)){
        BOOL found=NO;for(NSDictionary *row in rows)if([row[@"name"] isEqual:job.name]){found=YES;break;}
        if(!found)[rows addObject:@{@"name":job.name,@"path":job.key,@"config":NSNull.null,@"state":job.complete?@"needs attention":@"Preparing environment…",@"error":job.output?:@""}];
    }
    self.rows=rows;
}
- (void)run:(NSArray<NSString *> *)args owner:(NSString *)key name:(NSString *)name title:(NSString *)title quiet:(BOOL)quiet input:(NSData *)input completion:(void(^)(NSString *,int))done {
    [self initializeModel];if([self hasWork:key]){if(!quiet)[self showError:T(@"Another operation is running for this environment.")];return;}
    TMJob *job=[TMJob new];job.key=key;job.name=name;job.title=title;job.output=[NSMutableString new];job.quiet=quiet;self.jobs[key]=job;if([key hasPrefix:@"create:"]){self.selectedKey=key;[self mergeCreationTasks];BOOL wasRendering=self.rendering;self.rendering=YES;[self.table reloadData];for(NSUInteger i=0;i<self.rows.count;i++)if([[self keyForRow:self.rows[i]] isEqual:key])[self.table selectRowIndexes:[NSIndexSet indexSetWithIndex:i] byExtendingSelection:NO];self.rendering=wasRendering;}
    NSTask *task=[NSTask new];task.executableURL=[NSURL fileURLWithPath:executable()];task.arguments=args;task.standardInput=[NSFileHandle fileHandleWithNullDevice];job.task=task;
    if(input){NSURL *temp=[NSURL fileURLWithPath:[NSTemporaryDirectory() stringByAppendingPathComponent:NSUUID.UUID.UUIDString]];NSError *error=nil;if(![input writeToURL:temp options:NSDataWritingAtomic error:&error]){[self.jobs removeObjectForKey:key];[self showError:error.localizedDescription];return;}task.standardInput=[NSFileHandle fileHandleForReadingFromURL:temp error:&error];[NSFileManager.defaultManager removeItemAtURL:temp error:nil];if(!task.standardInput){[self.jobs removeObjectForKey:key];[self showError:error.localizedDescription];return;}}
    __weak TramesManager *weakSelf=self;
    job.stream=[[TramesTaskStream alloc]initWithTask:task onText:^(NSString *text){
        [job.output appendString:text];if(job.output.length>2*1024*1024)[job.output deleteCharactersInRange:NSMakeRange(0,job.output.length-2*1024*1024)];
        TramesManager *owner=weakSelf;
        if(!quiet && [owner.selectedKey isEqual:key] && owner.log){[owner.log.textStorage appendAttributedString:[[NSAttributedString alloc]initWithString:text attributes:@{NSFontAttributeName:owner.log.font}]];[owner.log scrollRangeToVisible:NSMakeRange(owner.log.string.length,0)];}
    } completion:^(NSString *out,int code){
        job.code=code;job.complete=YES;job.task=nil;job.stream=nil;
        TramesManager *owner=weakSelf;
        if(quiet)[owner.jobs removeObjectForKey:key];
        if(!quiet){owner.status.stringValue=code?T(@"Operation failed. See Activity for details."):T(@"Done.");}
        if(done)done(out,code);
        if(!quiet){[owner.sessions removeObjectForKey:key];[owner render];[owner refresh:nil];}
    }];
    NSError *error=nil;if(![job.stream launchAndReturnError:&error]){job.complete=YES;job.code=-1;job.task=nil;job.stream=nil;[job.output appendString:error.localizedDescription];if(quiet)[self.jobs removeObjectForKey:key];if(done)done(error.localizedDescription,-1);[self showError:error.localizedDescription];}
    if(!quiet){
        BOOL lengthy=[key hasPrefix:@"create:"] || (args.count>1 && [args[0] isEqual:@"env"] && [@[@"create",@"restore",@"import-store",@"backup",@"packages",@"setup",@"start",@"stop",@"remove"] containsObject:args[1]]) || [args containsObject:@"apk"];
        if(lengthy && [key isEqual:self.selectedKey]){self.page=@"env";self.tabs.selectedSegment=4;}[self render];
    }
}
- (void)operation:(NSArray<NSString *> *)args title:(NSString *)title {
    NSString *key=self.selectedKey?:@"global-operation";NSString *name=[self selected]?:@"";
    [self run:args owner:key name:name title:title quiet:NO input:nil completion:nil];
}
- (void)refresh:(id)sender {
    (void)sender;[self initializeModel];if(self.refreshing){self.refreshAgain=YES;return;}self.refreshing=YES;
    __weak TramesManager *weakSelf=self;
    [self run:@[@"_manager",@"snapshot"] owner:@"query:snapshot" name:@"" title:T(@"Refreshing…") quiet:YES input:nil completion:^(NSString *out,int code){
        TramesManager *owner=weakSelf;owner.refreshing=NO;
        if(!code){NSError *error=nil;NSDictionary *snapshot=[NSJSONSerialization JSONObjectWithData:[out dataUsingEncoding:NSUTF8StringEncoding] options:0 error:&error];
            if([snapshot isKindOfClass:NSDictionary.class] && [snapshot[@"version"] isEqual:@1]){
                owner.snapshot=snapshot;owner.rows=snapshot[@"environments"];[owner mergeCreationTasks];
                if(!owner.selectedKey)owner.selectedKey=[NSUserDefaults.standardUserDefaults stringForKey:@"ManagerSelectedEnvironment"];
                if(owner.pendingSelectionName){for(NSDictionary *r in owner.rows)if([r[@"name"] isEqual:owner.pendingSelectionName] && [r[@"config"] isKindOfClass:NSDictionary.class]){owner.selectedKey=[owner keyForRow:r];owner.pendingSelectionName=nil;break;}}
                if(![owner selectedRow] && owner.rows.count){NSString *preferred=owner.snapshot[@"global"][@"default_environment"];NSDictionary *choice=owner.rows.firstObject;for(NSDictionary *r in owner.rows)if([r[@"name"] isEqual:preferred]){choice=r;break;}owner.selectedKey=[owner keyForRow:choice];}
                owner.rendering=YES;[owner.table reloadData];for(NSUInteger i=0;i<owner.rows.count;i++)if([[owner keyForRow:owner.rows[i]] isEqual:owner.selectedKey])[owner.table selectRowIndexes:[NSIndexSet indexSetWithIndex:i] byExtendingSelection:NO];owner.rendering=NO;
                // Native field editors own in-progress text. Do not replace
                // controls underneath them when an external file changes.
                if(![owner.window.firstResponder isKindOfClass:NSTextView.class] || owner.window.firstResponder==owner.log)[owner render];
                [owner watchRegistry];
            }else owner.status.stringValue=error.localizedDescription?:T(@"Could not read environment list.");
        }else owner.status.stringValue=out;
        if(owner.refreshAgain){owner.refreshAgain=NO;[owner refresh:nil];}
    }];
}
- (void)watchRegistry {
    for(dispatch_source_t source in self.watches)dispatch_source_cancel(source);[self.watches removeAllObjects];
    NSString *registry=self.snapshot[@"registry"];if(!registry)return;
    NSMutableArray *paths=[NSMutableArray arrayWithObjects:registry,[registry stringByAppendingPathComponent:@"environments"],nil];NSString *selected=[self selectedRow][@"path"];if(selected)[paths addObject:selected];
    __weak TramesManager *weakSelf=self;
    for(NSString *path in paths){int fd=open(path.fileSystemRepresentation,O_EVTONLY|O_CLOEXEC);if(fd<0)continue;
        dispatch_source_t source=dispatch_source_create(DISPATCH_SOURCE_TYPE_VNODE,(uintptr_t)fd,DISPATCH_VNODE_WRITE|DISPATCH_VNODE_DELETE|DISPATCH_VNODE_RENAME,dispatch_get_main_queue());
        dispatch_source_set_event_handler(source,^{[weakSelf refresh:nil];});dispatch_source_set_cancel_handler(source,^{close(fd);});[self.watches addObject:source];dispatch_resume(source);
    }
}
- (void)heading:(NSString *)title subtitle:(NSString *)subtitle actions:(NSArray *)actions {
    NSStackView *names=TMStack(@[TMLabel(title,23,NO),TMLabel(subtitle,12,YES)],YES);names.spacing=4;
    NSStackView *line=TMStack(@[names,spacer()],NO);for(NSView *v in actions)[line addArrangedSubview:v];[self.header addArrangedSubview:line];
}
- (void)section:(NSString *)title view:(NSView *)view {
    NSStackView *section=TMStack(@[TMLabel(T(title),13,NO),view],YES);section.alignment=NSLayoutAttributeWidth;section.spacing=9;[self.content addArrangedSubview:section];
}
- (id)value:(NSString *)path {id changed=[self draft][path];return changed?:TMValue([self selectedRow][@"config"],path);}
- (void)change:(NSString *)path value:(id)value {
    if(!self.revisions[self.selectedKey])self.revisions[self.selectedKey]=[self selectedRow][@"revision"]?:@"";
    [self draft][path]=value?:NSNull.null;[self updateFooter];
}
- (void)updateFooter {
    BOOL dirty=[self draft].count && [self.page isEqual:@"env"];self.saveButton.hidden=!dirty;self.discardButton.hidden=!dirty;
    BOOL busy=[self hasWork:self.selectedKey];self.saveButton.enabled=!busy;
    self.status.stringValue=dirty?T(@"Unsaved draft. Changes apply on the next start."):[self.pendingRestart containsObject:self.selectedKey]?T(@"Configuration saved. Applies on next start."):@"";
}
- (void)render {
    if(!self.content)return;self.rendering=YES;self.log=nil;for(NSView *v in self.header.arrangedSubviews.copy){[self.header removeArrangedSubview:v];[v removeFromSuperview];}for(NSView *v in self.content.arrangedSubviews.copy){[self.content removeArrangedSubview:v];[v removeFromSuperview];}
    NSUInteger running=0;for(NSDictionary *r in self.rows)if(isRunning(r))running++;self.sidebarStatus.stringValue=[NSString stringWithFormat:T(@"%lu environments running"),(unsigned long)running];
    self.tabs.superview.hidden=![self.page isEqual:@"env"];self.saveButton.hidden=YES;self.discardButton.hidden=YES;if(![self.page isEqual:@"env"])self.status.stringValue=@"";
    if([self.page isEqual:@"networks"]){[self renderNetworks];[self renderActiveTasks];}
    else if([self.page isEqual:@"settings"]){[self renderSettings];}
    else {NSDictionary *r=[self selectedRow];if(!r){[self heading:T(@"Your environments") subtitle:T(@"Create an environment from Linux, Android, a rootfs or a TOML profile.") actions:@[]];[self.content addArrangedSubview:TMButtonMake(@"New Environment…",self,@selector(create:),nil)];[self renderActiveTasks];}
        else {
            BOOL android=[TMValue(r,@"config.environment.type") isEqual:@"android"];
            TMButton *open=TMButtonMake(isRunning(r)?(android?@"Open Desktop":@"Open Terminal"):@"Start Environment",self,@selector(start:),nil);open.enabled=![self hasWork:self.selectedKey] && [r[@"config"] isKindOfClass:NSDictionary.class];
            NSMutableArray *actions=[NSMutableArray arrayWithObject:open];if(isRunning(r)){TMButton *stop=TMButtonMake(@"Stop",self,@selector(stop:),nil);stop.enabled=![self hasWork:self.selectedKey];[actions addObject:stop];}[actions addObject:TMButtonMake(@"More…",self,@selector(more:),nil)];
            [self heading:r[@"name"] subtitle:[NSString stringWithFormat:@"%@ · %@",systemName(r),T(r[@"state"]?:@"")] actions:actions];
            if(self.jobs[self.selectedKey] && ![r[@"config"] isKindOfClass:NSDictionary.class]){[self renderActivity];}
            else if(![r[@"config"] isKindOfClass:NSDictionary.class]){[self.content addArrangedSubview:TMLabel(r[@"error"],13,YES)];[self.content addArrangedSubview:TMButtonMake(@"Open Configuration in Editor",self,@selector(externalEditor:),nil)];}
            else if(self.tabs.selectedSegment==0)[self renderOverview];
            else if(self.tabs.selectedSegment==1)[self renderConfiguration];
            else if(self.tabs.selectedSegment==2)[self renderFiles];
            else if(self.tabs.selectedSegment==3)[self renderPermissions];
            else [self renderActivity];
        }
        [self updateFooter];
    }
    self.rendering=NO;
}
- (void)renderOverview {
    NSDictionary *r=[self selectedRow];NSMutableArray *rows=[NSMutableArray new];
    for(NSArray *f in @[@[@"CPU cores",@"resources.cpus"],@[@"Memory",@"resources.memory"],@[@"Network mode",@"network.mode"],@[@"Boot mode",@"boot.mode"]])[rows addObject:TMRow(f[0],@"",TMLabel(T(TMString(TMValue(r[@"config"],f[1]))),13,NO))];
    [self section:@"Saved configuration" view:TMGroup(rows)];[self.content addArrangedSubview:TMLabel(T(@"Memory is allocated on demand. Saved settings apply at the next environment start."),11,YES)];
    [self section:@"Configuration source" view:TMGroup(@[TMRow([r[@"source"] isEqual:@"host"]?@"Host env.toml":@"Guest /etc/trames/env.toml",T(r[@"source_note"]),TMButtonMake(@"View TOML",self,@selector(showTOML:),nil))])];
    if([TMValue(r,@"config.environment.type") isEqual:@"android"])[self section:@"Android workspace" view:TMGroup(@[TMRow(@"Application packages",@"Install an APK after Android finishes starting.",TMButtonMake(@"Install APK…",self,@selector(installAPK:),nil))])];
    else {
        NSMutableArray *terminals=[NSMutableArray arrayWithObject:TMRow(@"Independent terminals",@"Each open creates another session in the same environment.",TMButtonMake(@"New Terminal",self,@selector(newTerminal:),nil))];
        for(NSDictionary *session in self.sessions[self.selectedKey]){
            BOOL attached=[session[@"attached"] boolValue];TMButton *attach=TMButtonMake(attached?@"Connected":@"Reattach",self,@selector(attachSession:),session[@"id"]);attach.enabled=!attached && isRunning(r);
            [terminals addObject:TMRow([NSString stringWithFormat:T(@"Terminal %@"),session[@"id"]],TMString(session[@"state"]),attach)];
        }
        [self section:@"Terminal sessions" view:TMGroup(terminals)];
        if(isRunning(r) && !self.sessions[self.selectedKey]){__weak TramesManager *weakSelf=self;NSString *key=self.selectedKey;dispatch_async(dispatch_get_main_queue(),^{[weakSelf querySessions:key];});}
    }
    [self renderActiveTasks];
}
- (void)querySessions:(NSString *)key {
    if(self.sessions[key])return;NSString *name=nil;for(NSDictionary *row in self.rows)if([[self keyForRow:row] isEqual:key])name=row[@"name"];if(!name)return;
    __weak TramesManager *weakSelf=self;
    [self run:@[@"-e",name,@"session",@"ls",@"--json"] owner:[@"query:sessions:" stringByAppendingString:key] name:name title:@"" quiet:YES input:nil completion:^(NSString *out,int code){TramesManager *owner=weakSelf;id sessions=code?nil:[NSJSONSerialization JSONObjectWithData:[out dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];owner.sessions[key]=[sessions isKindOfClass:NSArray.class]?sessions:@[];if([owner.selectedKey isEqual:key]&&owner.tabs.selectedSegment==0)[owner render];}];
}
- (void)attachSession:(TMButton *)sender {NSString *name=[self selected];if(name)[self operation:@[@"env",@"terminal",name,@"--session",[sender.payload stringValue]] title:T(@"Opening environment…")];}
- (void)showTOML:(id)sender{(void)sender;self.tabs.selectedSegment=1;[self render];[self displayTOML:nil];}
- (void)renderConfiguration {
    NSDictionary *r=[self selectedRow];[self.content addArrangedSubview:TMLabel([r[@"source"] isEqual:@"host"]?r[@"profile"]:T(@"The guest profile is cached at last start. Saving creates an explicit host profile."),11,YES)];
    [self.content addArrangedSubview:TMStack(@[TMButtonMake(@"View TOML",self,@selector(displayTOML:),nil),TMButtonMake(@"Open in Editor",self,@selector(externalEditor:),nil),spacer()],NO)];
    NSString *section=nil;NSMutableArray *group=[NSMutableArray new];
    for(NSDictionary *field in self.snapshot[@"fields"]){NSString *path=field[@"path"],*kind=field[@"kind"];
        if([TMValue(r,@"config.environment.type") isEqual:@"android"] && [path hasPrefix:@"session."])continue;
        if(![section isEqual:field[@"section"]] && group.count){[self section:section view:TMGroup(group)];group=[NSMutableArray new];}section=field[@"section"];
        id value=[self value:path];NSView *control=nil;
        if([kind isEqual:@"bool"]){NSButton *check=[NSButton checkboxWithTitle:@"" target:self action:@selector(fieldChanged:)];check.identifier=path;check.state=[value boolValue]?NSControlStateValueOn:NSControlStateValueOff;[check setAccessibilityLabel:T(field[@"title"])];control=check;}
        else if([kind isEqual:@"resource"]){NSComboBox *combo=TMResourceCombo(path,value);combo.delegate=self;[combo setAccessibilityLabel:T(field[@"title"])];control=combo;}
        else if([@[@"choice",@"switch",@"network"] containsObject:kind]){
            NSPopUpButton *popup=[[NSPopUpButton alloc]initWithFrame:NSZeroRect pullsDown:NO];popup.identifier=path;popup.target=self;popup.action=@selector(fieldChanged:);popup.menu.autoenablesItems=NO;
            NSArray *options=field[@"options"];if([kind isEqual:@"network"]){NSMutableArray *names=[NSMutableArray arrayWithObject:@""];for(NSDictionary *n in self.snapshot[@"networks"])[names addObject:n[@"name"]];options=names;}
            for(NSString *option in options){[popup addItemWithTitle:T(option.length?option:@"Not selected")];popup.lastItem.representedObject=[kind isEqual:@"switch"]&&![option isEqual:@"auto"]?[NSNumber numberWithBool:[option isEqual:@"true"]]:option;}
            for(NSInteger i=0;i<popup.numberOfItems;i++)if([TMString([popup itemAtIndex:i].representedObject) isEqual:TMString(value)])[popup selectItemAtIndex:i];
            if([path isEqual:@"boot.mode"]){if([TMValue(r,@"config.environment.type") isEqual:@"android"]){[popup selectItemAtIndex:2];popup.enabled=NO;}else{[popup itemAtIndex:2].enabled=NO;if([TMValue(r,@"config.image.distribution") isEqual:@"alpine"])[popup itemAtIndex:1].enabled=NO;}}
            if([kind isEqual:@"network"])popup.enabled=[[self value:@"network.mode"] isEqual:@"bridge"];
            [popup.widthAnchor constraintEqualToConstant:190].active=YES;control=popup;
        }else if([kind isEqual:@"ports"]){TMButton *b=TMButtonMake(@"Edit Ports…",self,@selector(editPorts:),nil);b.enabled=[[self value:@"network.mode"] isEqual:@"bridge"];control=b;}
        else {NSTextField *text;if([kind isEqual:@"argv"]){NSTokenField *tokens=[NSTokenField new];tokens.tokenizingCharacterSet=[NSCharacterSet characterSetWithCharactersInString:@"\n"];tokens.objectValue=value;text=tokens;}else{text=[NSTextField new];text.stringValue=TMString(value);}text.identifier=path;text.delegate=self;text.placeholderString=[kind isEqual:@"optional"]?T(@"None"):nil;[text setAccessibilityLabel:T(field[@"title"])];[text.widthAnchor constraintEqualToConstant:190].active=YES;control=text;}
        [group addObject:TMRow(field[@"title"],field[@"help"],control)];
    }
    if(group.count)[self section:section view:TMGroup(group)];
}
- (void)fieldChanged:(NSControl *)sender {
    if(self.rendering)return;id value;if([sender isKindOfClass:NSPopUpButton.class])value=((NSPopUpButton *)sender).selectedItem.representedObject;else value=[NSNumber numberWithBool:((NSButton *)sender).state==NSControlStateValueOn];
    if([sender.identifier isEqual:@"network.bridge"] && [value isEqual:@""])value=NSNull.null;
    [self change:sender.identifier value:value];
    if([sender.identifier isEqual:@"network.mode"]){if(![value isEqual:@"bridge"]){[self change:@"network.bridge" value:NSNull.null];[self change:@"network.publish" value:@[]];}else if([self value:@"network.bridge"]==NSNull.null && [self.snapshot[@"networks"] count])[self change:@"network.bridge" value:self.snapshot[@"networks"][0][@"name"]];[self render];}
}
- (void)controlTextDidChange:(NSNotification *)note {
    NSTextField *control=note.object;if(!control.identifier.length || self.rendering)return;
    id value=[control isKindOfClass:NSComboBox.class]?TMResourceValue((NSComboBox *)control):[control isKindOfClass:NSTokenField.class]?control.objectValue:control.stringValue;
    if([control.identifier isEqual:@"resources.cpus"] && ![control.stringValue isEqual:@"all"]){NSScanner *scan=[NSScanner scannerWithString:control.stringValue];long long n;if([scan scanLongLong:&n] && scan.isAtEnd)value=@(n);}
    if([@[@"resources.memory_limit",@"network.bridge"] containsObject:control.identifier] && !control.stringValue.length)value=NSNull.null;
    [self change:control.identifier value:value];
}
- (void)comboBoxSelectionDidChange:(NSNotification *)note {
    NSComboBox *combo=note.object;if(combo.indexOfSelectedItem>=0)combo.stringValue=[combo itemObjectValueAtIndex:combo.indexOfSelectedItem];[self controlTextDidChange:note];
}
- (void)controlTextDidEndEditing:(NSNotification *)note{[self controlTextDidChange:note];}
- (void)displayTOML:(id)sender {
    (void)sender;NSDictionary *r=[self selectedRow];NSData *data=[NSJSONSerialization dataWithJSONObject:@{@"version":@1,@"revision":self.revisions[self.selectedKey]?:r[@"revision"],@"changes":[self draft]?:@{}} options:0 error:nil];
    __weak TramesManager *weakSelf=self;NSString *key=self.selectedKey;
    [self run:@[@"_manager",@"validate",key] owner:[@"query:toml:" stringByAppendingString:key] name:[self selected] title:T(@"View TOML") quiet:YES input:data completion:^(NSString *out,int code){TramesManager *owner=weakSelf;if(code){[owner showError:out];return;}NSDictionary *value=[NSJSONSerialization JSONObjectWithData:[out dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];NSAlert *a=[NSAlert new];a.messageText=T(@"Effective TOML preview");a.informativeText=T(@"Uses the same configuration model as the CLI. Host grants are stored separately.");NSTextView *text=[[NSTextView alloc]initWithFrame:NSMakeRect(0,0,580,390)];text.string=value[@"toml"]?:@"";text.editable=NO;text.font=[NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];NSScrollView *scroll=[[NSScrollView alloc]initWithFrame:text.frame];scroll.documentView=text;TMConfigureScroll(scroll);a.accessoryView=scroll;[a beginSheetModalForWindow:owner.window completionHandler:nil];}];
}
- (void)renderFiles {
    NSDictionary *r=[self selectedRow];NSMutableArray *rows=[NSMutableArray new];id mounts=[self value:@"mounts"];if([mounts isKindOfClass:NSArray.class])for(NSUInteger i=0;i<[mounts count];i++){NSDictionary *m=mounts[i];NSString *alias=m[@"share"]?:m[@"host"]?:@"";NSString *host=TMString(TMValue(r,[@"grants.shares." stringByAppendingFormat:@"%@.host",alias]));NSString *help=[NSString stringWithFormat:@"%@ → %@%@",host.length?host:alias,m[@"target"],[m[@"readonly"] boolValue]?T(@" · Read only"):@""];[rows addObject:TMRow(alias,help,TMButtonMake(@"Remove Mount",self,@selector(removeMount:),@(i)))];}
    if(!rows.count)[rows addObject:TMRow(@"No shared folders",@"Add a Mac folder and choose its guest path.",TMButtonMake(@"Add Shared Folder…",self,@selector(share:),nil))];else [self.content addArrangedSubview:TMButtonMake(@"Add Shared Folder…",self,@selector(share:),nil)];
    [self section:@"Mounted directories" view:TMGroup(rows)];[self.content addArrangedSubview:TMLabel(T(@"Portable configuration stores share aliases. Host paths and grants stay on this Mac."),11,YES)];
    NSMutableArray *authorized=[NSMutableArray new];NSDictionary *shares=TMValue(r,@"grants.shares");
    if([shares isKindOfClass:NSDictionary.class])for(NSString *alias in [shares.allKeys sortedArrayUsingSelector:@selector(compare:)]){
        BOOL used=NO;for(NSDictionary *mount in r[@"config"][@"mounts"])if([mount[@"share"] isEqual:alias])used=YES;
        TMButton *revoke=TMButtonMake(@"Revoke",self,@selector(revokeShare:),alias);revoke.enabled=!used;
        [authorized addObject:TMRow(alias,used?T(@"Remove and save its mounts before revoking this directory."):shares[alias][@"host"],revoke)];
    }
    if(authorized.count)[self section:@"Authorized Mac folders" view:TMGroup(authorized)];
    [self section:@"Private storage" view:TMGroup(@[TMRow(@"Environment files",r[@"path"],TMButtonMake(@"Show Storage in Finder",self,@selector(revealStorage:),nil)),TMRow(@"Backup and restore",@"Stop the environment before creating a backup.",TMButtonMake(@"Backup…",self,@selector(backup:),nil))])];
}
- (void)renderPermissions {
    NSDictionary *r=[self selectedRow];if([r[@"grants_error"] isKindOfClass:NSString.class])[self.content addArrangedSubview:TMLabel(r[@"grants_error"],12,YES)];NSMutableArray *rows=[NSMutableArray new];
    for(NSDictionary *cap in self.snapshot[@"capabilities"]){NSString *key=cap[@"key"],*path=[NSString stringWithFormat:@"interop.%@.enabled",key];
        NSPopUpButton *grant=[[NSPopUpButton alloc]initWithFrame:NSZeroRect pullsDown:NO];grant.identifier=key;grant.enabled=![r[@"grants_error"] isKindOfClass:NSString.class] && ![self hasWork:self.selectedKey];[grant addItemsWithTitles:@[T(@"Not allowed"),T(@"Allowed")]];
        BOOL allowed=[TMValue(r[@"config"],path) boolValue] && [TMValue(r,[@"grants." stringByAppendingString:key]) boolValue];[grant selectItemAtIndex:allowed?1:0];grant.target=self;grant.action=@selector(permission:);[grant setAccessibilityLabel:T(cap[@"title"])];[grant.widthAnchor constraintEqualToConstant:150].active=YES;
        [rows addObject:TMRow(cap[@"title"],cap[@"help"],grant)];
    }[self section:@"Host permissions" view:TMGroup(rows)];
}
- (void)renderActivity {
    TMJob *job=self.jobs[self.selectedKey]?:self.jobs[[@"create:" stringByAppendingString:[self selected]?:@""]];if(!job)for(TMJob *candidate in self.jobs.allValues)if(!candidate.quiet && [candidate.name isEqual:[self selected]])job=candidate;
    [self.content addArrangedSubview:TMLabel(job?job.title:T(@"No operations recorded in this manager session."),13,NO)];
    self.log=[[NSTextView alloc]initWithFrame:NSMakeRect(0,0,600,350)];self.log.editable=NO;self.log.font=[NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];self.log.string=job.output?:@"";
    NSScrollView *logs=[NSScrollView new];logs.documentView=self.log;TMConfigureScroll(logs);logs.borderType=NSBezelBorder;[logs.heightAnchor constraintEqualToConstant:350].active=YES;[self.content addArrangedSubview:logs];
    if(job.complete && job.code && [job.key hasPrefix:@"create:"])[self.content addArrangedSubview:TMButtonMake(@"Dismiss Failed Task",self,@selector(dismissFailedTask:),job.key)];
    [self.content addArrangedSubview:TMLabel(job?(job.complete?(job.code?T(@"Failed"):T(@"Completed")):T(@"Operation running. Other environments remain available.")):T(@"Task output is streamed here while the operation runs."),11,YES)];[self renderActiveTasks];
}
- (void)dismissFailedTask:(TMButton *)sender{[self.jobs removeObjectForKey:sender.payload];self.selectedKey=nil;self.pendingSelectionName=nil;[self refresh:nil];}
- (void)renderActiveTasks {
    NSMutableArray *rows=[NSMutableArray new];for(TMJob *job in self.jobs.allValues)if(!job.quiet && !job.complete)[rows addObject:TMRow(job.name,job.title,TMLabel(T(@"Running"),12,YES))];if(rows.count)[self section:@"Active tasks" view:TMGroup(rows)];
}
- (void)renderNetworks {
    [self heading:T(@"Networks") subtitle:T(@"Trames-owned private networks; no macOS bridge is created.") actions:@[TMButtonMake(@"New Network…",self,@selector(createNetwork:),nil)]];if([self.snapshot[@"networks_error"] isKindOfClass:NSString.class])[self.content addArrangedSubview:TMLabel(self.snapshot[@"networks_error"],12,YES)];NSMutableArray *rows=[NSMutableArray new];
    for(NSDictionary *n in self.snapshot[@"networks"]){NSString *help=[n[@"nat"] boolValue]?T(@"External access through NAT"):T(@"Internal network only");NSString *count=[NSString stringWithFormat:T(@"%lu assigned members"),(unsigned long)[n[@"members"] count]];[rows addObject:TMRow(n[@"name"],[help stringByAppendingFormat:@" · %@",count],TMButtonMake(@"Remove…",self,@selector(removeNetwork:),n[@"name"]))];}
    if(!rows.count)[rows addObject:TMRow(@"No private networks",@"Create a network, then select it in an environment's configuration.",TMLabel(@"",12,YES))];[self section:@"Trames networks" view:TMGroup(rows)];
    [self section:@"Host networking" view:TMGroup(@[TMRow(@"Direct Mac networking",@"Default mode. Normal host bind addresses and port conflicts apply.",TMLabel(T(@"No publisher needed"),12,YES))])];
}
- (void)renderSettings {
    [self heading:T(@"Trames Settings") subtitle:T(@"This Mac; independent from guest locale") actions:@[]];if([self.snapshot[@"global_error"] isKindOfClass:NSString.class])[self.content addArrangedSubview:TMLabel(self.snapshot[@"global_error"],12,YES)];
    NSPopUpButton *defaults=[[NSPopUpButton alloc]initWithFrame:NSZeroRect pullsDown:NO];for(NSDictionary *r in self.rows)[defaults addItemWithTitle:r[@"name"]];[defaults selectItemWithTitle:TMString(TMValue(self.snapshot,@"global.default_environment"))];defaults.target=self;defaults.action=@selector(globalSetting:);defaults.identifier=@"default_environment";
    [self section:@"General" view:TMGroup(@[TMRow(@"Default environment",@"Used when the CLI has no -e selection",defaults),TMRow(@"Interface language",@"Follows the macOS per-app language; guest locale is unchanged.",TMLabel(NSBundle.mainBundle.preferredLocalizations.firstObject?:@"",12,YES))])];
    NSMutableArray *rows=[NSMutableArray new];for(NSDictionary *r in self.rows){NSButton *p=[NSButton checkboxWithTitle:@"" target:self action:@selector(autostart:)];p.state=[r[@"autostart"] boolValue]?NSControlStateValueOn:NSControlStateValueOff;p.identifier=[self keyForRow:r];[p setAccessibilityLabel:[NSString stringWithFormat:@"%@ — %@",r[@"name"],T(@"Start at login")]];[rows addObject:TMRow(r[@"name"],@"Start this environment at Mac user login",p)];}if(rows.count)[self section:@"Start at login" view:TMGroup(rows)];
    [self section:@"Configuration and diagnostics" view:TMGroup(@[TMRow(@"Global defaults",self.snapshot[@"global_path"],TMButtonMake(@"Open in Editor",self,@selector(editGlobal:),nil)),TMRow(@"Version and environment information",@"Collect configuration and build identities",TMButtonMake(@"Diagnostics",self,@selector(doctor:),nil))])];
}
- (void)revealStorage:(id)sender{(void)sender;NSString *path=[self selectedRow][@"path"];if(path)[NSWorkspace.sharedWorkspace activateFileViewerSelectingURLs:@[[NSURL fileURLWithPath:[path stringByAppendingPathComponent:@"store"]]]];}
- (void)removeNetwork:(TMButton *)sender{if([self confirm:@"Remove Trames network?" message:@"Attached environments must be stopped. Private environment files are preserved."])[self run:@[@"network",@"remove",sender.payload] owner:[@"network:" stringByAppendingString:sender.payload] name:sender.payload title:T(@"Removing network…") quiet:NO input:nil completion:nil];}
- (void)autostart:(NSButton *)sender{NSDictionary *row=nil;for(NSDictionary *r in self.rows)if([[self keyForRow:r] isEqual:sender.identifier])row=r;if(row)[self run:@[@"env",sender.state==NSControlStateValueOn?@"enable":@"disable",row[@"name"]] owner:[self keyForRow:row] name:row[@"name"] title:T(@"Saving login setting…") quiet:NO input:nil completion:nil];}
- (void)editGlobal:(id)sender{(void)sender;NSString *path=self.snapshot[@"global_path"];if([NSFileManager.defaultManager fileExistsAtPath:path])[NSWorkspace.sharedWorkspace openURL:[NSURL fileURLWithPath:path]];else self.status.stringValue=T(@"Set a default environment first to create config.toml.");}
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)app{(void)app;for(TMJob *job in self.jobs.allValues)if(!job.quiet&&!job.complete){self.status.stringValue=T(@"A management task is still running. Wait for it to finish before closing.");return NSTerminateCancel;}for(NSDictionary *d in self.drafts.allValues)if(d.count){if(![self confirm:@"Discard unsaved drafts?" message:@"Environment configuration files have not been changed."])return NSTerminateCancel;break;}for(dispatch_source_t source in self.watches)dispatch_source_cancel(source);return NSTerminateNow;}
@end
void trames_manager_run(void){TramesManagerLoadActions();@autoreleasepool{[NSApplication sharedApplication];TramesManager *manager=[TramesManager new];NSApp.delegate=manager;[NSApp run];(void)manager;}}
