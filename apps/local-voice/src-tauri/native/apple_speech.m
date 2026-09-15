#import <Foundation/Foundation.h>
#import <Speech/Speech.h>
#import <AppKit/AppKit.h>

// All recognizer work runs on the app's main queue. Rust waits on a worker;
// late callbacks retain their state and cannot write into freed Rust memory.
@interface LVASpeechJob : NSObject
@property NSLock *lock;
@property dispatch_semaphore_t done;
@property BOOL finished;
@property NSString *text;
@property NSString *error;
@property NSArray *segments;
@property SFSpeechRecognitionTask *task;
@property SFSpeechRecognizer *recognizer;
@property SFSpeechURLRecognitionRequest *request;
- (void)finish:(NSString *)text error:(NSString *)error;
- (void)finishTranscription:(SFTranscription *)transcription;
@end
@implementation LVASpeechJob
- (instancetype)init {
    if ((self = [super init])) {
        _lock = [NSLock new];
        _done = dispatch_semaphore_create(0);
    }
    return self;
}
- (void)finishTranscription:(SFTranscription *)transcription {
    NSMutableArray *segments = [NSMutableArray new];
    for (SFTranscriptionSegment *segment in transcription.segments) {
        [segments addObject:@{@"text": segment.substring,
            @"start_ms": @(llround(segment.timestamp * 1000.0)),
            @"end_ms": @(llround((segment.timestamp + segment.duration) * 1000.0))}];
    }
    [self.lock lock];
    if (!self.finished) self.segments = segments;
    [self.lock unlock];
    [self finish:transcription.formattedString error:nil];
}
- (void)finish:(NSString *)text error:(NSString *)error {
    [self.lock lock];
    if (!self.finished) {
        self.finished = YES;
        self.text = text ?: @"";
        self.error = error ?: @"";
        dispatch_semaphore_signal(self.done);
    }
    [self.lock unlock];
}
@end

// Cache installed on-device variants once per app launch. A generic "en"
// must not resolve to en-GB when only en-US is installed on this Mac.
static NSArray<NSLocale *> *LVAOfflineLocales(void) {
    static NSArray<NSLocale *> *locales;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        NSMutableArray *available = [NSMutableArray new];
        if (@available(macOS 10.15, *)) {
            for (NSLocale *locale in [SFSpeechRecognizer supportedLocales]) {
                SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:locale];
                if (recognizer.supportsOnDeviceRecognition) [available addObject:recognizer.locale];
            }
        }
        locales = [available sortedArrayUsingComparator:^NSComparisonResult(NSLocale *a, NSLocale *b) {
            return [a.localeIdentifier compare:b.localeIdentifier];
        }];
    });
    return locales;
}
static NSLocale *LVASpeechLocale(const char *language) {
    NSString *value = language ? [NSString stringWithUTF8String:language] : nil;
    BOOL automatic = !value.length || [value isEqualToString:@"auto"];
    if (automatic) value = NSLocale.preferredLanguages.firstObject ?: NSLocale.currentLocale.localeIdentifier;
    value = [value stringByReplacingOccurrencesOfString:@"_" withString:@"-"];
    NSLocale *requested = [NSLocale localeWithLocaleIdentifier:value];
    for (NSLocale *locale in LVAOfflineLocales()) {
        if ([[locale.localeIdentifier stringByReplacingOccurrencesOfString:@"_" withString:@"-"] caseInsensitiveCompare:value] == NSOrderedSame) return locale;
    }
    // Preserve an explicit regional choice; generic codes use an installed
    // regional variant of the same language. Never change languages silently.
    if (automatic || ![value containsString:@"-"]) {
        for (NSLocale *locale in LVAOfflineLocales()) {
            if ([locale.languageCode isEqualToString:requested.languageCode]) return locale;
        }
    }
    return requested;
}
char *lva_speech_locales(void) {
    @autoreleasepool {
        NSMutableArray *names = [NSMutableArray new];
        for (NSLocale *locale in LVAOfflineLocales()) [names addObject:locale.localeIdentifier];
        return strdup([[names componentsJoinedByString:@"\n"] UTF8String]);
    }
}
int lva_speech_supported(const char *language) {
    @autoreleasepool {
        NSLocale *wanted = LVASpeechLocale(language);
        return [LVAOfflineLocales() containsObject:wanted];
    }
}
int lva_speech_authorized(void) { return [SFSpeechRecognizer authorizationStatus] == SFSpeechRecognizerAuthorizationStatusAuthorized; }

char *lva_system_default_voice(void) {
    @autoreleasepool {
        NSDictionary *attributes = [NSSpeechSynthesizer attributesForVoice:NSSpeechSynthesizer.defaultVoice];
        NSString *name = attributes[NSVoiceName];
        return name.length ? strdup(name.UTF8String) : NULL;
    }
}

char *lva_speech_transcribe(const char *file, const char *language) {
    @autoreleasepool {
        LVASpeechJob *job = [LVASpeechJob new];
        if (NSThread.isMainThread) {
            return strdup("{\"error\":\"Speech recognition must run on a worker thread\"}");
        }
        NSString *path = [NSString stringWithUTF8String:file];
        NSLocale *locale = LVASpeechLocale(language);
        void (^start)(void) = ^{
            if (job.finished) return;
            if ([SFSpeechRecognizer authorizationStatus] != SFSpeechRecognizerAuthorizationStatusAuthorized) {
                [job finish:nil error:@"Apple speech permission is missing. Enable speech recognition for Local Voice AI in System Settings > Privacy & Security."];
                return;
            }
            if (@available(macOS 10.15, *)) {
                job.recognizer = [[SFSpeechRecognizer alloc] initWithLocale:locale];
                if (!job.recognizer.supportsOnDeviceRecognition) {
                    [job finish:nil error:@"Apple offline speech recognition is unavailable for this language. No audio was sent to a server."];
                    return;
                }
                job.request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:[NSURL fileURLWithPath:path]];
                job.request.requiresOnDeviceRecognition = YES;
                job.request.shouldReportPartialResults = NO;
                job.task = [job.recognizer recognitionTaskWithRequest:job.request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
                    if (result.isFinal) [job finishTranscription:result.bestTranscription];
                    else if (error) [job finish:nil error:error.localizedDescription];
                }];
            } else {
                [job finish:nil error:@"Apple offline speech recognition requires macOS 10.15 or later."];
            }
        };
        dispatch_async(dispatch_get_main_queue(), ^{
            if ([SFSpeechRecognizer authorizationStatus] == SFSpeechRecognizerAuthorizationStatusNotDetermined) {
                [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) {
                    dispatch_async(dispatch_get_main_queue(), start);
                }];
            } else {
                start();
            }
        });
        if (dispatch_semaphore_wait(job.done, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC)) != 0) {
            [job finish:nil error:@"Apple speech recognition timed out. The recording can be retried."];
        }
        [job.lock lock];
        NSDictionary *response = @{@"text": job.text ?: @"", @"error": job.error ?: @"", @"segments": job.segments ?: @[]};
        [job.lock unlock];
        dispatch_async(dispatch_get_main_queue(), ^{
            [job.task cancel];
            job.task = nil;
            job.recognizer = nil;
            job.request = nil;
        });
        NSData *json = [NSJSONSerialization dataWithJSONObject:response options:0 error:nil];
        NSString *string = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
        return strdup(string.UTF8String ?: "{\"error\":\"Invalid speech response\"}");
    }
}
void lva_speech_free(char *value) { free(value); }
