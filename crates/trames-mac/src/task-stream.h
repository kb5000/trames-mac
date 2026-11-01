// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
NS_ASSUME_NONNULL_BEGIN
NSString *TramesConsumeUTF8(NSMutableData *pending, BOOL final);
// Retain the stream until completion; callbacks run on the main queue.
@interface TramesTaskStream : NSObject
- (instancetype)initWithTask:(NSTask *)task
                      onText:(void (^)(NSString *))onText
                  completion:(void (^)(NSString *, int))completion;
- (BOOL)launchAndReturnError:(NSError * _Nullable * _Nullable)error;
@end
NS_ASSUME_NONNULL_END
