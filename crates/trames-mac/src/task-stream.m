// SPDX-License-Identifier: GPL-3.0-or-later
#import "task-stream.h"

NSString *TramesConsumeUTF8(NSMutableData *pending, BOOL final) {
    const uint8_t *bytes=pending.bytes;
    NSUInteger count=pending.length, end=count;
    if(!final && count) {
        NSUInteger start=count-1;
        while(start && count-start<4 && (bytes[start]&0xc0)==0x80)start--;
        uint8_t lead=bytes[start];
        NSUInteger width=lead>=0xc2&&lead<=0xdf?2:lead>=0xe0&&lead<=0xef?3:lead>=0xf0&&lead<=0xf4?4:0;
        if(width>count-start)end=start;
    }
    if(!end)return @"";
    NSString *text=[[NSString alloc]initWithBytes:bytes length:end encoding:NSUTF8StringEncoding];
    if(!text) {
        [NSString stringEncodingForData:[NSData dataWithBytes:bytes length:end]
            encodingOptions:@{NSStringEncodingDetectionSuggestedEncodingsKey:@[@(NSUTF8StringEncoding)],
                              NSStringEncodingDetectionUseOnlySuggestedEncodingsKey:@YES,
                              NSStringEncodingDetectionAllowLossyKey:@YES,
                              NSStringEncodingDetectionLossySubstitutionKey:@"�"}
            convertedString:&text usedLossyConversion:nil];
    }
    [pending replaceBytesInRange:NSMakeRange(0,end) withBytes:NULL length:0];
    return text?:@"�";
}

@interface TramesTaskStream ()
@property(strong) NSTask *task;
@property(strong) NSPipe *pipe;
@property(strong) NSMutableData *pending;
@property(strong) NSMutableString *output;
@property(copy) void (^onText)(NSString *);
@property(copy) void (^completion)(NSString *, int);
@property BOOL exited;
@property BOOL eof;
@property BOOL completed;
@property int status;
@end

@implementation TramesTaskStream
- (instancetype)initWithTask:(NSTask *)task onText:(void (^)(NSString *))onText completion:(void (^)(NSString *, int))completion {
    if((self=[super init])) {
        self.task=task;self.pipe=[NSPipe pipe];self.pending=[NSMutableData data];self.output=[NSMutableString string];
        self.onText=onText;self.completion=completion;
        task.standardOutput=self.pipe.fileHandleForWriting;
        task.standardError=self.pipe.fileHandleForWriting;
    }
    return self;
}
- (void)publish:(NSString *)text {
    if(text.length){[self.output appendString:text];self.onText(text);}
}
- (void)finishIfReady {
    // Termination and pipe EOF can arrive in either order. Preserve the final
    // buffered bytes before publishing completion or starting another command.
    if(!self.exited || !self.eof || self.completed)return;
    self.completed=YES;
    [self publish:TramesConsumeUTF8(self.pending,YES)];
    [self.pipe.fileHandleForReading closeFile];
    self.task.terminationHandler=nil;
    void (^done)(NSString *,int)=self.completion;
    self.completion=nil;self.onText=nil;
    done([self.output copy],self.status);
}
- (BOOL)launchAndReturnError:(NSError **)error {
    __weak TramesTaskStream *weakSelf=self;
    self.task.terminationHandler=^(NSTask *finished){
        int status=finished.terminationStatus;TramesTaskStream *stream=weakSelf;
        dispatch_async(dispatch_get_main_queue(),^{stream.status=status;stream.exited=YES;[stream finishIfReady];});
    };
    if(![self.task launchAndReturnError:error]) {
        self.task.terminationHandler=nil;
        [self.pipe.fileHandleForWriting closeFile];[self.pipe.fileHandleForReading closeFile];
        self.completion=nil;self.onText=nil;
        return NO;
    }
    [self.pipe.fileHandleForWriting closeFile];
    self.pipe.fileHandleForReading.readabilityHandler=^(NSFileHandle *handle){
        NSData *data=handle.availableData;TramesTaskStream *stream=weakSelf;
        if(!data.length)handle.readabilityHandler=nil;
        dispatch_async(dispatch_get_main_queue(),^{
            if(data.length){[stream.pending appendData:data];[stream publish:TramesConsumeUTF8(stream.pending,NO)];}
            else stream.eof=YES;
            [stream finishIfReady];
        });
    };
    return YES;
}
@end
