// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>
#import <simd/simd.h>
#include <stdatomic.h>
#include <stdint.h>
struct image { uint32_t kind; void *address; size_t length; uint32_t offset,width,height,stride,alpha; int32_t x,y,scale; uint32_t transform; };
struct window_info { uint64_t id,revision; uint32_t interaction_serial,interaction_edges; uint32_t width,height,min_width,min_height,max_width,max_height,mode; const char *title; };
_Static_assert(sizeof(struct image)==64,"Rust image layout");
_Static_assert(sizeof(struct window_info)==64,"Rust window layout");
struct uniform { vector_float2 origin,size,viewport; uint32_t offset,stride,width,height,alpha,transform; };
extern void trames_ui_frame_release(void *, uint32_t, uint32_t);
typedef void (*refresh_fn)(const void *);
typedef void (*event_fn)(const void *,uint64_t,uint32_t,double,double,uint32_t,uint32_t,uint32_t);
@class TramesUI, TramesFamily;
static void wake(CFRunLoopSourceRef source) { CFRunLoopSourceSignal(source); CFRunLoopWakeUp(CFRunLoopGetMain()); }
// A lease outlives both GPU reads and every drawable's presentation callback.
// Cancellation suppresses frame acknowledgement without releasing GPU sources.
@interface TramesFrame : NSObject {
    NSLock *_lock; void *_lease; CFRunLoopSourceRef _source;
    BOOL _gpuDone,_cancelled,_finished,_error,_presented; NSUInteger _pending;
}
@property(readonly) BOOL finished;
@property(readonly) BOOL error;
@property(readonly) BOOL presented;
- (instancetype)initWithLease:(void *)lease source:(CFRunLoopSourceRef)source count:(NSUInteger)count;
- (void)gpuDone:(BOOL)success;
- (void)shown:(BOOL)success;
- (void)cancel;
@end
@implementation TramesFrame
- (instancetype)initWithLease:(void *)lease source:(CFRunLoopSourceRef)source count:(NSUInteger)count {
    if((self=[super init])){_lock=[NSLock new];_lease=lease;_source=(CFRunLoopSourceRef)CFRetain(source);_pending=count;}return self;
}
- (BOOL)finished {[_lock lock];BOOL value=_finished;[_lock unlock];return value;}
- (BOOL)error {[_lock lock];BOOL value=_error;[_lock unlock];return value;}
- (BOOL)presented {[_lock lock];BOOL value=_presented;[_lock unlock];return value;}
- (void)finishLocked {
    if(_finished || !_gpuDone || (!_cancelled && _pending)){[_lock unlock];return;}
    _finished=YES;_presented=!_cancelled && !_error;void *lease=_lease;_lease=NULL;BOOL shown=_presented;
    [_lock unlock];
    trames_ui_frame_release(lease,shown,(uint32_t)(NSProcessInfo.processInfo.systemUptime*1000));
    wake(_source);
}
- (void)gpuDone:(BOOL)success {[_lock lock];_gpuDone=YES;_error=!success;_cancelled|=!success;[self finishLocked];}
- (void)shown:(BOOL)success {[_lock lock];if(_pending)_pending--;_cancelled|=!success;[self finishLocked];}
- (void)cancel {[_lock lock];_cancelled=YES;[self finishLocked];}
- (void)dealloc { if(_source)CFRelease(_source); }
@end
@interface TramesCanvas : NSView
@property(weak) TramesFamily *family;
@property NSPoint origin;
@end
// A toplevel can have SHM shadows or popups outside its content rectangle.
// These child windows route input to the same Wayland window and scene origin.
@interface TramesOverlay : NSWindow @end
@implementation TramesOverlay
- (BOOL)canBecomeKeyWindow{return YES;}
@end
@interface TramesFamily : NSObject<NSWindowDelegate>
@property(weak) TramesUI *ui;
@property NSWindow *window;
@property TramesCanvas *canvas;
@property NSMutableDictionary<NSNumber *,NSWindow *> *overlays;
@property NSMutableArray<TramesFrame *> *frames;
@property uint64_t identifier,lastRevision;
@property uint32_t clientW,clientH,hostW,hostH,mode;
@property BOOL seen,dirty,updating,closing,pendingResize,buttonDown;
@property uint32_t interactionSerial,dragEdges;
@property NSPoint dragMouse;
@property NSRect dragFrame;
- (void)drag;
- (void)emit:(uint32_t)kind x:(double)x y:(double)y a:(uint32_t)a b:(uint32_t)b c:(uint32_t)c;
- (void)cancel;
- (void)dispose;
@end
@interface TramesUI : NSObject<NSApplicationDelegate> {
@public _Atomic(bool) finished;
}
@property CFRunLoopSourceRef source;
@property const void *context;
@property refresh_fn refresh;
@property event_fn event;
@property NSString *shader;
@property NSMutableDictionary<NSNumber *,TramesFamily *> *windows;
@property NSMutableIndexSet *pressed;
@property uint64_t focus;
@property BOOL refreshing,refreshAgain;
@property id<MTLDevice> device;
@property id<MTLCommandQueue> queue;
@property id<MTLRenderPipelineState> shmPipeline,texturePipeline;
- (BOOL)startGraphics;
- (void)tick;
- (void)keys:(NSEvent *)event down:(BOOL)down family:(TramesFamily *)family;
- (void)quit:(id)sender;
- (void)closeFocused:(id)sender;
- (void)minimizeFocused:(id)sender;
@end
static uint32_t modifiers(NSEventModifierFlags flags) {
    return ((flags&NSEventModifierFlagShift)?1:0)|((flags&NSEventModifierFlagCapsLock)?2:0)|((flags&NSEventModifierFlagControl)?4:0)|((flags&NSEventModifierFlagOption)?8:0)|((flags&NSEventModifierFlagCommand)?64:0);
}
static const uint16_t keys[128]={
 [0]=30,[1]=31,[2]=32,[3]=33,[4]=35,[5]=34,[6]=44,[7]=45,[8]=46,[9]=47,[10]=86,[11]=48,[12]=16,[13]=17,[14]=18,[15]=19,[16]=21,[17]=20,
 [18]=2,[19]=3,[20]=4,[21]=5,[22]=7,[23]=6,[24]=13,[25]=10,[26]=8,[27]=12,[28]=9,[29]=11,[30]=27,[31]=24,[32]=22,[33]=26,[34]=23,[35]=25,[36]=28,[37]=38,[38]=36,[39]=40,[40]=37,[41]=39,[42]=43,[43]=51,[44]=53,[45]=49,[46]=50,[47]=52,[48]=15,[49]=57,[50]=41,[51]=14,[53]=1,[54]=126,[55]=125,[56]=42,[57]=58,[58]=56,[59]=29,[60]=54,[61]=100,[62]=97,
 [65]=83,[67]=55,[69]=78,[71]=69,[75]=98,[76]=96,[78]=74,[81]=117,[82]=82,[83]=79,[84]=80,[85]=81,[86]=75,[87]=76,[88]=77,[89]=71,[91]=72,[92]=73,
 [96]=63,[97]=64,[98]=65,[99]=61,[100]=66,[101]=67,[103]=87,[105]=99,[107]=70,[109]=68,[111]=88,[113]=119,[114]=110,[115]=102,[116]=104,[117]=111,[118]=62,[119]=107,[120]=60,[121]=109,[122]=59,[123]=105,[124]=106,[125]=108,[126]=103
};
@implementation TramesCanvas
- (BOOL)isFlipped{return YES;}
- (BOOL)acceptsFirstResponder{return YES;}
- (BOOL)acceptsFirstMouse:(NSEvent *)event{(void)event;return YES;}
- (CALayer *)makeBackingLayer {
    CAMetalLayer *layer=[CAMetalLayer layer];layer.device=self.family.ui.device;layer.pixelFormat=MTLPixelFormatBGRA8Unorm;
    layer.opaque=NO;layer.framebufferOnly=YES;layer.maximumDrawableCount=3;layer.allowsNextDrawableTimeout=YES;layer.presentsWithTransaction=NO;CGColorSpaceRef color=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);layer.colorspace=color;CGColorSpaceRelease(color);return layer;
}
- (void)updateTrackingAreas {
    for(NSTrackingArea *area in self.trackingAreas)[self removeTrackingArea:area];
    [self addTrackingArea:[[NSTrackingArea alloc]initWithRect:NSZeroRect options:NSTrackingMouseMoved|NSTrackingMouseEnteredAndExited|NSTrackingActiveInKeyWindow|NSTrackingInVisibleRect owner:self userInfo:nil]];
    [super updateTrackingAreas];
}
- (void)pointer:(NSEvent *)event {
    NSPoint p=[self convertPoint:event.locationInWindow fromView:nil];
    [self.family emit:2 x:p.x+self.origin.x y:p.y+self.origin.y a:0 b:0 c:0];
}
- (void)mouseMoved:(NSEvent *)event{[self pointer:event];}
- (void)mouseDragged:(NSEvent *)event{[self.family drag];[self pointer:event];}
- (void)rightMouseDragged:(NSEvent *)event{[self.family drag];[self pointer:event];}
- (void)otherMouseDragged:(NSEvent *)event{[self.family drag];[self pointer:event];}
- (void)mouseExited:(NSEvent *)event{(void)event;if(self.family.ui.event)self.family.ui.event(self.family.ui.context,0,2,0,0,0,0,0);}
- (void)button:(NSEvent *)event down:(BOOL)down {
    if(down){self.family.buttonDown=YES;self.family.dragMouse=NSEvent.mouseLocation;self.family.dragFrame=self.family.window.frame;}else{[self.family drag];self.family.buttonDown=NO;self.family.dragEdges=0;}
    [self.window makeFirstResponder:self];[self.family emit:1 x:0 y:0 a:0 b:0 c:0];[self pointer:event];
    uint32_t button=event.buttonNumber==0?0x110:event.buttonNumber==1?0x111:event.buttonNumber==2?0x112:0x110+(uint32_t)event.buttonNumber;
    [self.family emit:3 x:0 y:0 a:button b:down c:0];
}
- (void)mouseDown:(NSEvent *)event{[self button:event down:YES];}
- (void)mouseUp:(NSEvent *)event{[self button:event down:NO];}
- (void)rightMouseDown:(NSEvent *)event{[self button:event down:YES];}
- (void)rightMouseUp:(NSEvent *)event{[self button:event down:NO];}
- (void)otherMouseDown:(NSEvent *)event{[self button:event down:YES];}
- (void)otherMouseUp:(NSEvent *)event{[self button:event down:NO];}
- (void)scrollWheel:(NSEvent *)event{[self pointer:event];double scale=event.hasPreciseScrollingDeltas?1:10;[self.family emit:4 x:-event.scrollingDeltaX*scale y:-event.scrollingDeltaY*scale a:0 b:0 c:0];}
- (void)keyDown:(NSEvent *)event{if(!event.isARepeat)[self.family.ui keys:event down:YES family:self.family];}
- (void)keyUp:(NSEvent *)event{[self.family.ui keys:event down:NO family:self.family];}
- (void)flagsChanged:(NSEvent *)event {
    TramesUI *ui=self.family.ui;BOOL down=![ui.pressed containsIndex:event.keyCode];
    NSEventModifierFlags mask=event.keyCode==56||event.keyCode==60?NSEventModifierFlagShift:event.keyCode==58||event.keyCode==61?NSEventModifierFlagOption:event.keyCode==59||event.keyCode==62?NSEventModifierFlagControl:event.keyCode==54||event.keyCode==55?NSEventModifierFlagCommand:NSEventModifierFlagCapsLock;
    if(!(event.modifierFlags&mask))down=NO;
    if(event.keyCode==57){[ui keys:event down:YES family:self.family];[ui keys:event down:NO family:self.family];}else [ui keys:event down:down family:self.family];
}
@end
@implementation TramesFamily
- (void)emit:(uint32_t)kind x:(double)x y:(double)y a:(uint32_t)a b:(uint32_t)b c:(uint32_t)c { if(kind==1)self.ui.focus=self.identifier; if(self.ui.event)self.ui.event(self.ui.context,self.identifier,kind,x,y,a,b,c); }
- (BOOL)windowShouldClose:(NSWindow *)sender{(void)sender;if(self.closing)return YES;[self emit:7 x:0 y:0 a:0 b:0 c:0];return NO;}
- (void)windowDidBecomeKey:(NSNotification *)notification{(void)notification;self.dirty=YES;[self emit:1 x:0 y:0 a:0 b:0 c:0];wake(self.ui.source);}
- (void)windowDidResize:(NSNotification *)notification {
    if(notification.object!=self.window)return;NSSize size=self.canvas.bounds.size;
    uint32_t w=(uint32_t)llround(size.width),h=(uint32_t)llround(size.height);
    if(w==self.hostW && h==self.hostH)return;
    self.dirty=YES;self.hostW=w;self.hostH=h;
    if(!self.updating){self.pendingResize=YES;[self emit:6 x:0 y:0 a:w b:h c:0];}
    wake(self.ui.source);
}
- (void)windowDidMove:(NSNotification *)notification{if(notification.object!=self.window)return;if(getenv("TRAMES_MAC_WINDOW_TRACE"))fprintf(stderr,"trames mac window moved: id=%llu x=%.0f y=%.0f\n",self.identifier,self.window.frame.origin.x,self.window.frame.origin.y);}
- (void)windowDidChangeBackingProperties:(NSNotification *)notification{(void)notification;self.dirty=YES;wake(self.ui.source);}
- (void)windowDidChangeOcclusionState:(NSNotification *)notification{if(notification.object!=self.window)return;self.dirty=YES;if(!(self.window.occlusionState&NSWindowOcclusionStateVisible))[self cancel];wake(self.ui.source);}
- (void)windowDidMiniaturize:(NSNotification *)notification{(void)notification;[self cancel];if(!self.updating){self.mode=1;[self emit:8 x:0 y:0 a:1 b:0 c:0];}}
- (void)windowDidDeminiaturize:(NSNotification *)notification{(void)notification;self.dirty=YES;if(!self.updating){self.mode=0;[self emit:8 x:0 y:0 a:0 b:0 c:0];}wake(self.ui.source);}
- (BOOL)windowShouldZoom:(NSWindow *)window toFrame:(NSRect)frame {
    (void)frame;if(!self.updating){self.mode=window.zoomed?0:2;[self emit:8 x:0 y:0 a:self.mode b:0 c:0];}return YES;
}
- (void)windowDidExpose:(NSNotification *)notification{(void)notification;self.dirty=YES;wake(self.ui.source);}
- (void)drag {
    if(!self.buttonDown || !self.dragEdges)return;
    NSPoint mouse=NSEvent.mouseLocation;double dx=mouse.x-self.dragMouse.x,dy=mouse.y-self.dragMouse.y;
    NSRect frame=self.dragFrame;
    if(self.dragEdges==16){frame.origin.x+=dx;frame.origin.y+=dy;}else{
        if(self.dragEdges&4)frame.size.width-=dx;if(self.dragEdges&8)frame.size.width+=dx;
        if(self.dragEdges&1)frame.size.height+=dy;if(self.dragEdges&2)frame.size.height-=dy;
        double chromeW=self.window.frame.size.width-self.canvas.bounds.size.width,chromeH=self.window.frame.size.height-self.canvas.bounds.size.height;
        frame.size.width=round(MAX(self.window.contentMinSize.width+chromeW,MIN(frame.size.width,self.window.contentMaxSize.width+chromeW)));
        frame.size.height=round(MAX(self.window.contentMinSize.height+chromeH,MIN(frame.size.height,self.window.contentMaxSize.height+chromeH)));
        if(self.dragEdges&4)frame.origin.x=NSMaxX(self.dragFrame)-frame.size.width;
        if(self.dragEdges&2)frame.origin.y=NSMaxY(self.dragFrame)-frame.size.height;
    }
    [self.window setFrame:frame display:YES];
}
- (void)cancel{for(TramesFrame *frame in self.frames)[frame cancel];}
- (void)dispose {
    self.closing=YES;[self cancel];for(NSWindow *overlay in self.overlays.allValues){[self.window removeChildWindow:overlay];[overlay close];}
    [self.overlays removeAllObjects];[self.window close];
}
@end
static void source_perform(void *info){[(__bridge TramesUI *)info tick];}
@implementation TramesUI
- (instancetype)init {if((self=[super init])){atomic_init(&finished,false);_windows=[NSMutableDictionary new];_pressed=[NSMutableIndexSet new];}return self;}
- (BOOL)startGraphics {
    if(self.device)return YES;
    self.device=MTLCreateSystemDefaultDevice();self.queue=[self.device newCommandQueue];NSError *error=nil;
    id<MTLLibrary> library=[self.device newLibraryWithSource:self.shader options:nil error:&error];
    if(!library||!self.queue){fprintf(stderr,"trames-mac: Metal setup failed: %s\n",error.description.UTF8String);return NO;}
    MTLRenderPipelineDescriptor *p=[MTLRenderPipelineDescriptor new];p.vertexFunction=[library newFunctionWithName:@"waylandVertex"];
    p.colorAttachments[0].pixelFormat=MTLPixelFormatBGRA8Unorm;p.colorAttachments[0].blendingEnabled=YES;
    p.colorAttachments[0].sourceRGBBlendFactor=MTLBlendFactorOne;p.colorAttachments[0].destinationRGBBlendFactor=MTLBlendFactorOneMinusSourceAlpha;
    p.colorAttachments[0].sourceAlphaBlendFactor=MTLBlendFactorOne;p.colorAttachments[0].destinationAlphaBlendFactor=MTLBlendFactorOneMinusSourceAlpha;
    p.fragmentFunction=[library newFunctionWithName:@"waylandShm"];self.shmPipeline=[self.device newRenderPipelineStateWithDescriptor:p error:&error];
    p.fragmentFunction=[library newFunctionWithName:@"waylandTexture"];self.texturePipeline=[self.device newRenderPipelineStateWithDescriptor:p error:&error];
    if(!self.shmPipeline||!self.texturePipeline){fprintf(stderr,"trames-mac: Metal pipeline failed: %s\n",error.description.UTF8String);return NO;}
    if(getenv("TRAMES_MAC_WINDOW_TRACE"))fprintf(stderr,"trames mac windows: starting AppKit\n");
    if(NSApp)return YES;
    [NSApplication sharedApplication];NSApp.delegate=self;[NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    NSMenu *menu=[NSMenu new];NSMenuItem *app=[NSMenuItem new];[menu addItem:app];NSMenu *actions=[NSMenu new];
    NSMenuItem *quit=[[NSMenuItem alloc]initWithTitle:@"Quit Trames" action:@selector(quit:) keyEquivalent:@"q"];quit.target=self;[actions addItem:quit];app.submenu=actions;
    NSMenuItem *windowMenu=[NSMenuItem new];[menu addItem:windowMenu];NSMenu *windowActions=[[NSMenu alloc]initWithTitle:@"Window"];
    NSMenuItem *close=[[NSMenuItem alloc]initWithTitle:@"Close Window" action:@selector(closeFocused:) keyEquivalent:@"w"];close.target=self;[windowActions addItem:close];
    NSMenuItem *minimize=[[NSMenuItem alloc]initWithTitle:@"Minimize" action:@selector(minimizeFocused:) keyEquivalent:@"m"];minimize.target=self;[windowActions addItem:minimize];windowMenu.submenu=windowActions;
    NSApp.mainMenu=menu;NSApp.windowsMenu=windowActions;[NSApp finishLaunching];
    // Switch the initially headless CF loop to the AppKit event pump.
    if(getenv("TRAMES_MAC_WINDOW_TRACE"))fprintf(stderr,"trames mac windows: AppKit ready\n");
    CFRunLoopStop(CFRunLoopGetMain());return YES;
}
- (void)tick {
    if(atomic_load_explicit(&finished,memory_order_relaxed)){
        CFRunLoopStop(CFRunLoopGetMain());
        if(NSApp){[NSApp postEvent:[NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:0 data1:0 data2:0] atStart:NO];}return;
    }
    if(self.refreshing){self.refreshAgain=YES;return;}self.refreshing=YES;
    for(TramesFamily *family in self.windows.allValues){
        for(TramesFrame *frame in [family.frames copy])if(frame.finished){if(!frame.presented)family.dirty=YES;if(frame.error && self.event)self.event(self.context,0,10,0,0,0,0,0);[family.frames removeObject:frame];}
    }
    if(self.refresh)self.refresh(self.context);
    self.refreshing=NO;if(self.refreshAgain){self.refreshAgain=NO;wake(self.source);}
}
- (void)keys:(NSEvent *)event down:(BOOL)down family:(TramesFamily *)family {
    if(event.keyCode>=128||!keys[event.keyCode])return;
    if(down){if(self.focus!=family.identifier)[family emit:1 x:0 y:0 a:0 b:0 c:0];[self.pressed addIndex:event.keyCode];}else [self.pressed removeIndex:event.keyCode];
    [family emit:5 x:0 y:0 a:keys[event.keyCode] b:down c:modifiers(event.modifierFlags)];
}
- (TramesFamily *)focusedFamily {NSView *view=NSApp.keyWindow.contentView;if(![view isKindOfClass:TramesCanvas.class])view=NSApp.mainWindow.contentView;return [view isKindOfClass:TramesCanvas.class]?((TramesCanvas *)view).family:nil;}
- (void)applicationDidBecomeActive:(NSNotification *)notification{(void)notification;TramesFamily *family=[self focusedFamily];if(family)[family emit:1 x:0 y:0 a:0 b:0 c:0];for(TramesFamily *f in self.windows.allValues)f.dirty=YES;wake(self.source);}
- (void)closeFocused:(id)sender{(void)sender;[[self focusedFamily] emit:7 x:0 y:0 a:0 b:0 c:0];}
- (void)minimizeFocused:(id)sender{(void)sender;[[self focusedFamily].window miniaturize:nil];}
- (void)applicationDidResignActive:(NSNotification *)notification{(void)notification;self.focus=0;[self.pressed removeAllIndexes];if(self.event)self.event(self.context,0,1,0,0,0,0,0);}
- (void)quit:(id)sender{(void)sender;if(self.event)self.event(self.context,0,9,0,0,0,0,0);}
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender{(void)sender;[self quit:nil];return NSTerminateCancel;}
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender{(void)sender;return NO;}
- (void)dealloc{if(_source){CFRunLoopSourceInvalidate(_source);CFRelease(_source);}}
@end
static TramesCanvas *canvas(TramesFamily *family,NSRect rect,NSPoint origin){
    TramesCanvas *view=[[TramesCanvas alloc]initWithFrame:rect];view.family=family;view.origin=origin;view.wantsLayer=YES;view.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;return view;
}
void *trames_ui_create(const char *shader){
    TramesUI *ui=[TramesUI new];ui.shader=[NSString stringWithUTF8String:shader];
    CFRunLoopSourceContext context={0};context.info=(__bridge void *)ui;context.perform=source_perform;
    ui.source=CFRunLoopSourceCreate(NULL,0,&context);if(!ui.source)return NULL;
    CFRunLoopAddSource(CFRunLoopGetMain(),ui.source,kCFRunLoopCommonModes);return (void *)CFBridgingRetain(ui);
}
void *trames_ui_source(void *pointer){return (void *)CFRetain(((__bridge TramesUI *)pointer).source);}
void trames_ui_wake(void *pointer){wake((CFRunLoopSourceRef)pointer);}
void trames_ui_release_source(void *pointer){CFRelease(pointer);}
void trames_ui_callbacks(void *pointer,const void *context,refresh_fn refresh,event_fn event){TramesUI *ui=(__bridge TramesUI *)pointer;ui.context=context;ui.refresh=refresh;ui.event=event;}
void trames_ui_finish(void *pointer){TramesUI *ui=(__bridge TramesUI *)pointer;atomic_store_explicit(&ui->finished,true,memory_order_relaxed);wake(ui.source);}
void trames_ui_run(void *pointer){
    TramesUI *ui=(__bridge TramesUI *)pointer;
    while(!atomic_load_explicit(&ui->finished,memory_order_relaxed)){
        @autoreleasepool{if(!NSApp)CFRunLoopRun();else{
            NSEvent *event=[NSApp nextEventMatchingMask:NSEventMaskAny untilDate:NSDate.distantFuture inMode:NSDefaultRunLoopMode dequeue:YES];
            if(event)[NSApp sendEvent:event];[NSApp updateWindows];
        }}
    }
}
void trames_ui_destroy(void *pointer){
    TramesUI *ui=CFBridgingRelease(pointer);CFRunLoopSourceInvalidate(ui.source);
    for(TramesFamily *family in ui.windows.allValues)[family dispose];[ui.windows removeAllObjects];
    if(NSApp.delegate==ui)NSApp.delegate=nil;
}
void trames_ui_begin(void *pointer){for(TramesFamily *family in ((__bridge TramesUI *)pointer).windows.allValues)family.seen=NO;}
void trames_ui_end(void *pointer){
    TramesUI *ui=(__bridge TramesUI *)pointer;
    for(NSNumber *key in [ui.windows.allKeys copy])if(!ui.windows[key].seen){[ui.windows[key] dispose];[ui.windows removeObjectForKey:key];}
    if(NSApp && !ui.windows.count){[NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];ui.queue=nil;ui.shmPipeline=nil;ui.texturePipeline=nil;ui.device=nil;}
}
int trames_ui_window(void *pointer,const struct window_info *info,const struct image *images,size_t count,void *lease){
    TramesUI *ui=(__bridge TramesUI *)pointer;TramesFrame *owned=nil;
    @try{
        if(![ui startGraphics])return -1;
        NSNumber *key=@(info->id);TramesFamily *family=ui.windows[key];
        if(!family){
            family=[TramesFamily new];family.ui=ui;family.identifier=info->id;family.lastRevision=UINT64_MAX;family.dirty=YES;
            family.overlays=[NSMutableDictionary new];family.frames=[NSMutableArray new];family.updating=YES;
            family.hostW=info->width;family.hostH=info->height;
            family.window=[[NSWindow alloc]initWithContentRect:NSMakeRect(0,0,info->width,info->height) styleMask:NSWindowStyleMaskTitled|NSWindowStyleMaskClosable|NSWindowStyleMaskMiniaturizable|NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
            family.window.releasedWhenClosed=NO;family.window.delegate=family;family.window.acceptsMouseMovedEvents=YES;family.window.contentResizeIncrements=NSMakeSize(1,1);
            family.canvas=canvas(family,NSMakeRect(0,0,info->width,info->height),NSZeroPoint);family.window.contentView=family.canvas;
            [family.window center];NSRect frame=family.window.frame;frame.origin.x+=24*ui.windows.count;frame.origin.y-=24*ui.windows.count;[family.window setFrame:frame display:NO];
            ui.windows[key]=family;family.updating=NO;
            if(getenv("TRAMES_MAC_WINDOW_TRACE"))fprintf(stderr,"trames mac window created: id=%llu title=%s\n",info->id,info->title);
            [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];[family.window makeKeyAndOrderFront:nil];[family.window makeFirstResponder:family.canvas];[NSApp activate];
        }
        family.seen=YES;family.updating=YES;
        family.window.title=[NSString stringWithUTF8String:info->title]?:@"Linux";
        family.window.contentMinSize=NSMakeSize(MAX(1,MIN(info->min_width,4096)),MAX(1,MIN(info->min_height,4096)));
        family.window.contentMaxSize=NSMakeSize(MAX(family.window.contentMinSize.width,info->max_width?MIN(info->max_width,4096):4096),MAX(family.window.contentMinSize.height,info->max_height?MIN(info->max_height,4096):4096));
        if(family.pendingResize && family.hostW==info->width && family.hostH==info->height)family.pendingResize=NO;
        if(!family.pendingResize && family.clientW && (family.clientW!=info->width||family.clientH!=info->height)){
            family.hostW=info->width;family.hostH=info->height;[family.window setContentSize:NSMakeSize(info->width,info->height)];family.dirty=YES;
        }
        family.clientW=info->width;family.clientH=info->height;
        if(family.mode!=info->mode){
            family.mode=info->mode;family.dirty=YES;
            if(info->mode==1)[family.window miniaturize:nil];else{
                if(family.window.miniaturized)[family.window deminiaturize:nil];
                if((info->mode==2)!=family.window.zoomed)[family.window zoom:nil];
            }
        }
        family.updating=NO;
        if(info->interaction_serial && info->interaction_serial!=family.interactionSerial){family.interactionSerial=info->interaction_serial;if(family.buttonDown){family.dragEdges=info->interaction_edges?info->interaction_edges:16;[family drag];}}
        if(info->mode==1 || family.window.miniaturized || !(family.window.occlusionState&NSWindowOcclusionStateVisible))return 0;
        if(!family.dirty && family.lastRevision==info->revision)return 0;
        if(family.frames.count>=2)return 0;
        NSMutableArray<TramesCanvas *> *views=[NSMutableArray arrayWithObject:family.canvas];
        NSMutableSet<NSNumber *> *visible=[NSMutableSet new];NSRect root=family.canvas.bounds;
        for(size_t i=0;i<count;i++){
            const struct image *image=&images[i];double scale=MAX(1,image->scale);
            NSRect rect=NSMakeRect(image->x,image->y,(image->transform&1?image->height:image->width)/scale,(image->transform&1?image->width:image->height)/scale);
            if(NSContainsRect(root,rect))continue;
            // Separate popup rectangles avoid one large transparent window
            // intercepting clicks in the empty space between disjoint surfaces.
            NSNumber *index=@(i);[visible addObject:index];NSWindow *overlay=family.overlays[index];
            if(!overlay){overlay=[[TramesOverlay alloc]initWithContentRect:NSZeroRect styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];overlay.releasedWhenClosed=NO;overlay.opaque=NO;overlay.backgroundColor=NSColor.clearColor;overlay.hasShadow=NO;overlay.delegate=family;overlay.acceptsMouseMovedEvents=YES;family.overlays[index]=overlay;[family.window addChildWindow:overlay ordered:NSWindowAbove];overlay.contentView=canvas(family,NSMakeRect(0,0,rect.size.width,rect.size.height),rect.origin);}
            TramesCanvas *view=(TramesCanvas *)overlay.contentView;view.origin=rect.origin;
            NSRect screen=[family.window convertRectToScreen:[family.canvas convertRect:rect toView:nil]];
            [overlay setFrame:screen display:YES];[views addObject:view];
        }
        for(NSNumber *index in [family.overlays.allKeys copy])if(![visible containsObject:index]){NSWindow *overlay=family.overlays[index];[family.window removeChildWindow:overlay];[overlay close];[family.overlays removeObjectForKey:index];}
        // Keep only maximal presentation rectangles. A CSD shadow can cover
        // the whole root: hide its duplicate layer instead of waiting for a
        // completely occluded root drawable or rendering the same pixels twice.
        NSMutableArray<TramesCanvas *> *targets=[NSMutableArray new];
        for(TramesCanvas *view in views){
            NSRect rect=NSMakeRect(view.origin.x,view.origin.y,view.bounds.size.width,view.bounds.size.height);BOOL covered=NO;
            for(TramesCanvas *other in views){if(other==view)continue;NSRect box=NSMakeRect(other.origin.x,other.origin.y,other.bounds.size.width,other.bounds.size.height);
                if(NSContainsRect(box,rect)&&(!NSEqualRects(box,rect)||[views indexOfObject:other]>[views indexOfObject:view])){covered=YES;break;}}
            // The root view must remain available for key and pointer dispatch even
            // when a child rectangle presents all of its pixels.
            if(view==family.canvas){view.hidden=NO;view.layer.hidden=covered;}
            else if(covered)[view.window orderOut:nil];
            else if(!view.window.visible)[view.window orderWindow:NSWindowAbove relativeTo:family.window.windowNumber];
            if(!covered)[targets addObject:view];
        }
        views=targets;
        NSMutableArray *sources=[NSMutableArray new];
        for(size_t i=0;i<count;i++){
            const struct image *image=&images[i];id source=nil;
            if(image->kind==1)source=[ui.device newBufferWithBytesNoCopy:image->address length:image->length options:MTLResourceStorageModeShared deallocator:nil];
            else if(image->kind==2)source=(__bridge id<MTLTexture>)image->address;
            if(!source){fprintf(stderr,"trames-mac: cannot import retained display image\n");return -1;}[sources addObject:source];
        }
        NSMutableArray<id<CAMetalDrawable>> *drawables=[NSMutableArray new];
        id<MTLCommandBuffer> command=[ui.queue commandBuffer];if(!command)return -1;
        for(TramesCanvas *view in views){
            CAMetalLayer *layer=(CAMetalLayer *)view.layer;CGFloat scale=view.window.backingScaleFactor;
            layer.contentsScale=scale;layer.drawableSize=CGSizeMake(MAX(1,view.bounds.size.width*scale),MAX(1,view.bounds.size.height*scale));
            id<CAMetalDrawable> drawable=[layer nextDrawable];if(!drawable)return 0;
            [drawables addObject:drawable];MTLRenderPassDescriptor *pass=[MTLRenderPassDescriptor renderPassDescriptor];
            pass.colorAttachments[0].texture=drawable.texture;pass.colorAttachments[0].loadAction=MTLLoadActionClear;pass.colorAttachments[0].storeAction=MTLStoreActionStore;pass.colorAttachments[0].clearColor=MTLClearColorMake(0,0,0,0);
            id<MTLRenderCommandEncoder> encoder=[command renderCommandEncoderWithDescriptor:pass];if(!encoder)return -1;
            for(size_t i=0;i<count;i++){
                const struct image *image=&images[i];float divisor=MAX(1,image->scale);
                struct uniform value={.origin={(float)image->x-view.origin.x,(float)image->y-view.origin.y},.size={(image->transform&1?image->height:image->width)/divisor,(image->transform&1?image->width:image->height)/divisor},.viewport={view.bounds.size.width,view.bounds.size.height},.offset=image->offset,.stride=image->stride,.width=image->width,.height=image->height,.alpha=image->alpha,.transform=image->transform};
                [encoder setVertexBytes:&value length:sizeof(value) atIndex:0];[encoder setFragmentBytes:&value length:sizeof(value) atIndex:1];
                if(image->kind==1){[encoder setRenderPipelineState:ui.shmPipeline];[encoder setFragmentBuffer:sources[i] offset:0 atIndex:0];}else{[encoder setRenderPipelineState:ui.texturePipeline];[encoder setFragmentTexture:sources[i] atIndex:0];}
                [encoder drawPrimitives:MTLPrimitiveTypeTriangleStrip vertexStart:0 vertexCount:4];
            }
            [encoder endEncoding];
        }
        owned=[[TramesFrame alloc]initWithLease:lease source:ui.source count:drawables.count];[family.frames addObject:owned];
        for(id<CAMetalDrawable> drawable in drawables){TramesFrame *frame=owned;[drawable addPresentedHandler:^(id<MTLDrawable> shown){[frame shown:shown.presentedTime>0];}];[command presentDrawable:drawable];}
        TramesFrame *frame=owned;[command addCompletedHandler:^(id<MTLCommandBuffer> result){if(result.status!=MTLCommandBufferStatusCompleted)fprintf(stderr,"trames-mac: GPU display command failed: %s\n",result.error.description.UTF8String);[frame gpuDone:result.status==MTLCommandBufferStatusCompleted];}];
        family.lastRevision=info->revision;family.dirty=NO;[command commit];return 1;
    }@catch(NSException *exception){fprintf(stderr,"trames-mac: window error: %s\n",exception.description.UTF8String);if(owned){[owned cancel];[owned gpuDone:NO];return 2;}return -1;}
}

// Linked only by Rust unit tests (dead-stripped from the release entry point).
static void test_source(void *context){(void)context;}
void *trames_ui_test_frame(void *lease,unsigned count){
    CFRunLoopSourceContext context={0};context.perform=test_source;
    CFRunLoopSourceRef source=CFRunLoopSourceCreate(NULL,0,&context);
    TramesFrame *frame=[[TramesFrame alloc]initWithLease:lease source:source count:count];CFRelease(source);return (void *)CFBridgingRetain(frame);
}
void trames_ui_test_frame_event(void *pointer,unsigned event){TramesFrame *frame=(__bridge TramesFrame *)pointer;switch(event){case 1:[frame shown:YES];break;case 2:[frame shown:NO];break;case 3:[frame gpuDone:YES];break;case 4:[frame gpuDone:NO];break;case 5:[frame cancel];break;}}
unsigned trames_ui_test_frame_state(void *pointer){TramesFrame *frame=(__bridge TramesFrame *)pointer;return (frame.finished?1:0)|(frame.presented?2:0);}
void trames_ui_test_frame_drop(void *pointer){id frame=CFBridgingRelease(pointer);(void)frame;}
