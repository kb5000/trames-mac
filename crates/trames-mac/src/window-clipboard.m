// SPDX-License-Identifier: GPL-3.0-or-later
#import "window-input.h"
#include <errno.h>
#include <unistd.h>
// Clipboard payloads use ordinary pipes and the host's available memory.
@class TramesClipboardTransfer;
@interface TramesClipboard ()
@property NSPasteboard *pasteboard;
@property NSMutableSet<TramesClipboardTransfer *> *transfers;
@property uint64_t generation;
@property BOOL stopped;
- (void)exportData:(NSData *)data generation:(uint64_t)generation change:(NSInteger)change;
@end
@interface TramesClipboardTransfer : NSObject
@property(weak) TramesClipboard *owner;
@property dispatch_source_t source,timer;
@property NSMutableData *received;
@property NSData *sending;
@property NSUInteger offset;
@property int fd;
@property BOOL toGuest,finished;
@property uint64_t generation;
@property NSInteger change;
- (void)start;
- (void)finish;
@end
@implementation TramesClipboardTransfer
- (void)start {
    int fd=self.fd;
    self.source=dispatch_source_create(self.toGuest?DISPATCH_SOURCE_TYPE_WRITE:DISPATCH_SOURCE_TYPE_READ,(uintptr_t)fd,0,dispatch_get_main_queue());
    dispatch_source_set_event_handler(self.source,^{[self pump];});
    // Close only after libdispatch has released the FD registration, preventing
    // descriptor reuse from redirecting a queued callback to a different pipe.
    dispatch_source_set_cancel_handler(self.source,^{close(fd);});
    self.timer=dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER,0,0,dispatch_get_main_queue());
    dispatch_source_set_timer(self.timer,dispatch_time(DISPATCH_TIME_NOW,120*NSEC_PER_SEC),DISPATCH_TIME_FOREVER,NSEC_PER_SEC);
    dispatch_source_set_event_handler(self.timer,^{[self finish];});
    dispatch_resume(self.source);dispatch_resume(self.timer);
}
- (void)pump {
    if(self.finished)return;
    // A large text transfer yields the existing UI queue after 64 KiB. FD
    // readiness resumes it; no polling, busy wait or clipboard worker is used.
    NSUInteger work=0;
    while(work<65536){
        if(self.toGuest){
            if(self.offset==self.sending.length){[self finish];return;}
            ssize_t n=write(self.fd,(const uint8_t *)self.sending.bytes+self.offset,MIN(self.sending.length-self.offset,65536-work));
            if(n>0){self.offset+=(NSUInteger)n;work+=(NSUInteger)n;continue;}
            if(n<0&&(errno==EAGAIN||errno==EWOULDBLOCK))return;
            if(n<0&&errno==EINTR)continue;
            [self finish];return;
        }
        uint8_t bytes[16384];ssize_t n=read(self.fd,bytes,MIN(sizeof(bytes),65536-work));
        if(n>0){
            [self.received appendBytes:bytes length:(NSUInteger)n];work+=(NSUInteger)n;continue;
        }
        if(n<0&&(errno==EAGAIN||errno==EWOULDBLOCK))return;
        if(n<0&&errno==EINTR)continue;
        if(n==0)[self.owner exportData:self.received generation:self.generation change:self.change];
        [self finish];return;
    }
}
- (void)finish {
    if(self.finished)return;self.finished=YES;
    dispatch_source_cancel(self.source);dispatch_source_cancel(self.timer);self.source=nil;self.timer=nil;
    self.sending=nil;self.received=nil;[self.owner.transfers removeObject:self];self.fd=-1;
}
@end
@implementation TramesClipboard
- (instancetype)initWithPasteboard:(NSPasteboard *)pasteboard {
    if((self=[super init])){_pasteboard=pasteboard;_lastChange=-1;_transfers=[NSMutableSet new];}return self;
}
- (NSPasteboard *)board {return self.pasteboard?:NSPasteboard.generalPasteboard;}
- (void)checkForChanges {
    if(self.stopped)return;NSInteger count=self.board.changeCount;
    if(count!=self.lastChange){self.lastChange=count;if(self.changed)self.changed();}
}
- (void)cancelExports {
    self.generation++;
    for(TramesClipboardTransfer *transfer in [self.transfers copy])if(!transfer.toGuest)[transfer finish];
}
- (void)shutdown {
    self.stopped=YES;self.changed=nil;
    for(TramesClipboardTransfer *transfer in [self.transfers copy])[transfer finish];
}
- (void)transfer:(int)fd toGuest:(BOOL)toGuest {
    if(self.stopped){close(fd);return;}
    if(!toGuest)[self cancelExports];
    TramesClipboardTransfer *transfer=[TramesClipboardTransfer new];transfer.owner=self;transfer.fd=fd;transfer.toGuest=toGuest;
    transfer.change=self.board.changeCount;transfer.generation=self.generation;
    if(toGuest){
        NSString *text=[self.board stringForType:NSPasteboardTypeString]?:@"";
        transfer.sending=[text dataUsingEncoding:NSUTF8StringEncoding]?:[NSData data];
    }else transfer.received=[NSMutableData new];
    [self.transfers addObject:transfer];[transfer start];
}
- (void)exportData:(NSData *)data generation:(uint64_t)generation change:(NSInteger)change {
    if(self.stopped||generation!=self.generation||change!=self.board.changeCount)return;
    NSString *text=[[NSString alloc]initWithData:data encoding:NSUTF8StringEncoding];if(!text)return;
    [self.board clearContents];[self.board setString:text forType:NSPasteboardTypeString];self.lastChange=self.board.changeCount;
}
@end
void trames_clipboard_release(void *pointer){id object=CFBridgingRelease(pointer);(void)object;}
void trames_clipboard_transfer(void *pointer,int fd,uint32_t to_guest){
    TramesClipboard *clipboard=(__bridge TramesClipboard *)pointer;
    dispatch_async(dispatch_get_main_queue(),^{[clipboard transfer:fd toGuest:to_guest!=0];});
}
void trames_clipboard_changed(void *pointer){
    TramesClipboard *clipboard=(__bridge TramesClipboard *)pointer;
    dispatch_async(dispatch_get_main_queue(),^{[clipboard cancelExports];});
}

