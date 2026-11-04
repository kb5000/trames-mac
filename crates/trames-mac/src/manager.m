// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#import "localization.h"
#import "task-stream.h"

@interface TramesManager : NSObject <NSApplicationDelegate,NSTableViewDataSource,NSTableViewDelegate>
@property NSWindow *window;
@property NSTableView *table;
@property NSTextField *status;
@property NSTextView *log;
@property NSArray<NSDictionary *> *rows;
@property NSMutableArray<NSButton *> *actions;
@property NSTask *task;
@property(strong) TramesTaskStream *taskStream;
@property BOOL refreshing;
@end
static NSString *productName(void){return [NSBundle.mainBundle objectForInfoDictionaryKey:@"CFBundleDisplayName"] ?: @"Trames";}
static NSString *executable(void){
    NSString *entry=NSBundle.mainBundle.executablePath;
    if([entry.lastPathComponent isEqual:@"trames-manager"])
        return [entry.stringByDeletingLastPathComponent stringByAppendingPathComponent:@"trames-mac-run"];
    return entry ?: NSProcessInfo.processInfo.arguments.firstObject;
}
static NSTextField *label(NSString *s){NSTextField *v=[NSTextField labelWithString:s];v.lineBreakMode=NSLineBreakByWordWrapping;return v;}
static NSButton *button(NSString *s,id target,SEL action){NSButton *b=[NSButton buttonWithTitle:s target:target action:action];b.bezelStyle=NSBezelStyleRounded;return b;}
@implementation TramesManager
- (void)applicationDidBecomeActive:(NSNotification *)notification{(void)notification;if(self.window && !self.task && !NSApp.modalWindow)[self refresh:nil];}
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)app{(void)app;return YES;}
- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;self.rows=@[];self.actions=[NSMutableArray array];
    NSImage *icon=[[NSImage alloc]initWithContentsOfURL:[NSBundle.mainBundle URLForResource:@"Trames" withExtension:@"icns"]];
    if(icon)NSApp.applicationIconImage=icon;
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSMenu *menu=[NSMenu new];NSMenuItem *app=[NSMenuItem new];[menu addItem:app];NSMenu *sub=[NSMenu new];[sub addItemWithTitle:[NSString stringWithFormat:T(@"Quit %@"),productName()] action:@selector(terminate:) keyEquivalent:@"q"];app.submenu=sub;NSApp.mainMenu=menu;
    self.window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,900,650) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
    self.window.title=[NSString stringWithFormat:T(@"%@ — Linux Environments"),productName()];self.window.minSize=NSMakeSize(800,600);[self.window center];
    NSStackView *root=[NSStackView stackViewWithViews:@[]];root.orientation=NSUserInterfaceLayoutOrientationVertical;root.alignment=NSLayoutAttributeLeading;root.spacing=14;root.edgeInsets=NSEdgeInsetsMake(22,22,22,22);root.translatesAutoresizingMaskIntoConstraints=NO;[self.window.contentView addSubview:root];
    [NSLayoutConstraint activateConstraints:@[[root.leadingAnchor constraintEqualToAnchor:self.window.contentView.leadingAnchor],[root.trailingAnchor constraintEqualToAnchor:self.window.contentView.trailingAnchor],[root.topAnchor constraintEqualToAnchor:self.window.contentView.topAnchor],[root.bottomAnchor constraintEqualToAnchor:self.window.contentView.bottomAnchor]]];
    NSTextField *title=label(T(@"Your Linux environments"));title.font=[NSFont systemFontOfSize:24 weight:NSFontWeightSemibold];NSStackView *heading=[NSStackView stackViewWithViews:@[]];heading.alignment=NSLayoutAttributeCenterY;heading.spacing=10;if(icon){NSImageView *brand=[NSImageView imageViewWithImage:icon];brand.imageScaling=NSImageScaleProportionallyUpOrDown;[NSLayoutConstraint activateConstraints:@[[brand.widthAnchor constraintEqualToConstant:32],[brand.heightAnchor constraintEqualToConstant:32]]];[heading addArrangedSubview:brand];}[heading addArrangedSubview:title];[root addArrangedSubview:heading];
    [root addArrangedSubview:label(T(@"Start a terminal, share a folder, or manage a backup. Linux apps open their own windows when needed."))];
    NSStackView *toolbar=[NSStackView stackViewWithViews:@[]];toolbar.spacing=8;
    NSArray *titles=@[T(@"New Environment…"),T(@"Open Terminal"),T(@"Stop"),T(@"Import Store…"),T(@"Refresh")];
    SEL selectors[]={@selector(create:),@selector(start:),@selector(stop:),@selector(importStore:),@selector(refresh:)};
    for(NSUInteger i=0;i<titles.count;i++){NSButton *b=button(titles[i],self,selectors[i]);[toolbar addArrangedSubview:b];[self.actions addObject:b];}[root addArrangedSubview:toolbar];
    self.table=[NSTableView new];self.table.delegate=self;self.table.dataSource=self;self.table.allowsEmptySelection=YES;self.table.rowHeight=34;
    for(NSString *key in @[@"name",@"distribution",@"state"]){NSTableColumn *col=[[NSTableColumn alloc]initWithIdentifier:key];col.editable=NO;col.title=[@{@"name":T(@"Environment"),@"distribution":T(@"Linux"),@"state":T(@"Status")} objectForKey:key];col.width=[key isEqual:@"name"]?330:200;[self.table addTableColumn:col];}
    NSScrollView *scroll=[NSScrollView new];scroll.documentView=self.table;scroll.hasVerticalScroller=YES;scroll.borderType=NSBezelBorder;[root addArrangedSubview:scroll];[scroll.widthAnchor constraintEqualToAnchor:root.widthAnchor constant:-44].active=YES;[scroll.heightAnchor constraintGreaterThanOrEqualToConstant:180].active=YES;
    NSStackView *tools=[NSStackView stackViewWithViews:@[]];tools.spacing=8;
    NSArray *more=@[T(@"Settings…"),T(@"Share Folder…"),T(@"Install Tools…"),T(@"Backup…"),T(@"Restore…"),T(@"Delete…"),T(@"Diagnostics")];
    SEL moreSel[]={@selector(settings:),@selector(share:),@selector(tools:),@selector(backup:),@selector(restore:),@selector(remove:),@selector(doctor:)};
    for(NSUInteger i=0;i<more.count;i++){NSButton *b=button(more[i],self,moreSel[i]);[tools addArrangedSubview:b];[self.actions addObject:b];}[root addArrangedSubview:tools];
    self.status=label(T(@"Loading environments…"));[root addArrangedSubview:self.status];
    self.log=[NSTextView new];self.log.editable=NO;self.log.font=[NSFont monospacedSystemFontOfSize:11 weight:NSFontWeightRegular];
    NSScrollView *logs=[NSScrollView new];logs.documentView=self.log;logs.hasVerticalScroller=YES;logs.borderType=NSBezelBorder;[root addArrangedSubview:logs];[logs.widthAnchor constraintEqualToAnchor:scroll.widthAnchor].active=YES;[logs.heightAnchor constraintEqualToConstant:140].active=YES;
    [self.window makeKeyAndOrderFront:nil];[NSApp activateIgnoringOtherApps:YES];[self refresh:nil];
}
- (NSInteger)numberOfRowsInTableView:(NSTableView *)table{(void)table;return self.rows.count;}
- (id)tableView:(NSTableView *)table objectValueForTableColumn:(NSTableColumn *)column row:(NSInteger)row{(void)table;id value=self.rows[row][column.identifier];return [column.identifier isEqual:@"state"] && [value isKindOfClass:NSString.class] ? T(value) : value;}
- (NSString *)selected {NSInteger row=self.table.selectedRow;if(row<0||row>=(NSInteger)self.rows.count){self.status.stringValue=T(@"Select an environment first.");return nil;}return self.rows[row][@"name"];}
- (void)busy:(BOOL)busy {for(NSButton *b in self.actions)b.enabled=!busy;}
- (void)run:(NSArray<NSString *> *)args title:(NSString *)title completion:(void(^)(NSString *,int))done {
    if(self.task)return;[self busy:YES];self.status.stringValue=title;
    BOOL visible=![args isEqual:@[@"env",@"ls",@"--json"]];if(visible)self.log.string=@"";
    NSTask *task=[NSTask new];task.executableURL=[NSURL fileURLWithPath:executable()];task.arguments=args;task.standardInput=[NSFileHandle fileHandleWithNullDevice];
    self.task=task;__weak TramesManager *weakSelf=self;
    self.taskStream=[[TramesTaskStream alloc]initWithTask:task onText:^(NSString *text){
        TramesManager *owner=weakSelf;
        if(visible && owner){[owner.log.textStorage appendAttributedString:[[NSAttributedString alloc]initWithString:text attributes:@{NSFontAttributeName:owner.log.font}]];[owner.log scrollRangeToVisible:NSMakeRange(owner.log.string.length,0)];}
    } completion:^(NSString *output,int status){
        TramesManager *owner=weakSelf;owner.task=nil;owner.taskStream=nil;[owner busy:NO];
        if(!visible && status!=0)owner.log.string=output;
        owner.status.stringValue=status==0?T(@"Done."):T(@"Operation failed. Your existing environments were preserved; see details below.");
        if(done)done(output,status);
    }];
    NSError *error=nil;if(![self.taskStream launchAndReturnError:&error]){self.task=nil;self.taskStream=nil;[self busy:NO];self.status.stringValue=error.localizedDescription;}
}
- (void)refresh:(id)sender {
    (void)sender;NSString *selected=[self.table.selectedRow>=0?self.rows[self.table.selectedRow]:@{} objectForKey:@"name"];
    __weak TramesManager *weakSelf=self;
    [self run:@[@"env",@"ls",@"--json"] title:T(@"Refreshing…") completion:^(NSString *out,int code){
        TramesManager *owner=weakSelf;if(code || !owner)return;
        NSData *data=[out dataUsingEncoding:NSUTF8StringEncoding];id rows=[NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
        if(![rows isKindOfClass:NSArray.class]){owner.status.stringValue=T(@"Could not read environment list.");return;}
        owner.rows=rows;[owner.table reloadData];for(NSUInteger i=0;i<owner.rows.count;i++)if([owner.rows[i][@"name"] isEqual:selected])[owner.table selectRowIndexes:[NSIndexSet indexSetWithIndex:i] byExtendingSelection:NO];
        owner.status.stringValue=owner.rows.count?T(@"Ready. Settings apply on the next start."):T(@"Create your first environment. Alpine is the default; Debian is also available.");
    }];
}
- (void)operation:(NSArray *)args title:(NSString *)title {
    __weak TramesManager *weakSelf=self;
    [self run:args title:title completion:^(NSString *out,int code){(void)out;if(!code)[weakSelf refresh:nil];}];
}
- (NSAlert *)dialog:(NSString *)title message:(NSString *)message {NSAlert *a=[NSAlert new];a.messageText=title;a.informativeText=message;[a addButtonWithTitle:T(@"Continue")];[a addButtonWithTitle:T(@"Cancel")];return a;}
- (NSString *)askName:(NSString *)title {NSAlert *a=[self dialog:title message:T(@"Use letters, numbers, '-' or '_'. Existing environments are never replaced.")];NSTextField *input=[[NSTextField alloc]initWithFrame:NSMakeRect(0,0,330,26)];input.placeholderString=@"my-linux";a.accessoryView=input;[a.window setInitialFirstResponder:input];return [a runModal]==NSAlertFirstButtonReturn?input.stringValue:nil;}
- (void)distributionChanged:(NSPopUpButton *)sender {
    for(NSView *view in sender.superview.subviews)if([view.identifier isEqual:@"debian-packages"])((NSPopUpButton *)view).enabled=sender.indexOfSelectedItem==1||sender.indexOfSelectedItem==2;
}
- (void)create:(id)sender {
    (void)sender;
    NSAlert *a=[self dialog:T(@"New Linux Environment") message:T(@"Debian includes daily tools. Add C/C++ and Python development tools if needed. systemd is optional for managed services.")];
    NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,370,145)];
    NSTextField *input=[[NSTextField alloc]initWithFrame:NSMakeRect(0,110,370,26)];input.placeholderString=T(@"Environment name");
    NSPopUpButton *d=[[NSPopUpButton alloc]initWithFrame:NSMakeRect(0,65,370,30)];[d addItemsWithTitles:@[T(@"Alpine — shell (default)"),T(@"Debian 13 — shell"),T(@"Debian 13 — systemd"),T(@"Custom ARM64 rootfs…")]];d.target=self;d.action=@selector(distributionChanged:);
    NSTextField *l=label(T(@"Debian tools"));l.frame=NSMakeRect(0,15,105,24);
    NSPopUpButton *packages=[[NSPopUpButton alloc]initWithFrame:NSMakeRect(110,10,260,30)];packages.identifier=@"debian-packages";[packages addItemsWithTitles:@[T(@"Minimal base only"),T(@"Daily tools (default)"),T(@"Daily + development tools")]];[packages selectItemAtIndex:1];packages.enabled=NO;
    [v addSubview:input];[v addSubview:d];[v addSubview:l];[v addSubview:packages];a.accessoryView=v;[a.window setInitialFirstResponder:input];
    if([a runModal]!=NSAlertFirstButtonReturn)return;
    if(d.indexOfSelectedItem==3){NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseFiles=YES;p.canChooseDirectories=YES;p.message=T(@"Choose an unpacked ARM64 Linux rootfs directory or a tar/tar.gz archive.");if([p runModal]==NSModalResponseOK)[self operation:@[@"env",@"create",input.stringValue,@"--rootfs",p.URL.path,@"--init",@"shell"] title:T(@"Importing custom rootfs…")];return;}
    NSMutableArray *args=[NSMutableArray arrayWithArray:@[@"env",@"create",input.stringValue,@"--distro",d.indexOfSelectedItem==0?@"alpine":@"debian",@"--init",d.indexOfSelectedItem==2?@"systemd":@"shell"]];
    if(d.indexOfSelectedItem!=0)[args addObjectsFromArray:@[@"--packages",@[@"minimal",@"standard",@"development"][packages.indexOfSelectedItem]]];
    [self operation:args title:T(@"Creating environment and preparing tools…")];
}
- (void)tools:(id)sender {
    (void)sender;NSString *n=[self selected];if(!n)return;
    NSDictionary *c=self.rows[self.table.selectedRow][@"config"];
    if(![c isKindOfClass:NSDictionary.class] || ![c[@"distribution"] isEqual:@"debian"]){self.status.stringValue=T(@"Tool presets are available for built-in Debian environments.");return;}
    NSAlert *a=[self dialog:T(@"Install Debian Tools") message:T(@"Stop the environment first. Install daily tools or add C/C++ and Python development tools. Existing files and settings are preserved.")];
    NSPopUpButton *p=[[NSPopUpButton alloc]initWithFrame:NSMakeRect(0,0,350,30)];[p addItemsWithTitles:@[T(@"Daily tools"),T(@"Daily + development tools")]];a.accessoryView=p;
    if([a runModal]==NSAlertFirstButtonReturn)[self operation:@[@"env",@"packages",n,p.indexOfSelectedItem==0?@"standard":@"development"] title:T(@"Installing Debian tools…")];
}
- (void)start:(id)sender{(void)sender;NSString *n=[self selected];if(n)[self operation:@[@"env",@"terminal",n] title:T(@"Opening Terminal…")];}
- (void)stop:(id)sender{(void)sender;NSString *n=[self selected];if(n)[self operation:@[@"env",@"stop",n] title:T(@"Stopping and saving…")];}
- (void)importStore:(id)sender{(void)sender;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseFiles=NO;p.canChooseDirectories=YES;p.message=T(@"Select an existing, stopped Trames private store (containing manifest and objects). A separate copy will be created.");if([p runModal]!=NSModalResponseOK)return;NSString *n=[self askName:T(@"Import Existing Store")];if(n)[self operation:@[@"env",@"import-store",n,p.URL.path] title:T(@"Importing existing environment…")];}
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender{(void)sender;if(self.task){self.status.stringValue=T(@"An operation is still running. Wait for it to finish before closing the manager.");return NSTerminateCancel;}return NSTerminateNow;}
- (void)settings:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSDictionary *c=self.rows[self.table.selectedRow][@"config"];if(![c isKindOfClass:NSDictionary.class]){self.status.stringValue=self.rows[self.table.selectedRow][@"error"]?:T(@"Configuration needs attention.");return;}NSAlert *a=[self dialog:T(@"Environment Settings") message:T(@"Stop the environment before changing settings. Memory 'auto' grows with actual use. Prepare systemd with trames env setup NAME --init systemd before selecting it.")];
    NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,360,280)];NSPopUpButton *boot=[[NSPopUpButton alloc]initWithFrame:NSMakeRect(0,240,350,30)];[boot addItemsWithTitles:@[T(@"Shell (default)"),T(@"systemd (Debian / prepared image)")]];[boot selectItemAtIndex:[c[@"init"] isEqual:@"systemd"]?1:0];boot.menu.autoenablesItems=NO;[boot itemAtIndex:1].enabled=![c[@"distribution"] isEqual:@"alpine"];[v addSubview:boot];NSMutableArray *fields=[NSMutableArray array];NSArray *names=@[T(@"CPU slots"),T(@"Memory (MiB / auto)"),T(@"Network")];NSArray *values=@[[c[@"cpus"] description],c[@"memory_mib"],c[@"network"]];for(NSUInteger i=0;i<3;i++){NSTextField *l=label(names[i]);l.frame=NSMakeRect(0,200-i*38,160,24);[v addSubview:l];NSTextField *f=[[NSTextField alloc]initWithFrame:NSMakeRect(170,200-i*38,180,26)];f.stringValue=values[i];[v addSubview:f];[fields addObject:f];}
    NSMutableArray *checks=[NSMutableArray array];NSArray *keys=@[@"windows",@"vulkan",@"video"];NSArray *titles=@[T(@"Allow app windows"),T(@"Enable Vulkan"),T(@"Enable VideoToolbox / VA-API")];
    for(NSUInteger i=0;i<3;i++){NSButton *b=[NSButton checkboxWithTitle:titles[i] target:nil action:nil];b.frame=NSMakeRect(0,100-i*26,350,24);b.state=[c[keys[i]] boolValue]?NSControlStateValueOn:NSControlStateValueOff;[checks addObject:b];[v addSubview:b];}NSButton *clear=[NSButton checkboxWithTitle:T(@"Remove all shared folders") target:nil action:nil];clear.frame=NSMakeRect(0,0,350,26);[v addSubview:clear];a.accessoryView=v;if([a runModal]!=NSAlertFirstButtonReturn)return;
    NSMutableArray *arguments=[NSMutableArray arrayWithArray:@[@"env",@"configure",n,@"--init",boot.indexOfSelectedItem==0?@"shell":@"systemd",@"--cpus",[fields[0] stringValue],@"--memory-mib",[fields[1] stringValue],@"--network",[fields[2] stringValue],@"--windows",[checks[0] state]?@"on":@"off",@"--vulkan",[checks[1] state]?@"on":@"off",@"--video",[checks[2] state]?@"on":@"off"]];if(clear.state)[arguments addObject:@"--clear-mounts"];[self operation:arguments title:T(@"Saving settings…")];}
