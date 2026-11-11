// SPDX-License-Identifier: GPL-3.0-or-later
#import "window-input.h"
// AppKit indices are UTF-16; Wayland surrounding/preedit offsets are UTF-8 bytes.
static NSUInteger byte_offset(NSString *text,NSUInteger position) {
    position=MIN(position,text.length);
    if(position && position<text.length && CFStringIsSurrogateHighCharacter([text characterAtIndex:position-1]) && CFStringIsSurrogateLowCharacter([text characterAtIndex:position]))position--;
    return [[text substringToIndex:position] lengthOfBytesUsingEncoding:NSUTF8StringEncoding];
}
static NSUInteger native_offset(NSData *bytes,NSUInteger position) {
    position=MIN(position,bytes.length);const uint8_t *p=bytes.bytes;
    while(position && position<bytes.length && (p[position]&0xc0)==0x80)position--;
    return [[NSString alloc]initWithBytes:p length:position encoding:NSUTF8StringEncoding].length;
}
static BOOL valid_range(NSString *text,NSRange range) {
    if(range.location>text.length || range.length>text.length-range.location)return NO;
    NSUInteger a=range.location,b=NSMaxRange(range);
    return !(a && a<text.length && CFStringIsSurrogateHighCharacter([text characterAtIndex:a-1]) && CFStringIsSurrogateLowCharacter([text characterAtIndex:a])) && !(b && b<text.length && CFStringIsSurrogateHighCharacter([text characterAtIndex:b-1]) && CFStringIsSurrogateLowCharacter([text characterAtIndex:b]));
}
@implementation TramesTextState
- (instancetype)init {if((self=[super init])){_document=@"";_marked=NSMakeRange(NSNotFound,0);}return self;}
- (BOOL)update:(const struct trames_text_info *)info {
    BOOL focusChanged=self.identifier!=(info?info->id:0)||self.epoch!=(info?info->epoch:0);
    if(!info){self.identifier=0;self.epoch=0;self.document=@"";self.selection=NSMakeRange(0,0);self.marked=NSMakeRange(NSNotFound,0);self.cursor=0;return focusChanged;}
    self.caret=NSMakeRect(info->caret[0],info->caret[1],MAX(1,info->caret[2]),MAX(1,info->caret[3]));
    self.identifier=info->id;self.epoch=info->epoch;self.version=info->version;
    BOOL serialChanged=self.serial!=info->serial;self.serial=info->serial;
    // Keep the local preedit mirror, but acknowledge the newest client commit
    // serial even while composing. A stale done serial loses later preedit.
    if(!focusChanged && (self.marked.location!=NSNotFound || info->version==4 || !serialChanged))return NO;
    NSData *bytes=[NSData dataWithBytes:info->text length:info->length];
    self.document=[[NSString alloc]initWithData:bytes encoding:NSUTF8StringEncoding]?:@"";
    NSUInteger cursor=native_offset(bytes,info->cursor),anchor=native_offset(bytes,info->anchor);
    self.selection=NSMakeRange(MIN(cursor,anchor),MAX(cursor,anchor)-MIN(cursor,anchor));self.cursor=cursor;self.marked=NSMakeRange(NSNotFound,0);
    return focusChanged;
}
@end
@implementation TramesInputView
- (BOOL)handleTextEvent:(NSEvent *)event {
    if(!self.textState.identifier || (event.modifierFlags&(NSEventModifierFlagControl|NSEventModifierFlagCommand)))return NO;
    NSTextInputContext *context=self.inputContext;
    NSString *source=context.selectedKeyboardInputSource;
    // Leave ordinary US/ABC keys on the physical Wayland path, preserving DOM
    // keydown, shortcuts and client key repeat. IME/dead keys and Unicode use
    // the native text client; composition keys must never be sent twice.
    if(!self.hasMarkedText && !(event.modifierFlags&NSEventModifierFlagOption) && event.characters.length==1 && [event.characters characterAtIndex:0]<128 && ([source isEqualToString:@"com.apple.keylayout.ABC"]||[source isEqualToString:@"com.apple.keylayout.US"]))return NO;
    return [context handleEvent:event];
}
- (void)sendTextEdit:(const struct trames_text_edit *)edit{(void)edit;}
- (void)sendEditingKey:(uint32_t)key modifiers:(uint32_t)modifiers{(void)key;(void)modifiers;}
- (NSPoint)textSceneOrigin{return NSZeroPoint;}
- (BOOL)hasMarkedText{return self.textState && self.textState.marked.location!=NSNotFound;}
- (NSRange)markedRange{return self.textState.marked;}
- (NSRange)selectedRange{return self.textState.selection;}
- (NSArray<NSAttributedStringKey> *)validAttributesForMarkedText{return @[];}
- (void)editPreedit:(NSString *)preedit commit:(NSString *)commit before:(NSUInteger)before after:(NSUInteger)after {
    TramesTextState *state=self.textState;if(!state.identifier || state.discarding)return;
    NSData *p=[preedit dataUsingEncoding:NSUTF8StringEncoding],*c=[commit dataUsingEncoding:NSUTF8StringEncoding];
    if((preedit&&!p)||(commit&&!c)||p.length>4096||c.length>4096||before>4096||after>4096)return;
    struct trames_text_edit edit={.context=state.identifier,.epoch=state.epoch,.serial=state.serial,.before=(uint32_t)before,.after=(uint32_t)after};
    if(preedit){edit.preedit=p.length?p.bytes:(const uint8_t *)"";edit.preedit_length=p.length;
        NSUInteger start=state.selection.location-state.marked.location,end=start+state.selection.length;
        edit.cursor_begin=(int32_t)byte_offset(preedit,start);edit.cursor_end=(int32_t)byte_offset(preedit,end);}
    if(commit){edit.commit=c.length?c.bytes:(const uint8_t *)"";edit.commit_length=c.length;}
    [self sendTextEdit:&edit];
}
- (void)replaceLocal:(NSRange)range text:(NSString *)value {
    TramesTextState *s=self.textState;s.document=[s.document stringByReplacingCharactersInRange:range withString:value];s.selection=NSMakeRange(range.location+value.length,0);s.cursor=s.selection.location;
}
// A v3 commit/preedit already replaces the selected text; deletion lengths
// cover only the additional surrounding bytes. v1 counts from its cursor.
- (BOOL)deletionForRange:(NSRange)range before:(NSUInteger *)before after:(NSUInteger *)after {
    TramesTextState *s=self.textState;
    NSUInteger cursor=byte_offset(s.document,s.cursor),start=byte_offset(s.document,range.location),end=byte_offset(s.document,NSMaxRange(range));
    if(s.version==4)return !range.length && range.location==s.cursor;
    if(start>cursor||end<cursor)return NO;
    if(s.version==3 && s.selection.length){
        NSUInteger lower=byte_offset(s.document,s.selection.location),upper=byte_offset(s.document,NSMaxRange(s.selection));
        if(start>lower||end<upper)return NO;
        *before=lower-start;*after=end-upper;
    }else{*before=cursor-start;*after=end-cursor;}
    return *before<=4096 && *after<=4096;
}
- (void)insertText:(id)string replacementRange:(NSRange)replacement {
    NSString *value=[string isKindOfClass:NSAttributedString.class]?[string string]:string;
    TramesTextState *s=self.textState;if(!s.identifier||s.discarding||![value isKindOfClass:NSString.class]||[value lengthOfBytesUsingEncoding:NSUTF8StringEncoding]>4096)return;
    NSRange range=replacement.location==NSNotFound?([self hasMarkedText]?s.marked:s.selection):replacement;
    if(!valid_range(s.document,range))return;
    if([self hasMarkedText]){
        if(!NSEqualRanges(range,s.marked))return;
        [self editPreedit:@"" commit:value before:0 after:0];
    }else{
        NSUInteger before=0,after=0;
        if(![self deletionForRange:range before:&before after:&after])return;
        [self editPreedit:nil commit:value before:before after:after];
    }
    [self replaceLocal:range text:value];s.marked=NSMakeRange(NSNotFound,0);
}
- (void)setMarkedText:(id)string selectedRange:(NSRange)selected replacementRange:(NSRange)replacement {
    NSString *value=[string isKindOfClass:NSAttributedString.class]?[string string]:string;
    TramesTextState *s=self.textState;if(!s.identifier||s.discarding||![value isKindOfClass:NSString.class]||[value lengthOfBytesUsingEncoding:NSUTF8StringEncoding]>4096)return;
    NSRange range=replacement.location==NSNotFound?([self hasMarkedText]?s.marked:s.selection):replacement;
    if(!valid_range(s.document,range))return;
    NSUInteger before=0,after=0;
    if([self hasMarkedText]){if(!NSEqualRanges(range,s.marked))return;}
    else if(![self deletionForRange:range before:&before after:&after])return;
    [self replaceLocal:range text:value];s.marked=NSMakeRange(range.location,value.length);
    NSUInteger begin=MIN(selected.location,value.length),count=MIN(selected.length,value.length-begin);
    s.selection=NSMakeRange(range.location+begin,count);
    [self editPreedit:value commit:nil before:before after:after];
    if(!value.length)s.marked=NSMakeRange(NSNotFound,0);
}
- (void)unmarkText {
    TramesTextState *s=self.textState;if([self hasMarkedText]&&valid_range(s.document,s.marked)){
        [self editPreedit:@"" commit:[s.document substringWithRange:s.marked] before:0 after:0];s.marked=NSMakeRange(NSNotFound,0);
    }
}
- (void)cancelComposition {
    if([self hasMarkedText]){[self editPreedit:@"" commit:nil before:0 after:0];[self replaceLocal:self.textState.marked text:@""];self.textState.marked=NSMakeRange(NSNotFound,0);}
    self.textState.discarding=YES;[self.inputContext discardMarkedText];self.textState.discarding=NO;
}
- (NSAttributedString *)attributedSubstringForProposedRange:(NSRange)range actualRange:(NSRangePointer)actual {
    if(!valid_range(self.textState.document,range)){if(actual)*actual=NSMakeRange(NSNotFound,0);return nil;}
    if(actual)*actual=range;return [[NSAttributedString alloc]initWithString:[self.textState.document substringWithRange:range]];
}
- (NSRect)firstRectForCharacterRange:(NSRange)range actualRange:(NSRangePointer)actual {
    if(actual)*actual=NSMakeRange(self.textState.selection.location,0);(void)range;NSRect caret=self.textState.caret;NSPoint origin=[self textSceneOrigin];caret.origin.x-=origin.x;caret.origin.y-=origin.y;
    return [self.window convertRectToScreen:[self convertRect:caret toView:nil]];
}
- (NSUInteger)characterIndexForPoint:(NSPoint)point{(void)point;return self.textState.selection.location;}
- (NSInteger)windowLevel{return self.window.level;}
- (BOOL)drawsVerticallyForCharacterAtIndex:(NSUInteger)index{(void)index;return NO;}
- (void)doCommandBySelector:(SEL)selector {
    if(self.textState.discarding)return;
    NSString *name=NSStringFromSelector(selector);uint32_t key=0,mods=0;
    NSDictionary *keys=@{@"deleteBackward:":@14,@"deleteForward:":@111,@"moveLeft:":@105,@"moveRight:":@106,@"moveUp:":@103,@"moveDown:":@108,@"moveToBeginningOfLine:":@102,@"moveToEndOfLine:":@107,@"insertNewline:":@28,@"insertTab:":@15,@"insertBacktab:":@15,@"cancelOperation:":@1,@"moveWordLeft:":@105,@"moveWordRight:":@106,@"deleteWordBackward:":@14};
    key=[keys[name] unsignedIntValue];if([name isEqualToString:@"insertBacktab:"])mods=1;
    if([name hasPrefix:@"moveWord"]||[name isEqualToString:@"deleteWordBackward:"])mods=4;
    if(key)[self sendEditingKey:key modifiers:mods];else [super doCommandBySelector:selector];
}
@end
