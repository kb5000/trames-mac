// SPDX-License-Identifier: GPL-3.0-or-later
#import <AppKit/AppKit.h>
#import "localization.h"
#import "task-stream.h"

// Presentation only. These objects never retain guest execution resources.
@interface TMJob : NSObject
@property NSString *key;
@property NSString *name;
@property NSString *title;
@property NSMutableString *output;
@property NSTask *task;
@property TramesTaskStream *stream;
@property BOOL quiet;
@property BOOL complete;
@property int code;
@end
@interface TMButton : NSButton
@property id payload;
@end
NSTextField *TMLabel(NSString *text, CGFloat size, BOOL secondary);
NSStackView *TMStack(NSArray<NSView *> *views, BOOL vertical);
NSView *TMGroup(NSArray<NSView *> *rows);
NSView *TMRow(NSString *title, NSString *help, NSView *value);
TMButton *TMButtonMake(NSString *title, id target, SEL action, id payload);
id TMValue(NSDictionary *object, NSString *path);
NSString *TMString(id value);
// Explicit archive anchor: Objective-C categories have no natural C reference.
void TramesManagerLoadActions(void);

@interface TramesManager : NSObject <NSApplicationDelegate,NSTableViewDataSource,NSTableViewDelegate,NSSplitViewDelegate,NSTextFieldDelegate>
@property NSWindow *window;
@property NSTableView *table;
@property NSStackView *content;
@property NSStackView *header;
@property NSScrollView *scroll;
@property NSTextField *status;
@property NSTextField *sidebarStatus;
@property NSTextView *log;
@property NSSegmentedControl *tabs;
@property NSButton *saveButton;
@property NSButton *discardButton;
@property NSArray<NSDictionary *> *rows;
@property NSDictionary *snapshot;
@property NSMutableDictionary<NSString *,TMJob *> *jobs;
@property NSMutableDictionary<NSString *,NSMutableDictionary *> *drafts;
@property NSMutableDictionary<NSString *,NSString *> *revisions;
@property NSMutableSet<NSString *> *pendingRestart;
@property NSMutableDictionary<NSString *,NSArray *> *sessions;
@property NSString *selectedKey;
@property NSString *page;
@property NSString *pendingSelectionName;
@property NSMutableArray *watches;
@property BOOL refreshAgain;
@property BOOL refreshing;
@property BOOL rendering;
@property id createController;
- (void)initializeModel;
- (void)selectEnvironment:(id)sender;
- (void)refresh:(id)sender;
- (void)render;
- (NSDictionary *)selectedRow;
- (NSString *)selected;
- (NSString *)keyForRow:(NSDictionary *)row;
- (NSMutableDictionary *)draft;
- (void)change:(NSString *)path value:(id)value;
- (void)showError:(NSString *)message;
- (BOOL)confirm:(NSString *)title message:(NSString *)message;
- (void)run:(NSArray<NSString *> *)args owner:(NSString *)key name:(NSString *)name title:(NSString *)title quiet:(BOOL)quiet input:(NSData *)input completion:(void(^)(NSString *,int))done;
- (void)operation:(NSArray<NSString *> *)args title:(NSString *)title;
@end
@interface TramesManager (Actions)
- (void)create:(id)sender;
- (void)start:(id)sender;
- (void)newTerminal:(id)sender;
- (void)stop:(id)sender;
- (void)more:(id)sender;
- (void)tools:(id)sender;
- (void)installAPK:(id)sender;
- (void)share:(id)sender;
- (void)backup:(id)sender;
- (void)restore:(id)sender;
- (void)importStore:(id)sender;
- (void)remove:(id)sender;
- (void)doctor:(id)sender;
- (void)createNetwork:(id)sender;
- (void)saveConfiguration:(id)sender;
- (void)discardConfiguration:(id)sender;
- (void)externalEditor:(id)sender;
- (void)permission:(id)sender;
- (void)editPorts:(id)sender;
- (void)removeMount:(id)sender;
- (void)globalSetting:(id)sender;
@end