- (void)share:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseFiles=NO;p.canChooseDirectories=YES;p.message=T(@"Select a host folder to share with Linux.");if([p runModal]!=NSModalResponseOK)return;NSAlert *a=[self dialog:T(@"Shared Folder") message:T(@"Linux path (for example /work). Stop the environment before changing shared folders.")];NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,340,70)];NSTextField *path=[[NSTextField alloc]initWithFrame:NSMakeRect(0,40,340,26)];path.stringValue=@"/work";NSButton *ro=[NSButton checkboxWithTitle:T(@"Read only") target:nil action:nil];ro.frame=NSMakeRect(0,0,340,26);[v addSubview:path];[v addSubview:ro];a.accessoryView=v;if([a runModal]!=NSAlertFirstButtonReturn)return;[self operation:@[@"env",@"configure",n,ro.state?@"--mount-ro":@"--mount",[NSString stringWithFormat:@"%@=%@",path.stringValue,p.URL.path]] title:T(@"Saving shared folder…")];}
- (void)backup:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSSavePanel *p=[NSSavePanel savePanel];p.nameFieldStringValue=[n stringByAppendingString:@".trames.tar.gz"];p.message=T(@"Stop the environment first. Existing backups will not be overwritten.");if([p runModal]==NSModalResponseOK)[self operation:@[@"env",@"backup",n,p.URL.path] title:T(@"Creating verified backup…")];}
- (void)restore:(id)sender{(void)sender;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseDirectories=NO;p.message=T(@"Choose a Trames backup.");if([p runModal]!=NSModalResponseOK)return;NSString *n=[self askName:T(@"Restore as New Environment")];if(n)[self operation:@[@"env",@"restore",n,p.URL.path] title:T(@"Verifying and restoring backup…")];}
- (void)remove:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSAlert *a=[self dialog:T(@"Delete Environment?") message:[NSString stringWithFormat:T(@"All private Linux files in '%@' will be deleted. Shared host folders are preserved. Create a backup first if needed."),n]];a.alertStyle=NSAlertStyleWarning;if([a runModal]==NSAlertFirstButtonReturn)[self operation:@[@"env",@"delete",n,@"--yes"] title:T(@"Deleting environment…")];}
- (void)doctor:(id)sender{(void)sender;[self run:@[@"env",@"doctor"] title:T(@"Collecting version and environment information…") completion:nil];}
@end
void trames_manager_run(void){@autoreleasepool{[NSApplication sharedApplication];TramesManager *manager=[TramesManager new];NSApp.delegate=manager;[NSApp run];(void)manager;}}