// Host-owned pointers only. Device and Wayland controls use the same native
// pasteboard; these bounded text operations require no proxy/resource thread.
intptr_t trames_control_clipboard_read(void *board,uint8_t *bytes,size_t capacity){
    @autoreleasepool {
        NSPasteboard *pasteboard=board?(__bridge NSPasteboard *)board:NSPasteboard.generalPasteboard;
        NSString *text=[pasteboard stringForType:NSPasteboardTypeString]?:@"";
        if(text.length>capacity)return -90;
        NSData *data=[text dataUsingEncoding:NSUTF8StringEncoding];
        if(data.length>capacity)return -90;
        if(data.length)memcpy(bytes,data.bytes,data.length);
        return (intptr_t)data.length;
    }
}
intptr_t trames_control_clipboard_write(void *board,const uint8_t *bytes,size_t length){
    @autoreleasepool {
        NSPasteboard *pasteboard=board?(__bridge NSPasteboard *)board:NSPasteboard.generalPasteboard;
        NSString *text=[[NSString alloc]initWithBytes:bytes length:length encoding:NSUTF8StringEncoding];if(!text)return -22;
        [pasteboard clearContents];return [pasteboard setString:text forType:NSPasteboardTypeString]?0:-5;
    }
}

intptr_t trames_control_open_url(const uint8_t *bytes,size_t length){
    @autoreleasepool {
        NSString *text=[[NSString alloc]initWithBytes:bytes length:length encoding:NSUTF8StringEncoding];
        NSURL *url=text?[NSURL URLWithString:text]:nil;
        if(!url||![@[@"http",@"https"] containsObject:url.scheme.lowercaseString]||!url.host.length)return -22;
        return [NSWorkspace.sharedWorkspace openURL:url]?1:0;
    }
}
