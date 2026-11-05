#!/usr/bin/env python3
"""Generate a dependency-free Xcode project; stable IDs keep changes reviewable."""
import hashlib, pathlib, plistlib, json
root = pathlib.Path(__file__).resolve().parents[1]
objects = {}
def put(path, data):
    """Nur bei inhaltlicher Änderung schreiben: Xcode bricht inkrementelle Builds ab,
    wenn Entitlements oder Info.plist während des Builds ein neues Änderungsdatum bekommen."""
    if path.exists() and path.read_bytes() == data: return
    path.write_bytes(data)
def uid(name): return hashlib.sha1(name.encode()).hexdigest()[:24].upper()
def obj(key, **values):
    ident = uid(key); objects[ident] = values; return ident
def configs(name, settings):
    ids = [obj(name + mode, isa='XCBuildConfiguration', name=mode, buildSettings=dict(settings, **({'SWIFT_OPTIMIZATION_LEVEL':'-Onone', 'DEBUG_INFORMATION_FORMAT':'dwarf', 'SWIFT_ACTIVE_COMPILATION_CONDITIONS':'DEBUG'} if mode == 'Debug' else {'SWIFT_OPTIMIZATION_LEVEL':'-O'}))) for mode in ('Debug','Release')]
    return obj(name+'configs', isa='XCConfigurationList', buildConfigurations=ids, defaultConfigurationIsVisible=0, defaultConfigurationName='Debug')
