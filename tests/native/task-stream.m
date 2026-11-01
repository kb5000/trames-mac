// SPDX-License-Identifier: GPL-3.0-or-later
#import "../../crates/trames-mac/src/task-stream.h"
#import <CoreFoundation/CoreFoundation.h>

static void runTask(NSString *script, NSString *expected, int expectedStatus, BOOL early) {
    NSTask *task=[NSTask new];task.executableURL=[NSURL fileURLWithPath:@"/bin/sh"];task.arguments=@[@"-c",script];
    __block BOOL completed=NO, beforeExit=NO;
    __block NSUInteger callbacks=0;
    NSMutableString *streamed=[NSMutableString string];
    __attribute__((objc_precise_lifetime)) TramesTaskStream *stream=[[TramesTaskStream alloc]initWithTask:task onText:^(NSString *text){
        [streamed appendString:text];if(task.running)beforeExit=YES;
    } completion:^(NSString *text,int code){
        NSCAssert([text isEqual:expected],@"captured output differs: %@",text);
        NSCAssert([streamed isEqual:text],@"streamed output differs");
        NSCAssert(code==expectedStatus,@"wrong exit status");
        completed=YES;callbacks++;CFRunLoopStop(CFRunLoopGetMain());
    }];
    NSError *error=nil;NSCAssert([stream launchAndReturnError:&error],@"launch failed: %@",error);
    CFRunLoopRunInMode(kCFRunLoopDefaultMode,5,NO);
    NSCAssert(completed && callbacks==1,@"completion missing or repeated");
    NSCAssert(!early || beforeExit,@"output only arrived after command exit");
}
int main(void) {@autoreleasepool {
    NSString *expected=@"下载中 😀\n尾行";NSData *data=[expected dataUsingEncoding:NSUTF8StringEncoding];
    NSMutableData *pending=[NSMutableData data];NSMutableString *actual=[NSMutableString string];
    for(NSUInteger i=0;i<data.length;i++) {
        [pending appendData:[data subdataWithRange:NSMakeRange(i,1)]];
        [actual appendString:TramesConsumeUTF8(pending,NO)];NSCAssert(pending.length<=3,@"unbounded decoder carry");
    }
    [actual appendString:TramesConsumeUTF8(pending,YES)];NSCAssert([actual isEqual:expected],@"fragmented UTF-8 was lost");
    uint8_t unfinished[]={0xe4,0xb8};[pending appendBytes:unfinished length:2];
    NSCAssert(TramesConsumeUTF8(pending,NO).length==0,@"premature incomplete character");
    NSCAssert([TramesConsumeUTF8(pending,YES) isEqual:@"�"],@"EOF replacement missing");
    runTask(@"printf '第一行\\n'; sleep 0.4; printf 'stderr\\n' >&2; printf '尾行'",@"第一行\nstderr\n尾行",0,YES);
    runTask(@"printf 'failure tail'; exit 7",@"failure tail",7,NO);
    NSMutableString *large=[NSMutableString string];for(int i=0;i<2000;i++)[large appendString:@"完整输出😀\n"];
    runTask(@"i=0; while [ $i -lt 2000 ]; do printf '完整输出😀\\n'; i=$((i+1)); done",large,0,NO);
    NSTask *missing=[NSTask new];missing.executableURL=[NSURL fileURLWithPath:@"/no-such-trames-command"];
    TramesTaskStream *bad=[[TramesTaskStream alloc]initWithTask:missing onText:^(NSString *s){(void)s;abort();} completion:^(NSString *s,int code){(void)s;(void)code;abort();}];
    NSError *error=nil;NSCAssert(![bad launchAndReturnError:&error] && error,@"missing command did not report launch failure");
    puts("PASS: live output before exit, split UTF-8, EOF tail, stderr, large output, failure and launch cleanup");
    return 0;
}}
