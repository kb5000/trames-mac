// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
// Native UI language follows macOS per-app language; never changes guest locale.
static inline NSString *T(NSString *key) {
    return [NSBundle.mainBundle localizedStringForKey:key value:nil table:nil];
}