products=[]; files=[]; targets=[]
for name, platform, bundle in [('VoicePhone','iphoneos','de.localvoice.prototype'),('VoiceWatch','watchos','de.localvoice.prototype.watchkitapp')]:
    sources = ['Shared/VoiceApp.swift','Shared/VoiceModel.swift','Shared/VoiceTransport.swift','Shared/CaptureController.swift','Shared/SpeechVoicePicker.swift','VoiceCore/Sources/VoiceCore/Conversation.swift','VoiceCore/Sources/VoiceCore/ProcessingAccess.swift','VoiceCore/Sources/VoiceCore/WatchDestination.swift','VoiceCore/Sources/VoiceCore/Store.swift','VoiceCore/Sources/VoiceCore/Envelope.swift','VoiceCore/Sources/VoiceCore/Chunks.swift','VoiceCore/Sources/VoiceCore/CaptureStartGate.swift','VoiceCore/Sources/VoiceCore/StorageInventory.swift','VoiceCore/Sources/VoiceCore/Jobs.swift','VoiceCore/Sources/VoiceCore/JobProcessor.swift','VoiceCore/Sources/VoiceCore/ProcessingDeadline.swift','VoiceCore/Sources/VoiceCore/ResponsePolicy.swift','VoiceCore/Sources/VoiceCore/LiveMeeting.swift','VoiceCore/Sources/VoiceCore/MeetingIntake.swift']
    if platform == 'iphoneos': sources += ['iPhone/PhoneProcessingRuntime.swift','VoiceCore/Sources/VoiceCore/Meetings.swift','iPhone/LocalProviders.swift','iPhone/CPULocalProviders.swift','iPhone/InferenceCancellation.swift','iPhone/ModelLibrary.swift','iPhone/ModelDownloads.swift','iPhone/ModelPanel.swift','iPhone/MeetingLibrary.swift','iPhone/Intake/IncomingFiles.swift','Shared/ImportConsentView.swift','iPhone/LiveMeeting/LiveCapture.swift','iPhone/LiveMeeting/LiveSummarizer.swift','iPhone/LiveMeeting/LiveMeetingModel.swift','iPhone/LiveMeeting/LiveMeetingViews.swift','VoiceCore/Sources/VoiceCore/Library.swift','VoiceCore/Sources/VoiceCore/BookText.swift','iPhone/Library/PhoneLibrary.swift','iPhone/Library/LibraryPlayback.swift','iPhone/Library/LibraryViews.swift','iPhone/Library/LibrarySync.swift','iPhone/Library/PairingFlow.swift','VoiceCore/Sources/VoiceCore/Pairing.swift','iPhone/Library/PhononEngine.swift','iPhone/Library/ReaderAppearance.swift','Vendor/PhononTTS/Phonon.swift','Vendor/PhononTTS/PhononModels.swift','Vendor/PhononTTS/PhononPlayer.swift','iPhone/Library/LibraryCovers.swift','VoiceCore/Sources/VoiceCore/CoverLookup.swift','iPhone/Library/AudiobookMetadata.swift','iPhone/Library/AudiobookPlayer.swift','VoiceCore/Sources/VoiceCore/Audiobook.swift','VoiceCore/Sources/VoiceCore/SyncCrypto.swift','VoiceCore/Sources/VoiceCore/LibrarySyncModel.swift','VoiceCore/Sources/VoiceCore/AudioOffload.swift','VoiceCore/Sources/VoiceCore/AudioDownload.swift','iPhone/Library/AudioDownloads.swift','iPhone/Engines/Cancellation.mm','iPhone/Engines/WhisperBridge.mm','iPhone/Engines/LlamaBridge.mm','VoiceCore/Sources/VoiceCore/BookTalkIndex.swift','VoiceCore/Sources/VoiceCore/BookTalkSearch.swift','VoiceCore/Sources/VoiceCore/BookTalkIntent.swift','VoiceCore/Sources/VoiceCore/BookTalkCitations.swift','VoiceCore/Sources/VoiceCore/BookTalkPrompt.swift','VoiceCore/Sources/VoiceCore/BookTalkTypes.swift','VoiceCore/Sources/VoiceCore/BookTalkSession.swift','VoiceCore/Sources/VoiceCore/BookTalkStore.swift','VoiceCore/Sources/VoiceCore/BookTalkChat.swift','VoiceCore/Sources/VoiceCore/BookTalkVoice.swift','iPhone/BookTalk/BookTalkModels.swift','iPhone/BookTalk/BookTalkSpeaker.swift','iPhone/BookTalk/BookTalkListener.swift','iPhone/BookTalk/BookTalkController.swift','iPhone/BookTalk/BookTalkSheet.swift']
    buildfiles=[]
    for path in sources:
        ref=obj(path, isa='PBXFileReference', lastKnownFileType='sourcecode.cpp.objcpp' if path.endswith('.mm') else 'sourcecode.swift', path=path, sourceTree='<group>')
        if ref not in files: files.append(ref)
        buildfiles.append(obj(name+path, isa='PBXBuildFile', fileRef=ref))
    sourcephase=obj(name+'sources', isa='PBXSourcesBuildPhase', buildActionMask=2147483647, files=buildfiles, runOnlyForDeploymentPostprocessing=0)
    frameworks=obj(name+'frameworks', isa='PBXFrameworksBuildPhase', buildActionMask=2147483647, files=[], runOnlyForDeploymentPostprocessing=0)
    product=obj(name+'product', isa='PBXFileReference', explicitFileType='wrapper.application', includeInIndex=0, path=name+'.app', sourceTree='BUILT_PRODUCTS_DIR'); products.append(product)
    info={'CFBundleDisplayName':'Local Voice', 'CFBundleIdentifier':'$(PRODUCT_BUNDLE_IDENTIFIER)', 'CFBundleExecutable':'$(EXECUTABLE_NAME)', 'CFBundleName':'$(PRODUCT_NAME)', 'CFBundlePackageType':'APPL', 'CFBundleShortVersionString':'0.2.0', 'CFBundleVersion':'2', 'NSMicrophoneUsageDescription':'Nimmt deine ausdrücklich gestarteten Sprachnotizen, Besprechungen und Fragen im Buch-Gespräch auf.', 'NSSpeechRecognitionUsageDescription':'Transkribiert deine Sprachnotizen, Besprechungen und Fragen im Buch-Gespräch lokal auf dem Gerät.'}
    if platform=='watchos': info.update(WKApplication=True, WKCompanionAppBundleIdentifier='de.localvoice.prototype', CFBundleURLTypes=[{'CFBundleURLName':'de.localvoice.watch.navigation', 'CFBundleURLSchemes':['localvoice-watch']}])
    else: info.update(UILaunchScreen={}, UISupportedInterfaceOrientations=['UIInterfaceOrientationPortrait'])
    if platform == 'iphoneos': info.update({'NSCameraUsageDescription':'Scannt den Kopplungscode, den die Desktop-App am PC anzeigt.',
        'CFBundleURLTypes':[{'CFBundleURLName':'de.localvoice.pairing','CFBundleURLSchemes':['localvoice']}],  # localvoice://pair aus der System-Kamera
        'UIBackgroundModes':['audio','processing'], 'BGTaskSchedulerPermittedIdentifiers':['de.localvoice.prototype.process-recordings'],
        # „Öffnen in“ Local Voice für Audio und Video (Sprachmemos .m4a/.qta, Aufnahmen anderer Apps). Kopie statt Bearbeiten am Ort.
        'CFBundleDocumentTypes':[{'CFBundleTypeName':'Audio- und Videoaufnahme','CFBundleTypeRole':'Viewer','LSHandlerRank':'Alternate',
                                  'LSItemContentTypes':['public.audio','public.movie','public.audiovisual-content','com.apple.quicktime-audio','com.apple.m4a-audio']}],
        'LSSupportsOpeningDocumentsInPlace':False})
    put(root/(name+'-Info.plist'), plistlib.dumps(info))
    settings={'PRODUCT_NAME':name,'PRODUCT_BUNDLE_IDENTIFIER':bundle,'INFOPLIST_FILE':name+'-Info.plist','SDKROOT':platform,'SWIFT_VERSION':'5.0','TARGETED_DEVICE_FAMILY':'1' if platform=='iphoneos' else '4','CODE_SIGN_STYLE':'Automatic','GENERATE_INFOPLIST_FILE':'NO','SUPPORTED_PLATFORMS':'iphoneos iphonesimulator' if platform=='iphoneos' else 'watchos watchsimulator','IPHONEOS_DEPLOYMENT_TARGET':'26.0','WATCHOS_DEPLOYMENT_TARGET':'26.0','ENABLE_USER_SCRIPT_SANDBOXING':'YES'}
    settings['SKIP_INSTALL'] = 'NO' if platform == 'iphoneos' else 'YES'
    settings['ASSETCATALOG_COMPILER_APPICON_NAME'] = 'AppIcon'
    asset_path = 'Shared/Assets.xcassets'
    asset_ref = obj(asset_path, isa='PBXFileReference', lastKnownFileType='folder.assetcatalog', path=asset_path, sourceTree='<group>')
    if asset_ref not in files: files.append(asset_ref)
    asset_build = obj(name+'assets', isa='PBXBuildFile', fileRef=asset_ref)
    # Datenschutz-Manifest (Required Reason APIs: freier Speicher, Dateizeiten, UserDefaults).
    privacy_path = 'Shared/PrivacyInfo.xcprivacy'
    privacy_ref = obj(privacy_path, isa='PBXFileReference', lastKnownFileType='text.xml', path=privacy_path, sourceTree='<group>')
    if privacy_ref not in files: files.append(privacy_ref)
    privacy_build = obj(name+'privacy', isa='PBXBuildFile', fileRef=privacy_ref)
    resources = obj(name+'resources', isa='PBXResourcesBuildPhase', buildActionMask=2147483647, files=[asset_build, privacy_build], runOnlyForDeploymentPostprocessing=0)
    extra_phases=[resources]
    if platform == 'iphoneos':
        settings['CODE_SIGN_ENTITLEMENTS'] = 'VoicePhone.entitlements'
        # Phonon nur im Gerätebau: Das Paket ist arm64-only, Simulator-Builds auf Intel-Macs linken es nicht.
        phonon_slice = '$(SRCROOT)/Vendor/PhononCore.xcframework/ios-arm64'
        settings.update({'EXCLUDED_SOURCE_FILE_NAMES[sdk=iphonesimulator*]':'Phonon.swift PhononModels.swift PhononPlayer.swift',
            'SWIFT_ACTIVE_COMPILATION_CONDITIONS[sdk=iphoneos*]':'$(inherited) PHONON',
            'SWIFT_INCLUDE_PATHS[sdk=iphoneos*]':'$(inherited) '+phonon_slice+'/Headers',
            'LIBRARY_SEARCH_PATHS[sdk=iphoneos*]':'$(inherited) '+phonon_slice,
            'OTHER_LDFLAGS[sdk=iphoneos*]':'$(inherited) -lptts_ffi -lc++ -framework CoreML -framework CoreVideo -framework Accelerate -framework AVFoundation'})
        settings.update({'SWIFT_OBJC_BRIDGING_HEADER':'iPhone/Engines/LVEngines.h','CLANG_CXX_LANGUAGE_STANDARD':'c++17','LD_RUNPATH_SEARCH_PATHS':'$(inherited) @executable_path/Frameworks'})
        embedded=[]
        for library in ['whisper','llama']:
            ref=obj(library+'frameworkRef',isa='PBXFileReference',lastKnownFileType='wrapper.xcframework',path='Vendor/'+library+'.xcframework',sourceTree='<group>'); files.append(ref)
            link=obj(library+'frameworkLink',isa='PBXBuildFile',fileRef=ref)
            objects[frameworks]['files'].append(link)
            embedded.append(obj(library+'frameworkEmbed',isa='PBXBuildFile',fileRef=ref,settings={'ATTRIBUTES':['CodeSignOnCopy','RemoveHeadersOnCopy']}))
        extra_phases.append(obj('nativeEnginesEmbed',isa='PBXCopyFilesBuildPhase',buildActionMask=2147483647,dstPath='',dstSubfolderSpec=10,files=embedded,name='Embed iPhone Engines',runOnlyForDeploymentPostprocessing=0))
    target=obj(name, isa='PBXNativeTarget', buildConfigurationList=configs(name,settings), buildPhases=[sourcephase, frameworks]+extra_phases, buildRules=[], dependencies=[], name=name, productName=name, productReference=product, productType='com.apple.product-type.application'); targets.append(target)
