// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#include <stdint.h>
struct trames_text_info {
    uint64_t id,epoch,window;
    uint32_t version,serial,cursor,anchor,hint,purpose;
    int32_t caret[4];
    const uint8_t *text; size_t length;
};
struct trames_text_edit {
    uint64_t context,epoch;
    uint32_t serial,before,after;
    int32_t cursor_begin,cursor_end;
    const uint8_t *preedit; size_t preedit_length;
    const uint8_t *commit; size_t commit_length;
};
typedef void (*trames_text_fn)(const void *,const struct trames_text_edit *);
@interface TramesTextState : NSObject
@property uint64_t identifier,epoch;
@property uint32_t version,serial;
@property NSString *document;
@property NSRange selection,marked;
@property NSUInteger cursor;
@property NSRect caret;
@property BOOL discarding;
- (BOOL)update:(const struct trames_text_info *)info;
@end
@interface TramesInputView : NSView<NSTextInputClient>
@property TramesTextState *textState;
- (BOOL)handleTextEvent:(NSEvent *)event;
- (void)sendTextEdit:(const struct trames_text_edit *)edit;
- (void)sendEditingKey:(uint32_t)key modifiers:(uint32_t)modifiers;
- (NSPoint)textSceneOrigin;
- (void)cancelComposition;
@end
@interface TramesClipboard : NSObject
@property(copy) void (^changed)(void);
@property NSInteger lastChange;
- (instancetype)initWithPasteboard:(NSPasteboard *)pasteboard;
- (void)checkForChanges;
- (void)shutdown;
- (void)cancelExports;
- (void)transfer:(int)fd toGuest:(BOOL)toGuest;
@end
void trames_clipboard_release(void *pointer);
void trames_clipboard_transfer(void *pointer,int fd,uint32_t to_guest);
void trames_clipboard_changed(void *pointer);

intptr_t trames_control_clipboard_read(void *board,uint8_t *bytes,size_t capacity);
intptr_t trames_control_clipboard_write(void *board,const uint8_t *bytes,size_t length);

intptr_t trames_control_open_url(const uint8_t *bytes,size_t length);
