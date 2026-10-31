// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>

@interface TramesManager : NSObject <NSApplicationDelegate,NSTableViewDataSource,NSTableViewDelegate>
@property NSWindow *window;
@property NSTableView *table;
@property NSTextField *status;
@property NSTextView *log;
@property NSArray<NSDictionary *> *rows;
@property NSMutableArray<NSButton *> *actions;
@property NSTask *task;
@property BOOL refreshing;
@end
static NSString *executable(void){return NSBundle.mainBundle.executablePath ?: NSProcessInfo.processInfo.arguments.firstObject;}
static NSTextField *label(NSString *s){NSTextField *v=[NSTextField labelWithString:s];v.lineBreakMode=NSLineBreakByWordWrapping;return v;}
static NSButton *button(NSString *s,id target,SEL action){NSButton *b=[NSButton buttonWithTitle:s target:target action:action];b.bezelStyle=NSBezelStyleRounded;return b;}
@implementation TramesManager
- (void)applicationDidBecomeActive:(NSNotification *)notification{(void)notification;if(self.window && !self.task && !NSApp.modalWindow)[self refresh:nil];}
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)app{(void)app;return YES;}
- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;self.rows=@[];self.actions=[NSMutableArray array];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSMenu *menu=[NSMenu new];NSMenuItem *app=[NSMenuItem new];[menu addItem:app];NSMenu *sub=[NSMenu new];[sub addItemWithTitle:@"Quit Trames" action:@selector(terminate:) keyEquivalent:@"q"];app.submenu=sub;NSApp.mainMenu=menu;
    self.window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,900,650) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
    self.window.title=@"Trames — Linux Environments";self.window.minSize=NSMakeSize(800,600);[self.window center];
    NSStackView *root=[NSStackView stackViewWithViews:@[]];root.orientation=NSUserInterfaceLayoutOrientationVertical;root.alignment=NSLayoutAttributeLeading;root.spacing=14;root.edgeInsets=NSEdgeInsetsMake(22,22,22,22);root.translatesAutoresizingMaskIntoConstraints=NO;[self.window.contentView addSubview:root];
    [NSLayoutConstraint activateConstraints:@[[root.leadingAnchor constraintEqualToAnchor:self.window.contentView.leadingAnchor],[root.trailingAnchor constraintEqualToAnchor:self.window.contentView.trailingAnchor],[root.topAnchor constraintEqualToAnchor:self.window.contentView.topAnchor],[root.bottomAnchor constraintEqualToAnchor:self.window.contentView.bottomAnchor]]];
    NSTextField *title=label(@"Your Linux environments");title.font=[NSFont systemFontOfSize:24 weight:NSFontWeightSemibold];[root addArrangedSubview:title];
    [root addArrangedSubview:label(@"Start a terminal, share a folder, or manage a backup. Linux apps open their own windows when needed.")];
    NSStackView *toolbar=[NSStackView stackViewWithViews:@[]];toolbar.spacing=8;
    NSArray *titles=@[@"New Environment…",@"Open Terminal",@"Stop",@"Import Store…",@"Refresh"];
    SEL selectors[]={@selector(create:),@selector(start:),@selector(stop:),@selector(importStore:),@selector(refresh:)};
    for(NSUInteger i=0;i<titles.count;i++){NSButton *b=button(titles[i],self,selectors[i]);[toolbar addArrangedSubview:b];[self.actions addObject:b];}[root addArrangedSubview:toolbar];
    self.table=[NSTableView new];self.table.delegate=self;self.table.dataSource=self;self.table.allowsEmptySelection=YES;self.table.rowHeight=34;
    for(NSString *key in @[@"name",@"distribution",@"state"]){NSTableColumn *col=[[NSTableColumn alloc]initWithIdentifier:key];col.editable=NO;col.title=[@{@"name":@"Environment",@"distribution":@"Linux",@"state":@"Status"} objectForKey:key];col.width=[key isEqual:@"name"]?330:200;[self.table addTableColumn:col];}
    NSScrollView *scroll=[NSScrollView new];scroll.documentView=self.table;scroll.hasVerticalScroller=YES;scroll.borderType=NSBezelBorder;[root addArrangedSubview:scroll];[scroll.widthAnchor constraintEqualToAnchor:root.widthAnchor constant:-44].active=YES;[scroll.heightAnchor constraintGreaterThanOrEqualToConstant:180].active=YES;
    NSStackView *tools=[NSStackView stackViewWithViews:@[]];tools.spacing=8;
    NSArray *more=@[@"Settings…",@"Share Folder…",@"Backup…",@"Restore…",@"Delete…",@"Diagnostics"];
    SEL moreSel[]={@selector(settings:),@selector(share:),@selector(backup:),@selector(restore:),@selector(remove:),@selector(doctor:)};
    for(NSUInteger i=0;i<more.count;i++){NSButton *b=button(more[i],self,moreSel[i]);[tools addArrangedSubview:b];[self.actions addObject:b];}[root addArrangedSubview:tools];
    self.status=label(@"Loading environments…");[root addArrangedSubview:self.status];
    self.log=[NSTextView new];self.log.editable=NO;self.log.font=[NSFont monospacedSystemFontOfSize:11 weight:NSFontWeightRegular];
    NSScrollView *logs=[NSScrollView new];logs.documentView=self.log;logs.hasVerticalScroller=YES;logs.borderType=NSBezelBorder;[root addArrangedSubview:logs];[logs.widthAnchor constraintEqualToAnchor:scroll.widthAnchor].active=YES;[logs.heightAnchor constraintEqualToConstant:140].active=YES;
    [self.window makeKeyAndOrderFront:nil];[NSApp activateIgnoringOtherApps:YES];[self refresh:nil];
}
- (NSInteger)numberOfRowsInTableView:(NSTableView *)table{(void)table;return self.rows.count;}
- (id)tableView:(NSTableView *)table objectValueForTableColumn:(NSTableColumn *)column row:(NSInteger)row{(void)table;return self.rows[row][column.identifier];}
- (NSString *)selected {NSInteger row=self.table.selectedRow;if(row<0||row>=(NSInteger)self.rows.count){self.status.stringValue=@"Select an environment first.";return nil;}return self.rows[row][@"name"];}
- (void)busy:(BOOL)busy {for(NSButton *b in self.actions)b.enabled=!busy;}
- (void)run:(NSArray<NSString *> *)args title:(NSString *)title completion:(void(^)(NSString *,int))done {
    if(self.task)return;[self busy:YES];self.status.stringValue=title;
    NSString *path=[NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"trames-manager-%@.log",NSUUID.UUID.UUIDString]];
    [[NSFileManager defaultManager]createFileAtPath:path contents:nil attributes:@{NSFilePosixPermissions:@0600}];NSFileHandle *file=[NSFileHandle fileHandleForWritingAtPath:path];
    NSTask *task=[NSTask new];task.executableURL=[NSURL fileURLWithPath:executable()];task.arguments=args;task.standardOutput=file;task.standardError=file;task.standardInput=[NSFileHandle fileHandleWithNullDevice];
    self.task=task;
    task.terminationHandler=^(NSTask *finished){[file closeFile];NSString *output=[[NSString alloc]initWithContentsOfFile:path encoding:NSUTF8StringEncoding error:nil]?:@"";[[NSFileManager defaultManager]removeItemAtPath:path error:nil];int status=finished.terminationStatus;dispatch_async(dispatch_get_main_queue(),^{self.task=nil;[self busy:NO];if(![args isEqual:@[@"env",@"list"]] || status!=0)self.log.string=output;self.status.stringValue=status==0?@"Done.":@"Operation failed. Your existing environments were preserved; see details below.";if(done)done(output,status);});};
    NSError *error=nil;if(![task launchAndReturnError:&error]){self.task=nil;[file closeFile];[[NSFileManager defaultManager]removeItemAtPath:path error:nil];[self busy:NO];self.status.stringValue=error.localizedDescription;}
}
- (void)refresh:(id)sender{(void)sender;NSString *selected=[self.table.selectedRow>=0?self.rows[self.table.selectedRow]:@{} objectForKey:@"name"];[self run:@[@"env",@"list"] title:@"Refreshing…" completion:^(NSString *out,int code){if(code)return;NSData *data=[out dataUsingEncoding:NSUTF8StringEncoding];id rows=[NSJSONSerialization JSONObjectWithData:data options:0 error:nil];if(![rows isKindOfClass:NSArray.class]){self.status.stringValue=@"Could not read environment list.";return;}self.rows=rows;[self.table reloadData];for(NSUInteger i=0;i<self.rows.count;i++)if([self.rows[i][@"name"] isEqual:selected])[self.table selectRowIndexes:[NSIndexSet indexSetWithIndex:i] byExtendingSelection:NO];self.status.stringValue=self.rows.count?@"Ready. Settings apply on the next start.":@"Create your first environment. Alpine is the default; Debian is also available.";}];}
- (void)operation:(NSArray *)args title:(NSString *)title{[self run:args title:title completion:^(NSString *out,int code){(void)out;if(!code)[self refresh:nil];}];}
- (NSAlert *)dialog:(NSString *)title message:(NSString *)message {NSAlert *a=[NSAlert new];a.messageText=title;a.informativeText=message;[a addButtonWithTitle:@"Continue"];[a addButtonWithTitle:@"Cancel"];return a;}
- (NSString *)askName:(NSString *)title {NSAlert *a=[self dialog:title message:@"Use letters, numbers, '-' or '_'. Existing environments are never replaced."];NSTextField *input=[[NSTextField alloc]initWithFrame:NSMakeRect(0,0,330,26)];input.placeholderString=@"my-linux";a.accessoryView=input;[a.window setInitialFirstResponder:input];return [a runModal]==NSAlertFirstButtonReturn?input.stringValue:nil;}
- (void)create:(id)sender{(void)sender;NSAlert *a=[self dialog:@"New Linux Environment" message:@"Downloads and verifies an official ARM64 Linux system. Your files are stored separately from the app."];NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,350,90)];NSTextField *input=[[NSTextField alloc]initWithFrame:NSMakeRect(0,55,350,26)];input.placeholderString=@"Environment name";NSPopUpButton *d=[[NSPopUpButton alloc]initWithFrame:NSMakeRect(0,10,350,30)];[d addItemsWithTitles:@[@"Alpine (default)",@"Debian 13 slim"]];[v addSubview:input];[v addSubview:d];a.accessoryView=v;[a.window setInitialFirstResponder:input];if([a runModal]!=NSAlertFirstButtonReturn)return;[self operation:@[@"env",@"create",input.stringValue,@"--distro",d.indexOfSelectedItem==0?@"alpine":@"debian"] title:@"Downloading and creating environment…"];}
- (void)start:(id)sender{(void)sender;NSString *n=[self selected];if(n)[self operation:@[@"env",@"terminal",n] title:@"Opening Terminal…"];}
- (void)stop:(id)sender{(void)sender;NSString *n=[self selected];if(n)[self operation:@[@"env",@"stop",n] title:@"Stopping and saving…"];}
- (void)importStore:(id)sender{(void)sender;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseFiles=NO;p.canChooseDirectories=YES;p.message=@"Select an existing, stopped Trames private store (containing manifest and objects). A separate copy will be created.";if([p runModal]!=NSModalResponseOK)return;NSString *n=[self askName:@"Import Existing Store"];if(n)[self operation:@[@"env",@"import",n,p.URL.path] title:@"Importing existing environment…"];}
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender{(void)sender;if(self.task){self.status.stringValue=@"An operation is still running. Wait for it to finish before closing the manager.";return NSTerminateCancel;}return NSTerminateNow;}
- (void)settings:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSDictionary *c=self.rows[self.table.selectedRow][@"config"];if(![c isKindOfClass:NSDictionary.class]){self.status.stringValue=self.rows[self.table.selectedRow][@"error"]?:@"Configuration needs attention.";return;}NSAlert *a=[self dialog:@"Environment Settings" message:@"Stop the environment before changing settings. Memory 'auto' grows with actual use."];
    NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,360,235)];NSMutableArray *fields=[NSMutableArray array];NSArray *names=@[@"CPU slots",@"Memory (MiB / auto)",@"Network"];NSArray *values=@[[c[@"cpus"] description],c[@"memory_mib"],c[@"network"]];for(NSUInteger i=0;i<3;i++){NSTextField *l=label(names[i]);l.frame=NSMakeRect(0,200-i*38,160,24);[v addSubview:l];NSTextField *f=[[NSTextField alloc]initWithFrame:NSMakeRect(170,200-i*38,180,26)];f.stringValue=values[i];[v addSubview:f];[fields addObject:f];}
    NSMutableArray *checks=[NSMutableArray array];NSArray *keys=@[@"windows",@"vulkan",@"video"];NSArray *titles=@[@"Allow app windows",@"Enable Vulkan",@"Enable VideoToolbox / VA-API"];
    for(NSUInteger i=0;i<3;i++){NSButton *b=[NSButton checkboxWithTitle:titles[i] target:nil action:nil];b.frame=NSMakeRect(0,100-i*26,350,24);b.state=[c[keys[i]] boolValue]?NSControlStateValueOn:NSControlStateValueOff;[checks addObject:b];[v addSubview:b];}NSButton *clear=[NSButton checkboxWithTitle:@"Remove all shared folders" target:nil action:nil];clear.frame=NSMakeRect(0,0,350,26);[v addSubview:clear];a.accessoryView=v;if([a runModal]!=NSAlertFirstButtonReturn)return;
    NSMutableArray *arguments=[NSMutableArray arrayWithArray:@[@"env",@"configure",n,@"--cpus",[fields[0] stringValue],@"--memory-mib",[fields[1] stringValue],@"--network",[fields[2] stringValue],@"--windows",[checks[0] state]?@"on":@"off",@"--vulkan",[checks[1] state]?@"on":@"off",@"--video",[checks[2] state]?@"on":@"off"]];if(clear.state)[arguments addObject:@"--clear-mounts"];[self operation:arguments title:@"Saving settings…"];}