# WidgetKit complications are a separate, lightweight Watch extension.
name = 'VoiceComplications'
widget_files = []
for path in ['WatchWidgets/VoiceComplications.swift', 'VoiceCore/Sources/VoiceCore/WatchDestination.swift']:
    ref = obj(path, isa='PBXFileReference', lastKnownFileType='sourcecode.swift', path=path, sourceTree='<group>')
    if ref not in files: files.append(ref)
    widget_files.append(obj(name+path, isa='PBXBuildFile', fileRef=ref))
phase = obj(name+'sources', isa='PBXSourcesBuildPhase', buildActionMask=2147483647, files=widget_files, runOnlyForDeploymentPostprocessing=0)
product = obj(name+'product', isa='PBXFileReference', explicitFileType='wrapper.app-extension', includeInIndex=0, path=name+'.appex', sourceTree='BUILT_PRODUCTS_DIR'); products.append(product)
put(root/(name+'-Info.plist'), plistlib.dumps({'CFBundleDisplayName':'Local Voice AI', 'CFBundleIdentifier':'$(PRODUCT_BUNDLE_IDENTIFIER)', 'CFBundleExecutable':'$(EXECUTABLE_NAME)', 'CFBundleName':'$(PRODUCT_NAME)', 'CFBundlePackageType':'XPC!', 'CFBundleShortVersionString':'0.2.0', 'CFBundleVersion':'2', 'NSExtension':{'NSExtensionPointIdentifier':'com.apple.widgetkit-extension'}}))
settings = {'PRODUCT_NAME':name, 'PRODUCT_BUNDLE_IDENTIFIER':'de.localvoice.prototype.watchkitapp.complications', 'INFOPLIST_FILE':name+'-Info.plist', 'SDKROOT':'watchos', 'SWIFT_VERSION':'5.0', 'TARGETED_DEVICE_FAMILY':'4', 'CODE_SIGN_STYLE':'Automatic', 'GENERATE_INFOPLIST_FILE':'NO', 'SUPPORTED_PLATFORMS':'watchos watchsimulator', 'WATCHOS_DEPLOYMENT_TARGET':'26.0', 'APPLICATION_EXTENSION_API_ONLY':'YES', 'SKIP_INSTALL':'YES', 'LD_RUNPATH_SEARCH_PATHS':'$(inherited) @executable_path/Frameworks @executable_path/../../Frameworks'}
target = obj(name, isa='PBXNativeTarget', buildConfigurationList=configs(name,settings), buildPhases=[phase], buildRules=[], dependencies=[], name=name, productName=name, productReference=product, productType='com.apple.product-type.app-extension'); targets.append(target)
proxy = obj(name+'proxy', isa='PBXContainerItemProxy', containerPortal=uid('project'), proxyType=1, remoteGlobalIDString=target, remoteInfo=name)
dependency = obj(name+'dependency', isa='PBXTargetDependency', target=target, targetProxy=proxy)
objects[uid('VoiceWatch')]['dependencies'].append(dependency)
embedfile = obj(name+'embedfile', isa='PBXBuildFile', fileRef=product, settings={'ATTRIBUTES':['RemoveHeadersOnCopy']})
embed = obj(name+'embed', isa='PBXCopyFilesBuildPhase', buildActionMask=2147483647, dstPath='', dstSubfolderSpec=13, files=[embedfile], name='Embed Watch Complications', runOnlyForDeploymentPostprocessing=0)
objects[uid('VoiceWatch')]['buildPhases'].append(embed)
proxy=obj('watchProxy',isa='PBXContainerItemProxy',containerPortal=uid('project'),proxyType=1,remoteGlobalIDString=uid('VoiceWatch'),remoteInfo='VoiceWatch')
dependency=obj('watchDependency',isa='PBXTargetDependency',target=uid('VoiceWatch'),targetProxy=proxy)
embedfile=obj('watchEmbedFile',isa='PBXBuildFile',fileRef=uid('VoiceWatchproduct'),settings={'ATTRIBUTES':['RemoveHeadersOnCopy']})
embed=obj('watchEmbed',isa='PBXCopyFilesBuildPhase',buildActionMask=2147483647,dstPath='$(CONTENTS_FOLDER_PATH)/Watch',dstSubfolderSpec=16,files=[embedfile],name='Embed Watch Content',runOnlyForDeploymentPostprocessing=0)
objects[uid('VoicePhone')]['dependencies']=[dependency]
objects[uid('VoicePhone')]['buildPhases'].append(embed)
# Share-Extension (Epic #247, P4): kopiert genau eine Audio-/Videodatei mit Bestätigung in den Eingang der App-Gruppe.
name = 'VoiceShare'
share_files = []
for path in ['ShareExtension/ShareViewController.swift', 'Shared/ImportConsentView.swift', 'VoiceCore/Sources/VoiceCore/LiveMeeting.swift', 'VoiceCore/Sources/VoiceCore/MeetingIntake.swift']:
    ref = obj(path, isa='PBXFileReference', lastKnownFileType='sourcecode.swift', path=path, sourceTree='<group>')
    if ref not in files: files.append(ref)
    share_files.append(obj(name+path, isa='PBXBuildFile', fileRef=ref))
