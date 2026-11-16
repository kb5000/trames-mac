// SPDX-License-Identifier: GPL-3.0-or-later
#import "manager-ui.h"
void TramesManagerLoadActions(void) {}

static NSTextField *input(NSString *value,NSString *placeholder){NSTextField *f=[NSTextField new];f.stringValue=value?:@"";f.placeholderString=placeholder;[f.widthAnchor constraintGreaterThanOrEqualToConstant:280].active=YES;return f;}
static NSAlert *dialog(NSString *title,NSString *help){NSAlert *a=[NSAlert new];a.messageText=T(title);a.informativeText=T(help);[a addButtonWithTitle:T(@"Continue")];[a addButtonWithTitle:T(@"Cancel")];return a;}
static NSView *accessory(NSArray *rows,CGFloat width){NSStackView *s=TMStack(rows,YES);s.alignment=NSLayoutAttributeWidth;NSView *v=[[NSView alloc]initWithFrame:NSMakeRect(0,0,width,MAX(70,rows.count*65))];TMPin(s,v,0);return v;}
static NSString *askName(NSString *title){NSAlert *a=dialog(title,@"Use letters, numbers, '-' or '_'. Existing environments are never replaced.");NSTextField *f=input(@"",T(@"Environment name"));a.accessoryView=accessory(@[f],370);[a.window setInitialFirstResponder:f];return [a runModal]==NSAlertFirstButtonReturn?f.stringValue:nil;}
static NSURL *pick(NSString *message,BOOL directories){NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseFiles=!directories;p.canChooseDirectories=directories;p.message=T(message);return [p runModal]==NSModalResponseOK?p.URL:nil;}
static void editFile(NSString *path){NSURL *url=[NSURL fileURLWithPath:path];NSWorkspace *w=NSWorkspace.sharedWorkspace;NSURL *app=[w URLForApplicationToOpenURL:url]?:[w URLForApplicationWithBundleIdentifier:@"com.apple.TextEdit"];if(app)[w openURLs:@[url] withApplicationAtURL:app configuration:[NSWorkspaceOpenConfiguration configuration] completionHandler:nil];else [w openURL:url];}

