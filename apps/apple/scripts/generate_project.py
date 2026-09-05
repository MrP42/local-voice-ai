#!/usr/bin/env python3
"""Generate a dependency-free Xcode project; stable IDs keep changes reviewable."""
import hashlib, pathlib, plistlib, json
root = pathlib.Path(__file__).resolve().parents[1]
objects = {}
def uid(name): return hashlib.sha1(name.encode()).hexdigest()[:24].upper()
def obj(key, **values):
    ident = uid(key); objects[ident] = values; return ident
def configs(name, settings):
    ids = [obj(name + mode, isa='XCBuildConfiguration', name=mode, buildSettings=dict(settings, **({'SWIFT_OPTIMIZATION_LEVEL':'-Onone', 'DEBUG_INFORMATION_FORMAT':'dwarf', 'SWIFT_ACTIVE_COMPILATION_CONDITIONS':'DEBUG'} if mode == 'Debug' else {'SWIFT_OPTIMIZATION_LEVEL':'-O'}))) for mode in ('Debug','Release')]
    return obj(name+'configs', isa='XCConfigurationList', buildConfigurations=ids, defaultConfigurationIsVisible=0, defaultConfigurationName='Debug')
products=[]; files=[]; targets=[]
for name, platform, bundle in [('VoicePhone','iphoneos','de.localvoice.prototype'),('VoiceWatch','watchos','de.localvoice.prototype.watchkitapp')]:
    sources = ['Shared/VoiceApp.swift','Shared/VoiceModel.swift','VoiceCore/Sources/VoiceCore/Store.swift','VoiceCore/Sources/VoiceCore/Envelope.swift','VoiceCore/Sources/VoiceCore/Chunks.swift']
    if platform == 'iphoneos': sources += ['iPhone/LocalProviders.swift','iPhone/CPULocalProviders.swift','iPhone/Engines/WhisperBridge.mm','iPhone/Engines/LlamaBridge.mm']
    buildfiles=[]
    for path in sources:
        ref=obj(path, isa='PBXFileReference', lastKnownFileType='sourcecode.cpp.objcpp' if path.endswith('.mm') else 'sourcecode.swift', path=path, sourceTree='<group>')
        if ref not in files: files.append(ref)
        buildfiles.append(obj(name+path, isa='PBXBuildFile', fileRef=ref))
    sourcephase=obj(name+'sources', isa='PBXSourcesBuildPhase', buildActionMask=2147483647, files=buildfiles, runOnlyForDeploymentPostprocessing=0)
    frameworks=obj(name+'frameworks', isa='PBXFrameworksBuildPhase', buildActionMask=2147483647, files=[], runOnlyForDeploymentPostprocessing=0)
    product=obj(name+'product', isa='PBXFileReference', explicitFileType='wrapper.application', includeInIndex=0, path=name+'.app', sourceTree='BUILT_PRODUCTS_DIR'); products.append(product)
    info={'CFBundleDisplayName':'Local Voice', 'CFBundleIdentifier':'$(PRODUCT_BUNDLE_IDENTIFIER)', 'CFBundleExecutable':'$(EXECUTABLE_NAME)', 'CFBundleName':'$(PRODUCT_NAME)', 'CFBundlePackageType':'APPL', 'CFBundleShortVersionString':'0.1.0', 'CFBundleVersion':'1', 'NSMicrophoneUsageDescription':'Nimmt deine ausdrücklich gestarteten Sprachnotizen auf.', 'NSSpeechRecognitionUsageDescription':'Transkribiert deine Sprachnotizen lokal.'}
    if platform=='watchos': info.update(WKApplication=True, WKCompanionAppBundleIdentifier='de.localvoice.prototype')
    else: info.update(UILaunchScreen={}, UISupportedInterfaceOrientations=['UIInterfaceOrientationPortrait'])
    (root/(name+'-Info.plist')).write_bytes(plistlib.dumps(info))
    settings={'PRODUCT_NAME':name,'PRODUCT_BUNDLE_IDENTIFIER':bundle,'INFOPLIST_FILE':name+'-Info.plist','SDKROOT':platform,'SWIFT_VERSION':'5.0','TARGETED_DEVICE_FAMILY':'1' if platform=='iphoneos' else '4','CODE_SIGN_STYLE':'Automatic','GENERATE_INFOPLIST_FILE':'NO','SUPPORTED_PLATFORMS':'iphoneos iphonesimulator' if platform=='iphoneos' else 'watchos watchsimulator','IPHONEOS_DEPLOYMENT_TARGET':'26.0','WATCHOS_DEPLOYMENT_TARGET':'26.0','ENABLE_USER_SCRIPT_SANDBOXING':'YES'}
    extra_phases=[]
    if platform == 'iphoneos':
        settings.update({'SWIFT_OBJC_BRIDGING_HEADER':'iPhone/Engines/LVEngines.h','CLANG_CXX_LANGUAGE_STANDARD':'c++17','LD_RUNPATH_SEARCH_PATHS':'$(inherited) @executable_path/Frameworks'})
        embedded=[]
        for library in ['whisper','llama']:
            ref=obj(library+'frameworkRef',isa='PBXFileReference',lastKnownFileType='wrapper.xcframework',path='Vendor/'+library+'.xcframework',sourceTree='<group>'); files.append(ref)
            link=obj(library+'frameworkLink',isa='PBXBuildFile',fileRef=ref)
            objects[frameworks]['files'].append(link)
            embedded.append(obj(library+'frameworkEmbed',isa='PBXBuildFile',fileRef=ref,settings={'ATTRIBUTES':['CodeSignOnCopy','RemoveHeadersOnCopy']}))
        extra_phases.append(obj('nativeEnginesEmbed',isa='PBXCopyFilesBuildPhase',buildActionMask=2147483647,dstPath='',dstSubfolderSpec=10,files=embedded,name='Embed iPhone Engines',runOnlyForDeploymentPostprocessing=0))
    target=obj(name, isa='PBXNativeTarget', buildConfigurationList=configs(name,settings), buildPhases=[sourcephase, frameworks]+extra_phases, buildRules=[], dependencies=[], name=name, productName=name, productReference=product, productType='com.apple.product-type.application'); targets.append(target)