phase = obj(name+'sources', isa='PBXSourcesBuildPhase', buildActionMask=2147483647, files=share_files, runOnlyForDeploymentPostprocessing=0)
# Auch die Extension nutzt UserDefaults der App-Gruppe: eigenes Datenschutz-Manifest (dieselbe Datei).
share_privacy = obj(name+'privacy', isa='PBXBuildFile', fileRef=uid('Shared/PrivacyInfo.xcprivacy'))
share_resources = obj(name+'resources', isa='PBXResourcesBuildPhase', buildActionMask=2147483647, files=[share_privacy], runOnlyForDeploymentPostprocessing=0)
product = obj(name+'product', isa='PBXFileReference', explicitFileType='wrapper.app-extension', includeInIndex=0, path=name+'.appex', sourceTree='BUILT_PRODUCTS_DIR'); products.append(product)
# Genau ein Element mit genau einem Anhang, und der ist Audio oder Video.
rule = ('SUBQUERY (extensionItems, $extensionItem, $extensionItem.attachments.@count == 1 AND '
        'SUBQUERY ($extensionItem.attachments, $attachment, '
        'ANY $attachment.registeredTypeIdentifiers UTI-CONFORMS-TO "public.audiovisual-content").@count == 1).@count == 1 '
        'AND extensionItems.@count == 1')
