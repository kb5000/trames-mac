// SPDX-License-Identifier: GPL-3.0-or-later
#import "../../crates/trames-mac/src/window-input.h"
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
@interface RecordingInputContext : NSTextInputContext
@property NSString *source;
@property BOOL handled;
@end
@implementation RecordingInputContext
- (NSString *)selectedKeyboardInputSource{return self.source;}
- (BOOL)handleEvent:(NSEvent *)event{(void)event;self.handled=YES;return YES;}
@end
@interface RecordingView : TramesInputView
@property NSMutableArray<NSDictionary *> *edits;
@property RecordingInputContext *recordingContext;
@end
@implementation RecordingView
- (NSTextInputContext *)inputContext{return self.recordingContext?:[super inputContext];}
- (void)sendTextEdit:(const struct trames_text_edit *)edit {
    NSMutableDictionary *value=[@{@"context":@(edit->context),@"epoch":@(edit->epoch),@"serial":@(edit->serial),@"before":@(edit->before),@"after":@(edit->after),@"begin":@(edit->cursor_begin),@"end":@(edit->cursor_end)} mutableCopy];
    if(edit->preedit)value[@"preedit"]=[[NSString alloc]initWithBytes:edit->preedit length:edit->preedit_length encoding:NSUTF8StringEncoding];
    if(edit->commit)value[@"commit"]=[[NSString alloc]initWithBytes:edit->commit length:edit->commit_length encoding:NSUTF8StringEncoding];
    [self.edits addObject:value];
}
@end
static void text_tests(void) {
    RecordingView *view=[[RecordingView alloc]initWithFrame:NSMakeRect(0,0,400,200)];view.edits=[NSMutableArray new];view.textState=[TramesTextState new];
    const char *text="A😀中Z";
    struct trames_text_info info={.id=7,.epoch=3,.window=1,.version=3,.serial=1,.text=(const uint8_t *)text,.length=strlen(text),.cursor=5,.anchor=1,.caret={10,20,2,18}};
    [view.textState update:&info];assert(NSEqualRanges(view.selectedRange,NSMakeRange(1,2)));
    [view insertText:@"文" replacementRange:NSMakeRange(NSNotFound,0)];
    assert([view.textState.document isEqualToString:@"A文中Z"]);assert([view.edits.lastObject[@"before"] intValue]==0);assert([view.edits.lastObject[@"after"] intValue]==0);
    info.epoch++;info.serial++;info.text=(const uint8_t *)"abc";info.length=3;info.cursor=3;info.anchor=3;[view.textState update:&info];
    [view setMarkedText:@"ni" selectedRange:NSMakeRange(2,0) replacementRange:NSMakeRange(NSNotFound,0)];
    assert([view hasMarkedText]);assert([view.textState.document isEqualToString:@"abcni"]);
    info.serial++;[view.textState update:&info];assert([view.textState.document isEqualToString:@"abcni"]);
    [view setMarkedText:@"你" selectedRange:NSMakeRange(1,0) replacementRange:NSMakeRange(NSNotFound,0)];
    assert([view.edits.lastObject[@"serial"] unsignedIntValue]==info.serial);assert([view.edits.lastObject[@"begin"] intValue]==3);
    [view insertText:@"你好" replacementRange:NSMakeRange(NSNotFound,0)];
    assert(!view.hasMarkedText);assert([view.textState.document isEqualToString:@"abc你好"]);assert([view.edits.lastObject[@"commit"] isEqualToString:@"你好"]);assert([view.edits.lastObject[@"preedit"] isEqualToString:@""]);
    [view setMarkedText:@"拼" selectedRange:NSMakeRange(1,0) replacementRange:NSMakeRange(NSNotFound,0)];
    [view cancelComposition];assert([view.textState.document isEqualToString:@"abc你好"]);assert(!view.hasMarkedText);assert(view.edits.lastObject[@"commit"]==nil);assert([view.edits.lastObject[@"preedit"] isEqualToString:@""]);
    info.id=8;info.epoch++;info.serial++;[view.textState update:&info];NSUInteger count=view.edits.count;view.textState.discarding=YES;
    [view insertText:@"旧候选" replacementRange:NSMakeRange(NSNotFound,0)];assert(view.edits.count==count);assert([view.textState.document isEqualToString:@"abc"]);view.textState.discarding=NO;
    info.version=1;info.epoch++;info.text=(const uint8_t *)text;info.length=strlen(text);info.cursor=5;info.anchor=1;[view.textState update:&info];
    [view setMarkedText:@"ni" selectedRange:NSMakeRange(2,0) replacementRange:NSMakeRange(NSNotFound,0)];assert([view.edits.lastObject[@"before"] intValue]==4);
    [view.textState update:NULL];assert(!view.hasMarkedText);assert(!view.textState.identifier);
    info.epoch++;info.cursor=5;info.anchor=5;[view.textState update:&info];count=view.edits.count;
    [view insertText:@"x" replacementRange:NSMakeRange(2,0)];assert(view.edits.count==count); // surrogate boundary
    RecordingInputContext *context=[[RecordingInputContext alloc]initWithClient:view];view.recordingContext=context;context.source=@"com.apple.keylayout.ABC";
    NSEvent *key=[NSEvent keyEventWithType:NSEventTypeKeyDown location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0 context:nil characters:@"a" charactersIgnoringModifiers:@"a" isARepeat:NO keyCode:0];
    assert(![view handleTextEvent:key]&&!context.handled);
    context.source=@"com.apple.inputmethod.SCIM.Shuangpin";assert([view handleTextEvent:key]&&context.handled);
    context.handled=NO;context.source=@"com.apple.keylayout.ABC";view.textState.marked=NSMakeRange(0,1);assert([view handleTextEvent:key]&&context.handled);
    view.textState.marked=NSMakeRange(NSNotFound,0);
    puts("native-text-ok utf16-selection preedit-serial commit cancel focus-reset v1 v3 physical-keys");
}
static void make_pipe(int fds[2]){
    assert(pipe(fds)==0);for(int i=0;i<2;i++)assert(fcntl(fds[i],F_SETFL,fcntl(fds[i],F_GETFL)|O_NONBLOCK)==0);
    assert(fcntl(fds[1],F_SETNOSIGPIPE,1)==0);
}
static NSString *hostText=@"Host 中文 😀 clipboard",*guestText=@"Guest 中文 😀 clipboard";
@interface ClipboardObserver : NSObject
@property TramesClipboard *clipboard;
@property NSPasteboard *board;
@property int stage;
@property BOOL finished;
@end
@implementation ClipboardObserver
- (void)observeValueForKeyPath:(NSString *)path ofObject:(id)object change:(NSDictionary *)change context:(void *)context {
    (void)path;(void)object;(void)change;(void)context;
    NSString *text=[self.board stringForType:NSPasteboardTypeString];
    if(self.stage==1){
        assert([text isEqualToString:guestText]);self.stage=2;
        int old[2],current[2];make_pipe(old);make_pipe(current);
        trames_clipboard_transfer((__bridge void *)self.clipboard,old[0],0);
        trames_clipboard_transfer((__bridge void *)self.clipboard,current[0],0);
        assert(write(old[1],"stale",5)==5);close(old[1]);assert(write(current[1],"current",7)==7);close(current[1]);
    }else if(self.stage==2){
        assert([text isEqualToString:@"current"]);self.stage=3;
        int fds[2];make_pipe(fds);[self.clipboard transfer:fds[0] toGuest:NO];
        [self.board clearContents];[self.board setString:@"new external copy" forType:NSPasteboardTypeString];
        assert(write(fds[1],"late",4)==4);close(fds[1]);
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW,50*NSEC_PER_MSEC),dispatch_get_main_queue(),^{
            assert([[self.board stringForType:NSPasteboardTypeString] isEqualToString:@"new external copy"]);
            [self.clipboard shutdown];self.finished=YES;CFRunLoopStop(CFRunLoopGetMain());
        });
    }else assert(0);
}
@end
int main(void){@autoreleasepool {
    [NSApplication sharedApplication];[NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];text_tests();
    // Cross many pipe capacity / UI work-budget boundaries without a payload quota.
    hostText=[hostText stringByPaddingToLength:17*1024*1024 withString:@"x" startingAtIndex:0];
    // A private real NSPasteboard keeps all user clipboard formats untouched.
    NSPasteboard *board=[NSPasteboard pasteboardWithUniqueName];[board setString:hostText forType:NSPasteboardTypeString];
    NSString *controlText=@"device 剪贴板 🧪";[board clearContents];[board setString:controlText forType:NSPasteboardTypeString];
    uint8_t controlBytes[4096];NSData *expectedControl=[controlText dataUsingEncoding:NSUTF8StringEncoding];
    intptr_t controlCount=trames_control_clipboard_read((__bridge void *)board,controlBytes,sizeof controlBytes);
    NSCAssert(controlCount==(intptr_t)expectedControl.length && memcmp(controlBytes,expectedControl.bytes,expectedControl.length)==0,@"device clipboard shares native Unicode content");
    NSCAssert(trames_control_clipboard_read((__bridge void *)board,controlBytes,1)==-90,@"device read is bounded");
    const uint8_t fileURL[]="file:///etc/passwd";NSCAssert(trames_control_open_url(fileURL,sizeof(fileURL)-1)==-22,@"UI control cannot open local files");
    const uint8_t invalidUTF8[]={0xff};
    NSCAssert(trames_control_clipboard_write((__bridge void *)board,invalidUTF8,1)==-22,@"invalid text leaves clipboard intact");
    NSCAssert([[board stringForType:NSPasteboardTypeString] isEqual:controlText],@"failed write preserves existing content");
    NSCAssert(trames_control_clipboard_write((__bridge void *)board,expectedControl.bytes,expectedControl.length)==0,@"device clipboard write uses shared backend");

    [board clearContents];[board setString:hostText forType:NSPasteboardTypeString];
    TramesClipboard *clipboard=[[TramesClipboard alloc]initWithPasteboard:board];[clipboard checkForChanges];
    ClipboardObserver *observer=[ClipboardObserver new];observer.clipboard=clipboard;observer.board=board;
    [clipboard addObserver:observer forKeyPath:@"lastChange" options:0 context:NULL];
    int fds[2];make_pipe(fds);int reader=fds[0];
    __block dispatch_source_t source=dispatch_source_create(DISPATCH_SOURCE_TYPE_READ,(uintptr_t)reader,0,dispatch_get_main_queue());NSMutableData *received=[NSMutableData new];
    dispatch_source_set_cancel_handler(source,^{close(reader);});
    dispatch_source_set_event_handler(source,^{
        uint8_t bytes[1024];ssize_t n;
        while((n=read(reader,bytes,sizeof(bytes)))>0)[received appendBytes:bytes length:(NSUInteger)n];
        if(n!=0)return;
        assert([[[NSString alloc]initWithData:received encoding:NSUTF8StringEncoding] isEqualToString:hostText]);dispatch_source_cancel(source);source=nil;
        observer.stage=1;int pair[2];make_pipe(pair);trames_clipboard_transfer((__bridge void *)clipboard,pair[0],0);
        NSData *data=[guestText dataUsingEncoding:NSUTF8StringEncoding];assert(write(pair[1],data.bytes,data.length)==(ssize_t)data.length);close(pair[1]);
    });dispatch_resume(source);trames_clipboard_transfer((__bridge void *)clipboard,fds[1],1);
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,5*NSEC_PER_SEC),dispatch_get_main_queue(),^{if(!observer.finished){fputs("native clipboard timeout\n",stderr);abort();}});
    CFRunLoopRun();assert(observer.finished);[clipboard removeObserver:observer forKeyPath:@"lastChange"];[board releaseGlobally];
    puts("native-clipboard-ok bidirectional-unicode stale-export external-change shutdown");
    return 0;
}}
