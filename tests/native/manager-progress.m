// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#import <CoreFoundation/CoreFoundation.h>
#import "../../crates/trames-mac/src/manager-ui.h"

int main(int argc,const char **argv){@autoreleasepool {
    if(argc>1&&!strcmp(argv[1],"--fixture")){fputs("正在下载 😀\n",stdout);fflush(stdout);usleep(400000);fputs("安装完成，没有末尾换行",stderr);return 0;}
    if(argc>1&&!strcmp(argv[1],"--other")){fputs("independent task\n",stdout);fflush(stdout);usleep(100000);fputs("other done",stdout);return 0;}
    if(argc>1&&!strcmp(argv[1],"_manager")){puts("{\"version\":1,\"environments\":[],\"networks\":[],\"fields\":[],\"capabilities\":[],\"global\":{}}");return 0;}
    [NSApplication sharedApplication];TramesManager *manager=[TramesManager new];
    NSCAssert([manager respondsToSelector:@selector(saveConfiguration:)] && [manager respondsToSelector:@selector(create:)],@"archive dropped manager action categories");manager.selectedKey=@"first";
    manager.status=[NSTextField labelWithString:@""];manager.log=[[NSTextView alloc]initWithFrame:NSMakeRect(0,0,500,200)];manager.log.font=[NSFont monospacedSystemFontOfSize:11 weight:NSFontWeightRegular];
    __block BOOL early=NO;__block NSUInteger completed=0;
    id token=[NSNotificationCenter.defaultCenter addObserverForName:NSTextStorageDidProcessEditingNotification object:manager.log.textStorage queue:nil usingBlock:^(NSNotification *note){(void)note;if(manager.jobs[@"first"].task.running&&[manager.log.string containsString:@"正在下载 😀"])early=YES;}];
    [manager run:@[@"--fixture"] owner:@"first" name:@"first" title:@"Initializing" quiet:NO input:nil completion:^(NSString *out,int code){
        NSCAssert(early,@"manager output only appeared after process exit");NSCAssert(code==0&&[out hasSuffix:@"安装完成，没有末尾换行"],@"final UTF-8 output lost");
        NSCAssert([manager.log.string isEqual:out],@"another environment's output contaminated the visible log");completed++;if(completed==2)CFRunLoopStop(CFRunLoopGetMain());
    }];
    [manager run:@[@"--other"] owner:@"second" name:@"second" title:@"Independent" quiet:NO input:nil completion:^(NSString *out,int code){NSCAssert(code==0&&[out isEqual:@"independent task\nother done"],@"independent task failed");NSCAssert(manager.jobs[@"first"].task.running,@"second environment waited for the first one");completed++;if(completed==2)CFRunLoopStop(CFRunLoopGetMain());}];
    NSCAssert(manager.jobs[@"first"].task.running&&manager.jobs[@"second"].task.running,@"manager globally serialized independent operations");
    CFRunLoopRunInMode(kCFRunLoopDefaultMode,5,NO);NSCAssert(completed==2,@"manager tasks did not finish");
    [NSNotificationCenter.defaultCenter removeObserver:token];NSString *previous=[manager.log.string copy];__block BOOL queried=NO;
    [manager run:@[@"_manager",@"snapshot"] owner:@"test-query" name:@"" title:@"Query" quiet:YES input:nil completion:^(NSString *out,int code){NSCAssert(code==0&&[out containsString:@"environments"],@"query failed");NSCAssert([manager.log.string isEqual:previous],@"refresh erased visible operation output");queried=YES;CFRunLoopStop(CFRunLoopGetMain());}];
    CFRunLoopRunInMode(kCFRunLoopDefaultMode,5,NO);NSCAssert(queried,@"query did not finish");
    NSCAssert(!manager.jobs[@"first"].task&&!manager.jobs[@"first"].stream&&manager.jobs[@"first"].complete,@"task lifecycle retained completed resources");
    [manager initializeModel];manager.rows=@[@{@"name":@"first",@"path":@"first",@"config":@{@"environment":@{@"uuid":@"first"}}}];manager.table=[NSTableView new];[manager.table addTableColumn:[[NSTableColumn alloc]initWithIdentifier:@"env"]];manager.table.dataSource=manager;[manager.table reloadData];[manager.table selectRowIndexes:[NSIndexSet indexSetWithIndex:0] byExtendingSelection:NO];manager.page=@"networks";[manager selectEnvironment:nil];NSCAssert([manager.page isEqual:@"env"],@"clicking selected environment did not leave global settings");
    puts("PASS: independent environment operations, live GUI output, UTF-8 final tail, quiet snapshots and task cleanup");return 0;
}}