put(root/(name+'-Info.plist'), plistlib.dumps({'CFBundleDisplayName':'Local Voice', 'CFBundleIdentifier':'$(PRODUCT_BUNDLE_IDENTIFIER)', 'CFBundleExecutable':'$(EXECUTABLE_NAME)', 'CFBundleName':'$(PRODUCT_NAME)', 'CFBundlePackageType':'XPC!', 'CFBundleShortVersionString':'0.2.0', 'CFBundleVersion':'2',
    'NSExtension':{'NSExtensionPointIdentifier':'com.apple.share-services', 'NSExtensionPrincipalClass':'$(PRODUCT_MODULE_NAME).ShareViewController',
                   'NSExtensionAttributes':{'NSExtensionActivationRule':rule}}}))
settings = {'PRODUCT_NAME':name, 'PRODUCT_BUNDLE_IDENTIFIER':'de.localvoice.prototype.share', 'INFOPLIST_FILE':name+'-Info.plist', 'SDKROOT':'iphoneos', 'SWIFT_VERSION':'5.0', 'TARGETED_DEVICE_FAMILY':'1', 'CODE_SIGN_STYLE':'Automatic', 'CODE_SIGN_ENTITLEMENTS':'VoiceShare.entitlements', 'GENERATE_INFOPLIST_FILE':'NO', 'SUPPORTED_PLATFORMS':'iphoneos iphonesimulator', 'IPHONEOS_DEPLOYMENT_TARGET':'26.0', 'APPLICATION_EXTENSION_API_ONLY':'YES', 'SKIP_INSTALL':'YES', 'LD_RUNPATH_SEARCH_PATHS':'$(inherited) @executable_path/Frameworks @executable_path/../../Frameworks'}
target = obj(name, isa='PBXNativeTarget', buildConfigurationList=configs(name,settings), buildPhases=[phase, share_resources], buildRules=[], dependencies=[], name=name, productName=name, productReference=product, productType='com.apple.product-type.app-extension'); targets.append(target)
proxy = obj(name+'proxy', isa='PBXContainerItemProxy', containerPortal=uid('project'), proxyType=1, remoteGlobalIDString=target, remoteInfo=name)
share_dependency = obj(name+'dependency', isa='PBXTargetDependency', target=target, targetProxy=proxy)
embedfile = obj(name+'embedfile', isa='PBXBuildFile', fileRef=product, settings={'ATTRIBUTES':['RemoveHeadersOnCopy']})
share_embed = obj(name+'embed', isa='PBXCopyFilesBuildPhase', buildActionMask=2147483647, dstPath='', dstSubfolderSpec=13, files=[embedfile], name='Embed Share Extension', runOnlyForDeploymentPostprocessing=0)
objects[uid('VoicePhone')]['dependencies'].append(share_dependency)
objects[uid('VoicePhone')]['buildPhases'].append(share_embed)
for path in ['VoicePhone.entitlements', 'VoiceShare.entitlements']:
    entitlements = {'com.apple.security.application-groups':['group.de.localvoice.prototype']}
    # Gemma 4 E2B (3,1 GB Gewichte plus Kontext) liegt ueber dem Standardlimit; nur die App, nicht die Share Extension.
    if path == 'VoicePhone.entitlements': entitlements['com.apple.developer.kernel.increased-memory-limit'] = True
    put(root/path, plistlib.dumps(entitlements))
    ref = obj(path, isa='PBXFileReference', lastKnownFileType='text.plist.entitlements', path=path, sourceTree='<group>')
    if ref not in files: files.append(ref)
