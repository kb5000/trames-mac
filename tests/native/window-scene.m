// SPDX-License-Identifier: GPL-3.0-or-later
// Exercise the real native scene assembly across crossing resize dimensions.
#import "../../crates/trames-mac/src/window.m"
#include <assert.h>
void trames_ui_frame_release(void *lease,uint32_t shown,uint32_t time){(void)lease;(void)shown;(void)time;}
@interface SceneTestFamily : TramesFamily
@end
@implementation SceneTestFamily
// Geometry assembly must be tested even when the test window is occluded.
- (BOOL)isVisible{return YES;}
@end
static void check_scene(TramesUI *ui,uint64_t windowID,NSSize windowSize,NSSize oldBuffer,BOOL popup,BOOL display){
    SceneTestFamily *family=[SceneTestFamily new];family.ui=ui;family.identifier=windowID;family.embeddedDisplay=display;
    family.textState=[TramesTextState new];family.frames=[NSMutableArray new];family.overlays=[NSMutableDictionary new];
    family.window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,windowSize.width,windowSize.height) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
    family.window.releasedWhenClosed=NO;family.canvas=canvas(family,NSMakeRect(0,0,windowSize.width,windowSize.height),NSZeroPoint);family.window.contentView=family.canvas;
    family.clientW=family.hostW=windowSize.width;family.clientH=family.hostH=windowSize.height;family.dirty=YES;ui.windows[@(windowID)]=family;
    MTLTextureDescriptor *desc=[MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:1 height:1 mipmapped:NO];
    desc.usage=MTLTextureUsageShaderRead;id<MTLTexture> texture=[ui.device newTextureWithDescriptor:desc];assert(texture);
    struct image images[2]={
        {.kind=2,.root=1,.address=(__bridge void *)texture,.width=1,.height=1,.scale=1,.display_width=oldBuffer.width,.display_height=oldBuffer.height,.source={0,0,1,1}},
        {.kind=2,.address=(__bridge void *)texture,.width=1,.height=1,.scale=1,.x=windowSize.width-20,.y=windowSize.height-20,.display_width=60,.display_height=50,.source={0,0,1,1}}
    };
    struct window_info info={.id=windowID,.revision=1,.width=windowSize.width,.height=windowSize.height,.embedded_display=display,.title="scene resize regression"};
    int result=trames_ui_window((__bridge void *)ui,&info,images,popup?2:1,NULL);assert(result>=0);
    assert(family.overlays.count==((popup&&!display)?1:0));
    if(popup&&!display)assert(family.overlays[@1]);
    [family dispose];[ui.windows removeObjectForKey:@(windowID)];
}
struct ResizeStats {unsigned count,width,height;};
static void resize_event(const void *context,uint64_t id,uint32_t kind,double x,double y,uint32_t a,uint32_t b,uint32_t c){(void)id;(void)x;(void)y;(void)c;if(kind==6){struct ResizeStats *stats=(void *)context;stats->count++;stats->width=a;stats->height=b;}}
static void check_resize(TramesUI *ui){
    struct ResizeStats stats={0};ui.context=&stats;ui.event=resize_event;
    SceneTestFamily *family=[SceneTestFamily new];family.ui=ui;family.identifier=9;family.embeddedDisplay=YES;family.textState=[TramesTextState new];family.frames=[NSMutableArray new];family.overlays=[NSMutableDictionary new];
    family.window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,200,200) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];family.window.releasedWhenClosed=NO;
    family.canvas=canvas(family,NSMakeRect(0,0,200,200),NSZeroPoint);family.window.contentView=family.canvas;family.window.delegate=family;
    family.clientW=family.clientH=200;ui.windows[@9]=family;
    for(int i=0;i<20;i++)[family.window setContentSize:NSMakeSize(220+i,260+i)];
    [family syncSize];assert(stats.count==0 && family.resizeTimer);
    // A delayed guest acknowledgement must not undo the latest native geometry.
    struct window_info oldAck={.id=9,.revision=1,.width=210,.height=210,.embedded_display=1,.title="resize coalescing"};
    assert(trames_ui_window((__bridge void *)ui,&oldAck,NULL,0,NULL)>=0);
    assert(family.canvas.bounds.size.width==239 && family.canvas.bounds.size.height==279);
    [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.3]];
    assert(stats.count==1 && stats.width==239 && stats.height==279);
    family.clientW=239;family.clientH=279;family.pendingResize=NO;
    [family.window setContentSize:NSMakeSize(260,300)];[family syncSize];
    [family.window setContentSize:NSMakeSize(239,279)];[family syncSize];
    [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];assert(stats.count==1);
    [family.window setContentSize:NSMakeSize(280,350)];[family syncSize];
    [family windowDidEndLiveResize:[NSNotification notificationWithName:NSWindowDidEndLiveResizeNotification object:family.window]];
    assert(stats.count==2 && stats.width==280 && stats.height==350 && !family.resizeTimer);
    family.embeddedDisplay=NO;[family.window setContentSize:NSMakeSize(300,400)];[family syncSize];assert(stats.count==3);
    [family dispose];[ui.windows removeObjectForKey:@9];ui.context=NULL;ui.event=NULL;
}
int main(int argc,char **argv){@autoreleasepool{
    assert(argc==2);NSError *error=nil;NSString *shader=[NSString stringWithContentsOfFile:@(argv[1]) encoding:NSUTF8StringEncoding error:&error];assert(shader);
    void *owner=trames_ui_create(shader.UTF8String);TramesUI *ui=(__bridge TramesUI *)owner;if(![ui startGraphics]){trames_ui_destroy(owner);puts("native-scene-skip no Metal device");return 0;}
    check_scene(ui,1,NSMakeSize(180,320),NSMakeSize(320,180),NO,NO);
    check_scene(ui,2,NSMakeSize(320,180),NSMakeSize(180,320),NO,NO);
    check_scene(ui,3,NSMakeSize(180,320),NSMakeSize(320,180),YES,NO);
    check_scene(ui,4,NSMakeSize(180,320),NSMakeSize(320,180),YES,YES);
    check_resize(ui);
    trames_ui_destroy(owner);
    puts("native-scene-ok root/Android clipping, Linux popups and coalesced physical display resize");return 0;
}}
