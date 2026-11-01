// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#import <CoreFoundation/CoreFoundation.h>
#import "../../crates/trames-mac/src/task-stream.h"

@interface TramesManager : NSObject
@property(strong) NSArray<NSDictionary *> *rows;
@property(strong) NSTableView *table;
@property(strong) NSTask *task;
@property(strong) TramesTaskStream *taskStream;
@property(strong) NSTextField *status;
@property(strong) NSTextView *log;
@property(strong) NSMutableArray<NSButton *> *actions;
- (void)tools:(id)sender;
- (void)run:(NSArray<NSString *> *)args title:(NSString *)title completion:(void(^)(NSString *,int))done;
@end

int main(int argc,const char **argv) {@autoreleasepool {
    if(argc>1 && !strcmp(argv[1],"--fixture")) {
        fputs("正在下载 😀\n",stdout);fflush(stdout);usleep(400000);
        fputs("安装完成，没有末尾换行",stderr);return 0;
    }
    if(argc>2 && !strcmp(argv[1],"env") && !strcmp(argv[2],"list")){puts("[]");return 0;}
    [NSApplication sharedApplication];
    TramesManager *manager=[TramesManager new];
    manager.status=[NSTextField labelWithString:@""];
    manager.log=[[NSTextView alloc]initWithFrame:NSMakeRect(0,0,500,200)];
    manager.log.font=[NSFont monospacedSystemFontOfSize:11 weight:NSFontWeightRegular];
    NSButton *action=[NSButton buttonWithTitle:@"Test" target:nil action:nil];manager.actions=[NSMutableArray arrayWithObject:action];
    __block BOOL beforeExit=NO,completed=NO;
    id token=[NSNotificationCenter.defaultCenter addObserverForName:NSTextStorageDidProcessEditingNotification object:manager.log.textStorage queue:nil usingBlock:^(NSNotification *note){
        (void)note;if(manager.task.running && [manager.log.string containsString:@"正在下载 😀"])beforeExit=YES;
    }];
    [manager run:@[@"--fixture"] title:@"Initializing…" completion:^(NSString *out,int code){
        NSCAssert(beforeExit,@"GUI log was only updated after command termination");
        NSCAssert([manager.log.string isEqual:out] && [out hasSuffix:@"安装完成，没有末尾换行"],@"GUI lost final output");
        NSCAssert(code==0 && !manager.task && !manager.taskStream && action.enabled,@"completion did not restore controls");
        completed=YES;CFRunLoopStop(CFRunLoopGetMain());
    }];
    CFRunLoopRunInMode(kCFRunLoopDefaultMode,5,NO);NSCAssert(completed,@"manager did not complete");
    [NSNotificationCenter.defaultCenter removeObserver:token];
    NSString *previous=[manager.log.string copy];completed=NO;
    [manager run:@[@"env",@"list"] title:@"Refreshing…" completion:^(NSString *out,int code){
        NSCAssert(code==0 && [out isEqual:@"[]\n"],@"quiet JSON response changed");
        NSCAssert([manager.log.string isEqual:previous],@"refresh erased operation log");
        completed=YES;CFRunLoopStop(CFRunLoopGetMain());
    }];
    CFRunLoopRunInMode(kCFRunLoopDefaultMode,5,NO);NSCAssert(completed,@"quiet refresh did not complete");
    manager.rows=@[@{@"name":@"broken",@"config":NSNull.null}];
    manager.table=[NSTableView new];manager.table.dataSource=(id<NSTableViewDataSource>)manager;
    [manager.table reloadData];[manager.table selectRowIndexes:[NSIndexSet indexSetWithIndex:0] byExtendingSelection:NO];
    [manager tools:nil];NSCAssert(!manager.task && [manager.status.stringValue containsString:@"built-in Debian"],@"invalid configuration was not handled");
    puts("PASS: actual manager log updates during initialization, retains final text, restores controls and preserves logs on refresh");
    return 0;
}}