# Native UI test targets. They do not change either production app target.
for tested, platform in [('VoicePhone','iphoneos'),('VoiceWatch','watchos')]:
    name=tested+'UITests';path='UITests/VoiceUITests.swift'
    ref=obj(path,isa='PBXFileReference',lastKnownFileType='sourcecode.swift',path=path,sourceTree='<group>')
    if ref not in files: files.append(ref)
    buildfile=obj(name+'sourceFile',isa='PBXBuildFile',fileRef=ref)
    phase=obj(name+'sources',isa='PBXSourcesBuildPhase',buildActionMask=2147483647,files=[buildfile],runOnlyForDeploymentPostprocessing=0)
    product=obj(name+'product',isa='PBXFileReference',explicitFileType='wrapper.cfbundle',includeInIndex=0,path=name+'.xctest',sourceTree='BUILT_PRODUCTS_DIR');products.append(product)
    proxy=obj(name+'proxy',isa='PBXContainerItemProxy',containerPortal=uid('project'),proxyType=1,remoteGlobalIDString=uid(tested),remoteInfo=tested)
    dependency=obj(name+'dependency',isa='PBXTargetDependency',target=uid(tested),targetProxy=proxy)
    settings={'PRODUCT_NAME':name,'PRODUCT_BUNDLE_IDENTIFIER':'de.localvoice.tests.'+name.lower(),'SDKROOT':platform,'SWIFT_VERSION':'5.0','GENERATE_INFOPLIST_FILE':'YES','TEST_TARGET_NAME':tested,'IPHONEOS_DEPLOYMENT_TARGET':'26.0','WATCHOS_DEPLOYMENT_TARGET':'26.0','TARGETED_DEVICE_FAMILY':'1' if platform=='iphoneos' else '4','CODE_SIGN_STYLE':'Automatic'}
    target=obj(name,isa='PBXNativeTarget',buildConfigurationList=configs(name,settings),buildPhases=[phase],buildRules=[],dependencies=[dependency],name=name,productName=name,productReference=product,productType='com.apple.product-type.bundle.ui-testing');targets.append(target)