- (void)share:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseFiles=NO;p.canChooseDirectories=YES;p.message=@"Select a host folder to share with Linux.";if([p runModal]!=NSModalResponseOK)return;NSAlert *a=[self dialog:@"Shared Folder" message:@"Linux path (for example /work). Stop the environment before changing shared folders."];NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,340,70)];NSTextField *path=[[NSTextField alloc]initWithFrame:NSMakeRect(0,40,340,26)];path.stringValue=@"/work";NSButton *ro=[NSButton checkboxWithTitle:@"Read only" target:nil action:nil];ro.frame=NSMakeRect(0,0,340,26);[v addSubview:path];[v addSubview:ro];a.accessoryView=v;if([a runModal]!=NSAlertFirstButtonReturn)return;[self operation:@[@"env",@"configure",n,ro.state?@"--mount-ro":@"--mount",[NSString stringWithFormat:@"%@=%@",path.stringValue,p.URL.path]] title:@"Saving shared folder…"];}
- (void)backup:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSSavePanel *p=[NSSavePanel savePanel];p.nameFieldStringValue=[n stringByAppendingString:@".trames.tar.gz"];p.message=@"Stop the environment first. Existing backups will not be overwritten.";if([p runModal]==NSModalResponseOK)[self operation:@[@"env",@"backup",n,p.URL.path] title:@"Creating verified backup…"];}
- (void)restore:(id)sender{(void)sender;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseDirectories=NO;p.message=@"Choose a Trames backup.";if([p runModal]!=NSModalResponseOK)return;NSString *n=[self askName:@"Restore as New Environment"];if(n)[self operation:@[@"env",@"restore",n,p.URL.path] title:@"Verifying and restoring backup…"];}
- (void)remove:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSAlert *a=[self dialog:@"Delete Environment?" message:[NSString stringWithFormat:@"All private Linux files in '%@' will be deleted. Shared host folders are preserved. Create a backup first if needed.",n]];a.alertStyle=NSAlertStyleWarning;if([a runModal]==NSAlertFirstButtonReturn)[self operation:@[@"env",@"delete",n,@"--yes"] title:@"Deleting environment…"];}
- (void)doctor:(id)sender{(void)sender;[self run:@[@"env",@"doctor"] title:@"Collecting version and environment information…" completion:nil];}
@end
void trames_manager_run(void){@autoreleasepool{[NSApplication sharedApplication];TramesManager *manager=[TramesManager new];NSApp.delegate=manager;[NSApp run];(void)manager;}}