proxy=obj('watchProxy',isa='PBXContainerItemProxy',containerPortal=uid('project'),proxyType=1,remoteGlobalIDString=uid('VoiceWatch'),remoteInfo='VoiceWatch')
dependency=obj('watchDependency',isa='PBXTargetDependency',target=uid('VoiceWatch'),targetProxy=proxy)
embedfile=obj('watchEmbedFile',isa='PBXBuildFile',fileRef=uid('VoiceWatchproduct'),settings={'ATTRIBUTES':['RemoveHeadersOnCopy']})
embed=obj('watchEmbed',isa='PBXCopyFilesBuildPhase',buildActionMask=2147483647,dstPath='$(CONTENTS_FOLDER_PATH)/Watch',dstSubfolderSpec=16,files=[embedfile],name='Embed Watch Content',runOnlyForDeploymentPostprocessing=0)
objects[uid('VoicePhone')]['dependencies']=[dependency]
objects[uid('VoicePhone')]['buildPhases'].append(embed)
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
(p/'project.pbxproj').write_text('// !$*UTF8*$!\n'+render({'archiveVersion':1,'classes':{},'objectVersion':56,'objects':objects,'rootObject':project})+'\n')

# Explicit shared schemes make command-line UI testing reproducible.
for tested in ['VoicePhone','VoiceWatch']:
    name=tested+'UITests'
    def reference(target, product):
        return f'<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{uid(target)}" BuildableName="{product}" BlueprintName="{target}" ReferencedContainer="container:LocalVoice.xcodeproj"/>'
    scheme=f"""<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="2630" version="1.3">
<BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries>
<BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">{reference(tested,tested+'.app')}</BuildActionEntry>
<BuildActionEntry buildForTesting="YES" buildForRunning="NO" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">{reference(name,name+'.xctest')}</BuildActionEntry>
</BuildActionEntries></BuildAction>
<TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" shouldUseLaunchSchemeArgsEnv="YES"><Testables><TestableReference skipped="NO">{reference(name,name+'.xctest')}</TestableReference></Testables></TestAction>
<LaunchAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" launchStyle="0" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugDocumentVersioning="YES" allowLocationSimulation="YES"><BuildableProductRunnable runnableDebuggingMode="0">{reference(tested,tested+'.app')}</BuildableProductRunnable></LaunchAction>
</Scheme>
"""
    directory=p/'xcshareddata/xcschemes';directory.mkdir(parents=True,exist_ok=True)
    (directory/(name+'.xcscheme')).write_text(scheme)
    (directory/(tested+'.xcscheme')).write_text(scheme)

name='SimulatorControlsUITests'
ref=reference(name,name+'.xctest')
(p/'xcshareddata/xcschemes'/f'{name}.xcscheme').write_text(f"""<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="2630" version="1.3"><BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries><BuildActionEntry buildForTesting="YES" buildForRunning="NO" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">{ref}</BuildActionEntry></BuildActionEntries></BuildAction><TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" shouldUseLaunchSchemeArgsEnv="YES"><Testables><TestableReference skipped="NO">{ref}</TestableReference></Testables></TestAction></Scheme>
""")