name='SimulatorControlsUITests'
ref=obj(name+'file',isa='PBXFileReference',lastKnownFileType='sourcecode.swift',path='UITests/SimulatorControlsTests.swift',sourceTree='<group>');files.append(ref)
bf=obj(name+'buildfile',isa='PBXBuildFile',fileRef=ref)
phase=obj(name+'sources',isa='PBXSourcesBuildPhase',buildActionMask=2147483647,files=[bf],runOnlyForDeploymentPostprocessing=0)
product=obj(name+'product',isa='PBXFileReference',explicitFileType='wrapper.cfbundle',includeInIndex=0,path=name+'.xctest',sourceTree='BUILT_PRODUCTS_DIR');products.append(product)
settings={'PRODUCT_NAME':name,'PRODUCT_BUNDLE_IDENTIFIER':'de.localvoice.tests.simulatorcontrols','SDKROOT':'macosx','SWIFT_VERSION':'5.0','GENERATE_INFOPLIST_FILE':'YES','MACOSX_DEPLOYMENT_TARGET':'15.0','CODE_SIGN_IDENTITY':'-','CODE_SIGN_STYLE':'Manual'}
target=obj(name,isa='PBXNativeTarget',buildConfigurationList=configs(name,settings),buildPhases=[phase],buildRules=[],dependencies=[],name=name,productName=name,productReference=product,productType='com.apple.product-type.bundle.ui-testing');targets.append(target)
main=obj('group',isa='PBXGroup',children=files+[obj('products',isa='PBXGroup',children=products,name='Products',sourceTree='<group>')],sourceTree='<group>')
project=obj('project',isa='PBXProject',attributes={'LastUpgradeCheck':'2630','BuildIndependentTargetsInParallel':'YES'},buildConfigurationList=configs('project',{'CLANG_ENABLE_MODULES':'YES'}),compatibilityVersion='Xcode 14.0',developmentRegion='de',hasScannedForEncodings=0,knownRegions=['de','en','Base'],mainGroup=main,productRefGroup=uid('products'),projectDirPath='',projectRoot='',targets=targets)
def render(value):
    if isinstance(value,dict): return '{ '+ ' '.join(json.dumps(k)+' = '+render(v)+';' for k,v in value.items())+' }'
    if isinstance(value,list): return '('+','.join(render(v) for v in value)+')'
    return json.dumps(str(value))