@interface TMCreateController : NSWindowController
@property(weak) TramesManager *manager;
@property NSStackView *body;
@property NSTextField *name;
@property NSTextField *source;
@property NSPopUpButton *boot;
@property NSPopUpButton *packages;
@property NSButton *clipboard;
@property NSComboBox *cpus;
@property NSComboBox *memory;
@property NSString *kind;
@property NSInteger step;
@end
@implementation TMCreateController
- (instancetype)initWithManager:(TramesManager *)manager {
    NSPanel *panel=[[NSPanel alloc]initWithContentRect:NSMakeRect(0,0,620,600) styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    if((self=[super initWithWindow:panel])){self.manager=manager;self.kind=@"alpine";self.name=input(@"",T(@"Environment name"));self.source=input(@"",T(@"Choose a file or directory…"));self.source.editable=NO;
        self.boot=[[NSPopUpButton alloc]initWithFrame:NSZeroRect pullsDown:NO];[self.boot addItemsWithTitles:@[@"shell",@"systemd"]];self.packages=[[NSPopUpButton alloc]initWithFrame:NSZeroRect pullsDown:NO];[self.packages addItemsWithTitles:@[T(@"Daily tools (default)"),T(@"Daily + development tools"),T(@"Minimal base only")]];
        self.cpus=TMResourceCombo(@"resources.cpus",@"all");self.memory=TMResourceCombo(@"resources.memory",@"auto");self.clipboard=[NSButton checkboxWithTitle:T(@"Allow clipboard access on this Mac") target:nil action:nil];self.clipboard.state=NSControlStateValueOn;
        self.body=TMStack(@[],YES);self.body.alignment=NSLayoutAttributeWidth;self.body.edgeInsets=NSEdgeInsetsMake(24,24,20,24);TMPin(self.body,panel.contentView,0);panel.title=T(@"New Environment");panel.becomesKeyOnlyIfNeeded=NO;[self render];}
    return self;
}
- (void)render {
    for(NSView *v in self.body.arrangedSubviews.copy){[self.body removeArrangedSubview:v];[v removeFromSuperview];}
    [self.body addArrangedSubview:TMLabel(T(@"New Environment"),22,NO)];NSArray *steps=@[@"1 · Choose source",@"2 · Configure",@"3 · Review and authorize"];[self.body addArrangedSubview:TMLabel(T(steps[self.step]),12,YES)];
    if(self.step==0){NSArray *choices=@[@[@"alpine",@"Alpine — lightweight Linux"],@[@"debian",@"Debian — daily tools and optional systemd"],@[@"android",@"Android — Waydroid images and adapters"],@[@"custom",@"Custom ARM64 rootfs"],@[@"profile",@"From a TOML profile"],@[@"backup",@"Restore a backup"]];
        for(NSArray *choice in choices){TMButton *b=TMButtonMake(choice[1],self,@selector(kind:),choice[0]);b.state=[self.kind isEqual:choice[0]]?NSControlStateValueOn:NSControlStateValueOff;b.bezelStyle=NSBezelStylePush;b.buttonType=NSButtonTypeToggle;[self.body addArrangedSubview:b];}
    }else if(self.step==1){[self.body addArrangedSubview:TMRow(@"Environment name",@"",self.name)];
        if([@[@"custom",@"android",@"profile",@"backup"] containsObject:self.kind]){[self.body addArrangedSubview:TMLabel([self.kind isEqual:@"android"]?T(@"Select the Android adapter package. Official Waydroid images are downloaded separately."):T(@"Choose an ARM64 rootfs, TOML profile or Trames backup."),12,YES)];[self.body addArrangedSubview:self.source];[self.body addArrangedSubview:TMButtonMake(@"Choose…",self,@selector(choose:),nil)];}
        if([self.kind isEqual:@"debian"]){[self.body addArrangedSubview:TMRow(@"Boot mode",@"",self.boot)];[self.body addArrangedSubview:TMRow(@"Debian tools",@"",self.packages)];}
        if(![@[@"profile",@"backup"] containsObject:self.kind]){[self.body addArrangedSubview:TMRow(@"CPU cores",@"all, or a positive core count",self.cpus)];[self.body addArrangedSubview:TMRow(@"Memory",@"auto shares host memory; a size is a demand-backed limit",self.memory)];}
    }else{[self.body addArrangedSubview:TMGroup(@[TMRow(@"Environment name",@"",TMLabel(self.name.stringValue,13,NO)),TMRow(@"Source",@"",TMLabel(self.source.stringValue.length?self.source.stringValue:self.kind,12,YES))])];[self.body addArrangedSubview:self.clipboard];[self.body addArrangedSubview:TMLabel(T(@"Host command execution and dynamic controls are not granted by importing a profile. Shared folders can be authorized later."),12,YES)];[self.body addArrangedSubview:TMLabel(T(@"The new environment receives its own identity. Existing environments are not replaced."),12,YES)];}
    NSView *gap=[NSView new];[gap.heightAnchor constraintGreaterThanOrEqualToConstant:5].active=YES;[gap setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationVertical];[self.body addArrangedSubview:gap];
    NSMutableArray *actions=[NSMutableArray arrayWithObject:TMButtonMake(@"Cancel",self,@selector(cancel:),nil)];if(self.step)[actions addObject:TMButtonMake(@"Back",self,@selector(back:),nil)];[actions addObject:TMButtonMake(self.step==2?@"Create Environment":@"Continue",self,@selector(next:),nil)];NSView *actionGap=[NSView new];[actionGap setContentHuggingPriority:1 forOrientation:NSLayoutConstraintOrientationHorizontal];[actions insertObject:actionGap atIndex:0];[self.body addArrangedSubview:TMStack(actions,NO)];
}
- (void)kind:(TMButton *)sender{self.kind=sender.payload;self.source.stringValue=@"";[self render];}
- (void)choose:(id)sender{(void)sender;NSOpenPanel *p=[NSOpenPanel openPanel];p.canChooseDirectories=[self.kind isEqual:@"custom"];p.canChooseFiles=YES;if([p runModal]==NSModalResponseOK)self.source.stringValue=p.URL.path;}
- (void)cancel:(id)sender{(void)sender;[self.manager.window endSheet:self.window];[self close];self.manager.createController=nil;}
- (void)back:(id)sender{(void)sender;self.step--;[self render];}
- (void)next:(id)sender {
    (void)sender;if(self.step==0){self.step++;[self render];return;}
    if(self.step==1){NSString *n=self.name.stringValue;NSCharacterSet *allowed=[NSCharacterSet characterSetWithCharactersInString:@"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_"];if(!n.length||n.length>40||[n rangeOfCharacterFromSet:allowed.invertedSet].location!=NSNotFound){NSBeep();return;}
        if([@[@"custom",@"android",@"profile",@"backup"] containsObject:self.kind]&&!self.source.stringValue.length){NSBeep();return;}self.step++;[self render];return;}
    NSString *name=self.name.stringValue;NSMutableArray *args=[NSMutableArray arrayWithArray:@[@"env",@"create",name]];
    if([self.kind isEqual:@"backup"])args=[NSMutableArray arrayWithArray:@[@"env",@"restore",name,self.source.stringValue]];
    else if([self.kind isEqual:@"profile"])[args addObjectsFromArray:@[@"--config",self.source.stringValue,@"--new-identity"]];
    else {if([self.kind isEqual:@"custom"])[args addObjectsFromArray:@[@"--rootfs",self.source.stringValue]];else [args addObjectsFromArray:@[@"--distro",self.kind]];
        if([self.kind isEqual:@"android"])[args addObjectsFromArray:@[@"--rootfs",self.source.stringValue]];
        if(![self.kind isEqual:@"android"])[args addObjectsFromArray:@[@"--init",[self.kind isEqual:@"debian"]?self.boot.titleOfSelectedItem:@"shell"]];
        [args addObjectsFromArray:@[@"--cpus",TMString(TMResourceValue(self.cpus)),@"--memory",TMString(TMResourceValue(self.memory))]];
        if([self.kind isEqual:@"debian"])[args addObjectsFromArray:@[@"--packages",@[@"standard",@"development",@"minimal"][self.packages.indexOfSelectedItem]]];}
    TramesManager *manager=self.manager;BOOL clipboard=self.clipboard.state==NSControlStateValueOn;[self cancel:nil];manager.pendingSelectionName=name;
    NSString *key=[@"create:" stringByAppendingString:name];
    [manager run:args owner:key name:name title:T(@"Creating environment and preparing tools…") quiet:NO input:nil completion:^(NSString *out,int code){(void)out;if(!code){manager.pendingSelectionName=name;if(!clipboard)[manager run:@[@"env",@"grant",name,@"clipboard",@"off"] owner:[@"grant:" stringByAppendingString:name] name:name title:T(@"Saving host grant…") quiet:NO input:nil completion:nil];}}];
}
@end

@interface TMPortsController : NSWindowController <NSTableViewDataSource,NSTableViewDelegate>
@property(weak) TramesManager *manager;
@property NSString *key;
@property NSMutableArray<NSMutableDictionary *> *rules;
@property NSTableView *table;
@end
@implementation TMPortsController
- (NSInteger)numberOfRowsInTableView:(NSTableView *)table{(void)table;return self.rules.count;}
- (id)tableView:(NSTableView *)table objectValueForTableColumn:(NSTableColumn *)column row:(NSInteger)row{(void)table;return self.rules[row][column.identifier];}
- (void)tableView:(NSTableView *)table setObjectValue:(id)value forTableColumn:(NSTableColumn *)column row:(NSInteger)row{(void)table;self.rules[row][column.identifier]=[column.identifier isEqual:@"guest"]?@([value integerValue]):value;}
- (void)add:(id)sender{(void)sender;[self.rules addObject:[@{@"protocol":@"tcp",@"host":@"127.0.0.1:8080",@"guest":@80} mutableCopy]];[self.table reloadData];}
- (void)remove:(id)sender{(void)sender;if(self.table.selectedRow>=0){[self.rules removeObjectAtIndex:self.table.selectedRow];[self.table reloadData];}}
- (void)cancel:(id)sender{(void)sender;[self.manager.window endSheet:self.window];[self close];self.manager.createController=nil;}
- (void)save:(id)sender{(void)sender;[self.window makeFirstResponder:nil];self.manager.drafts[self.key][@"network.publish"]=[self.rules copy];[self cancel:nil];[self.manager render];}
@end

@implementation TramesManager (Actions)
- (void)create:(id)sender{(void)sender;if(self.createController)return;TMCreateController *c=[[TMCreateController alloc]initWithManager:self];self.createController=c;[self.window beginSheet:c.window completionHandler:nil];}
- (void)start:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSDictionary *r=[self selectedRow];if([r[@"state"] hasPrefix:@"running"])[self operation:@[@"env",@"terminal",n] title:T(@"Opening environment…")];else [self operation:@[@"env",@"start",n] title:T(@"Starting environment…")];}
- (void)newTerminal:(id)sender{(void)sender;NSString *name=[self selected];if(name)[self operation:@[@"env",@"terminal",name] title:T(@"Opening environment…")];}
- (void)stop:(id)sender{(void)sender;NSString *n=[self selected];if(n&&[self confirm:@"Stop environment?" message:@"This ends its terminals and services. Other environments keep running."])[self operation:@[@"env",@"stop",n] title:T(@"Stopping and saving…")];}
- (void)more:(id)sender {
    NSMenu *menu=[NSMenu new];NSArray *names=@[@"Set as Default",@"Install Tools…",@"Prepare systemd…",@"Install APK…",@"Backup…",@"Restore…",@"Import Store…",@"Show Configuration in Finder",@"Diagnostics",@"Delete…"];
    SEL actions[]={@selector(setDefault:),@selector(tools:),@selector(prepareSystemd:),@selector(installAPK:),@selector(backup:),@selector(restore:),@selector(importStore:),@selector(revealConfig:),@selector(doctor:),@selector(remove:)};
    for(NSUInteger i=0;i<names.count;i++){NSMenuItem *item=[[NSMenuItem alloc]initWithTitle:T(names[i]) action:actions[i] keyEquivalent:@""];item.target=self;[menu addItem:item];}
    NSView *view=sender;[menu popUpMenuPositioningItem:nil atLocation:NSMakePoint(0,view.bounds.size.height) inView:view];
}
- (void)setDefault:(id)sender{(void)sender;NSString *n=[self selected];if(n)[self operation:@[@"config",@"--global",@"set",@"default_environment",n] title:T(@"Saving default environment…")];}
- (void)globalSetting:(NSPopUpButton *)sender{NSString *n=sender.titleOfSelectedItem;if(n)[self run:@[@"config",@"--global",@"set",@"default_environment",n] owner:@"global-settings" name:@"" title:T(@"Saving default environment…") quiet:NO input:nil completion:nil];}
- (void)prepareSystemd:(id)sender{(void)sender;NSString *n=[self selected];if(n&&[self confirm:@"Prepare systemd?" message:@"Stop the environment first. This installs the required guest dependencies."])[self operation:@[@"env",@"setup",n,@"--init",@"systemd"] title:T(@"Preparing systemd…")];}
- (void)tools:(id)sender {
    (void)sender;NSDictionary *r=[self selectedRow];if(![TMValue(r,@"config.image.distribution") isEqual:@"debian"]){[self showError:T(@"Tool presets are available for built-in Debian environments.")];return;}
    NSAlert *a=dialog(@"Install Debian Tools",@"Stop the environment first. Install daily tools or add C/C++ and Python development tools. Existing files and settings are preserved.");NSPopUpButton *p=[[NSPopUpButton alloc]initWithFrame:NSZeroRect pullsDown:NO];[p addItemsWithTitles:@[T(@"Daily tools"),T(@"Daily + development tools")]];a.accessoryView=accessory(@[p],370);if([a runModal]==NSAlertFirstButtonReturn)[self operation:@[@"env",@"packages",r[@"name"],p.indexOfSelectedItem?@"development":@"standard"] title:T(@"Installing Debian tools…")];
}
- (void)installAPK:(id)sender{(void)sender;if(![TMValue([self selectedRow],@"config.environment.type") isEqual:@"android"]){[self showError:T(@"Choose an Android environment first.")];return;}NSURL *url=pick(@"Choose an Android APK to install. Open the desktop and let Android finish starting first.",NO);if(url)[self operation:@[@"-e",[self selected],@"apk",@"install",url.path] title:T(@"Installing APK…")];}
- (void)saveConfiguration:(id)sender {
    (void)sender;[self.window makeFirstResponder:nil];NSDictionary *row=[self selectedRow];if(!row||![self draft].count)return;
    NSString *key=self.selectedKey,*name=row[@"name"];NSDictionary *changes=[[self draft] copy];NSString *revision=self.revisions[key]?:row[@"revision"];
    NSData *body=[NSJSONSerialization dataWithJSONObject:@{@"version":@1,@"revision":revision,@"changes":changes} options:0 error:nil];__weak TramesManager *weakSelf=self;
    [self run:@[@"_manager",@"validate",key] owner:[@"query:validate:" stringByAppendingString:key] name:name title:T(@"Checking configuration…") quiet:YES input:body completion:^(NSString *out,int code){
        TramesManager *owner=weakSelf;if(code){[owner showError:out];return;}NSDictionary *result=[NSJSONSerialization JSONObjectWithData:[out dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];NSMutableString *summary=[NSMutableString new];for(NSDictionary *change in result[@"changes"])[summary appendFormat:@"%@\n%@ → %@\n\n",change[@"path"],TMString(change[@"before"]),TMString(change[@"after"])];
        NSAlert *a=dialog(@"Save configuration?",@"The running environment continues unchanged. Saved settings apply on the next start.");a.informativeText=[NSString stringWithFormat:@"%@\n\n%@",name,T(@"The running environment continues unchanged. Saved settings apply on the next start.")];
        NSTextView *text=[[NSTextView alloc]initWithFrame:NSMakeRect(0,0,520,230)];text.editable=NO;text.string=summary;text.font=[NSFont monospacedSystemFontOfSize:12 weight:NSFontWeightRegular];NSScrollView *s=[[NSScrollView alloc]initWithFrame:text.frame];s.documentView=text;TMConfigureScroll(s);a.accessoryView=s;
        [a beginSheetModalForWindow:owner.window completionHandler:^(NSModalResponse response){if(response!=NSAlertFirstButtonReturn)return;
            [owner run:@[@"_manager",@"save",key] owner:key name:name title:T(@"Saving configuration…") quiet:NO input:body completion:^(NSString *output,int status){(void)output;if(!status){if([owner.drafts[key] isEqual:changes])[owner.drafts removeObjectForKey:key];[owner.revisions removeObjectForKey:key];[owner.pendingRestart addObject:key];}}];
        }];
    }];
}
- (void)discardConfiguration:(id)sender{(void)sender;[self.window makeFirstResponder:nil];[self.drafts removeObjectForKey:self.selectedKey];[self.revisions removeObjectForKey:self.selectedKey];[self render];}
- (void)externalEditor:(id)sender {
    (void)sender;NSDictionary *row=[self selectedRow];NSString *path=row[@"profile"]?:[row[@"path"] stringByAppendingPathComponent:@"env.toml"];if(!path)return;
    if([NSFileManager.defaultManager fileExistsAtPath:path]){editFile(path);return;}
    if(![self confirm:@"Create a host profile?" message:@"This copies the effective guest settings into a host env.toml. Future starts use that explicit profile."])return;
    NSData *body=[NSJSONSerialization dataWithJSONObject:@{@"version":@1,@"revision":row[@"revision"],@"changes":@{}} options:0 error:nil];
    [self run:@[@"_manager",@"save",self.selectedKey] owner:self.selectedKey name:row[@"name"] title:T(@"Creating host profile…") quiet:NO input:body completion:^(NSString *out,int code){(void)out;if(!code)editFile(path);}];
}
- (void)revealConfig:(id)sender{(void)sender;NSString *path=[self selectedRow][@"path"];if(path)[NSWorkspace.sharedWorkspace activateFileViewerSelectingURLs:@[[NSURL fileURLWithPath:path]]];}
- (void)permission:(NSPopUpButton *)sender {
    NSString *cap=sender.identifier;BOOL allow=sender.indexOfSelectedItem==1;NSDictionary *row=[self selectedRow];NSString *key=self.selectedKey,*base=row[@"revision"];
    NSDictionary *description=nil;for(NSDictionary *c in self.snapshot[@"capabilities"])if([c[@"key"] isEqual:cap])description=c;
    if(!description)return;
    if(allow&&![self confirm:@"Allow this host capability?" message:[NSString stringWithFormat:@"%@\n\n%@",T(description[@"title"]),T(description[@"help"])]]){[sender selectItemAtIndex:0];return;}
    __weak TramesManager *weakSelf=self;
    [self run:@[@"_manager",@"permission",key,cap,allow?@"on":@"off",base] owner:key name:row[@"name"] title:T(@"Saving host grant…") quiet:NO input:nil completion:^(NSString *out,int code){
        TramesManager *owner=weakSelf;if(code){[owner showError:out];return;}NSDictionary *result=[NSJSONSerialization JSONObjectWithData:[out dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
        if([owner.revisions[key] isEqual:base])owner.revisions[key]=result[@"revision"];
        [owner.pendingRestart addObject:key];
    }];
}
- (void)share:(id)sender {
    (void)sender;NSURL *url=pick(@"Select a host folder to share with Linux.",YES);if(!url)return;
    NSAlert *a=dialog(@"Shared Folder",@"The host path stays in this Mac's grants. TOML stores only the share alias and guest path.");NSTextField *alias=input(@"project",T(@"Share alias")),*target=input(@"/work",T(@"Guest path"));NSButton *ro=[NSButton checkboxWithTitle:T(@"Read only") target:nil action:nil];a.accessoryView=accessory(@[TMRow(@"Share alias",@"",alias),TMRow(@"Guest path",@"",target),ro],530);if([a runModal]!=NSAlertFirstButtonReturn)return;
    NSString *key=self.selectedKey,*name=[self selected];NSDictionary *row=[self selectedRow];NSMutableArray *mounts=[NSMutableArray arrayWithArray:[self draft][@"mounts"]?:row[@"config"][@"mounts"]?:@[]];[mounts filterUsingPredicate:[NSPredicate predicateWithBlock:^BOOL(NSDictionary *m,NSDictionary *bindings){(void)bindings;return ![m[@"target"] isEqual:target.stringValue];}]];[mounts addObject:@{@"share":alias.stringValue,@"target":target.stringValue,@"readonly":[NSNumber numberWithBool:ro.state==NSControlStateValueOn]}];
    __weak TramesManager *weakSelf=self;[self run:@[@"env",@"grant",name,@"share",alias.stringValue,url.path,ro.state?@"ro":@"rw"] owner:key name:name title:T(@"Authorizing shared folder…") quiet:NO input:nil completion:^(NSString *out,int code){(void)out;if(code)return;TramesManager *owner=weakSelf;if(!owner.drafts[key])owner.drafts[key]=[NSMutableDictionary new];owner.drafts[key][@"mounts"]=mounts;if(!owner.revisions[key])owner.revisions[key]=row[@"revision"];owner.tabs.selectedSegment=2;}];
}
- (void)revokeShare:(TMButton *)sender{if([self confirm:@"Revoke this shared folder?" message:@"This removes the directory grant on this Mac. Existing environment files are preserved."])[self operation:@[@"env",@"grant",[self selected],@"unshare",sender.payload] title:T(@"Saving host grant…")];}
- (void)removeMount:(TMButton *)sender{NSMutableArray *mounts=[NSMutableArray arrayWithArray:[self draft][@"mounts"]?:[self selectedRow][@"config"][@"mounts"]?:@[]];NSUInteger i=[sender.payload unsignedIntegerValue];if(i<mounts.count){[mounts removeObjectAtIndex:i];[self change:@"mounts" value:mounts];[self render];}}
- (void)editPorts:(id)sender {
    (void)sender;if(self.createController)return;TMPortsController *c=[TMPortsController new];c.manager=self;c.key=self.selectedKey;if(!self.revisions[c.key])self.revisions[c.key]=[self selectedRow][@"revision"];[self draft];
    c.rules=[NSMutableArray new];for(NSDictionary *p in [self draft][@"network.publish"]?:[self selectedRow][@"config"][@"network"][@"publish"])[c.rules addObject:[p mutableCopy]];
    c.window=[[NSPanel alloc]initWithContentRect:NSMakeRect(0,0,600,350) styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];NSStackView *root=TMStack(@[],YES);root.alignment=NSLayoutAttributeWidth;root.edgeInsets=NSEdgeInsetsMake(20,20,20,20);TMPin(root,c.window.contentView,0);[root addArrangedSubview:TMLabel(T(@"Published ports"),20,NO)];
    c.table=[NSTableView new];c.table.dataSource=c;c.table.delegate=c;for(NSArray *column in @[@[@"protocol",@"Protocol"],@[@"host",@"Host address and port"],@[@"guest",@"Guest port"]]){NSTableColumn *col=[[NSTableColumn alloc]initWithIdentifier:column[0]];col.title=T(column[1]);col.editable=YES;col.width=[column[0] isEqual:@"host"]?280:110;[c.table addTableColumn:col];}NSScrollView *scroll=[NSScrollView new];scroll.documentView=c.table;TMConfigureScroll(scroll);[scroll.heightAnchor constraintGreaterThanOrEqualToConstant:160].active=YES;[root addArrangedSubview:scroll];[root addArrangedSubview:TMStack(@[TMButtonMake(@"Add",c,@selector(add:),nil),TMButtonMake(@"Remove",c,@selector(remove:),nil),TMButtonMake(@"Cancel",c,@selector(cancel:),nil),TMButtonMake(@"Keep Changes",c,@selector(save:),nil)],NO)];self.createController=c;[self.window beginSheet:c.window completionHandler:nil];
}
- (void)backup:(id)sender{(void)sender;NSString *n=[self selected];if(!n)return;NSSavePanel *p=[NSSavePanel savePanel];p.nameFieldStringValue=[n stringByAppendingString:@".trames.tar.gz"];p.message=T(@"Stop the environment first. Existing backups will not be overwritten.");if([p runModal]==NSModalResponseOK)[self operation:@[@"env",@"backup",n,p.URL.path] title:T(@"Creating verified backup…")];}
- (void)restore:(id)sender{(void)sender;NSURL *url=pick(@"Choose a Trames backup.",NO);if(!url)return;NSString *n=askName(@"Restore as New Environment");if(n){self.pendingSelectionName=n;[self run:@[@"env",@"restore",n,url.path] owner:[@"create:" stringByAppendingString:n] name:n title:T(@"Verifying and restoring backup…") quiet:NO input:nil completion:nil];}}
- (void)importStore:(id)sender{(void)sender;NSURL *url=pick(@"Select an existing, stopped Trames private store (containing manifest and objects). A separate copy will be created.",YES);if(!url)return;NSString *n=askName(@"Import Existing Store");if(n){self.pendingSelectionName=n;[self run:@[@"env",@"import-store",n,url.path] owner:[@"create:" stringByAppendingString:n] name:n title:T(@"Importing existing environment…") quiet:NO input:nil completion:nil];}}
- (void)remove:(id)sender{(void)sender;NSString *n=[self selected];if(n&&[self confirm:@"Delete Environment?" message:T(@"All private files in this environment will be deleted. Shared host folders are preserved. Create a backup first if needed.")])[self operation:@[@"env",@"remove",n,@"--yes"] title:T(@"Deleting environment…")];}
- (void)doctor:(id)sender{(void)sender;[self operation:@[@"env",@"doctor"] title:T(@"Collecting version and environment information…")];}
- (void)createNetwork:(id)sender{(void)sender;NSAlert *a=dialog(@"New Trames Network",@"Private networking belongs to Trames; no host bridge is installed.");NSTextField *name=input(@"development",T(@"Network name"));NSButton *nat=[NSButton checkboxWithTitle:T(@"Allow external access through NAT") target:nil action:nil];nat.state=NSControlStateValueOn;a.accessoryView=accessory(@[name,nat],400);if([a runModal]==NSAlertFirstButtonReturn){NSMutableArray *args=[NSMutableArray arrayWithArray:@[@"network",@"create",name.stringValue]];if(!nat.state)[args addObject:@"--internal"];[self run:args owner:[@"network:" stringByAppendingString:name.stringValue] name:name.stringValue title:T(@"Creating network…") quiet:NO input:nil completion:nil];}}
@end