p=root/'LocalVoice.xcodeproj'; p.mkdir(exist_ok=True)
put(p/'project.pbxproj', ('// !$*UTF8*$!\n'+render({'archiveVersion':1,'classes':{},'objectVersion':56,'objects':objects,'rootObject':project})+'\n').encode())

# Explicit shared schemes make command-line UI testing reproducible.
for tested in ['VoicePhone','VoiceWatch']:
    name=tested+'UITests'
    def reference(target, product):
        return f'<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{uid(target)}" BuildableName="{product}" BlueprintName="{target}" ReferencedContainer="container:LocalVoice.xcodeproj"/>'
    scheme=f"""<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="2630" version="1.3">
<BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries>
<BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">{reference(tested,tested+'.app')}</BuildActionEntry>
<BuildActionEntry buildForTesting="YES" buildForRunning="NO" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">{reference(name,name+'.xctest')}</BuildActionEntry>
</BuildActionEntries></BuildAction>
<TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" shouldUseLaunchSchemeArgsEnv="YES"><Testables><TestableReference skipped="NO">{reference(name,name+'.xctest')}</TestableReference></Testables></TestAction>
<LaunchAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" launchStyle="0" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugDocumentVersioning="YES" allowLocationSimulation="YES"><BuildableProductRunnable runnableDebuggingMode="0">{reference(tested,tested+'.app')}</BuildableProductRunnable></LaunchAction>
<ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/>
</Scheme>
"""
    directory=p/'xcshareddata/xcschemes';directory.mkdir(parents=True,exist_ok=True)
    put(directory/(name+'.xcscheme'), (scheme).encode())
    put(directory/(tested+'.xcscheme'), (scheme).encode())

name='SimulatorControlsUITests'
ref=reference(name,name+'.xctest')
put(p/'xcshareddata/xcschemes'/f'{name}.xcscheme', (f"""<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="2630" version="1.3"><BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries><BuildActionEntry buildForTesting="YES" buildForRunning="NO" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">{ref}</BuildActionEntry></BuildActionEntries></BuildAction><TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" shouldUseLaunchSchemeArgsEnv="YES"><Testables><TestableReference skipped="NO">{ref}</TestableReference></Testables></TestAction></Scheme>
""").encode())
