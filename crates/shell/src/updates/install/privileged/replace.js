// Runs under the system AppleScript authorization. Only macOS tools and Foundation run here.
ObjC.import('Foundation');
ObjC.bindFunction('renamex_np', ['int', ['char *', 'char *', 'unsigned int']]);

const swapDirectories = 2; // Darwin RENAME_SWAP.
const files = $.NSFileManager.defaultManager;

function system(program, args) {
    const output = $.NSPipe.pipe;
    const task = $.NSTask.alloc.init;
    task.executableURL = $.NSURL.fileURLWithPath(program);
    task.arguments = args;
    task.standardOutput = output;
    task.standardError = output;
    const launchError = Ref();
    if (!task.launchAndReturnError(launchError)) {
        throw Error('Could not start ' + program + '.');
    }
    const bytes = output.fileHandleForReading.readDataToEndOfFile;
    task.waitUntilExit;
    if (task.terminationStatus !== 0) {
        throw Error(program + ' failed during Quickie replacement.');
    }
    return ObjC.unwrap($.NSString.alloc.initWithDataEncoding(bytes, $.NSUTF8StringEncoding)).trim();
}

function stat(path, format) {
    return system('/usr/bin/stat', ['-f', format, path]);
}

function realDirectory(path) {
    if (stat(path, '%HT') !== 'Directory') {
        return false;
    }
    // Foundation returns /var for the macOS /private/var alias.
    const expected = path.startsWith('/private/var/') ? path.slice('/private'.length) : path;
    return ObjC.unwrap($(path).stringByResolvingSymlinksInPath) === expected;
}

function codeHash(path) {
    const signed = system('/usr/bin/codesign', ['--display', '--arch', 'arm64', '--verbose=4', path]);
    const match = signed.match(/^CDHash=([a-f0-9]{40})$/m);
    return match ? match[1] : '';
}

function verifyFileTypes(path) {
    const unsupported = system('/usr/bin/find',
        ['-P', path, '!', '(', '-type', 'f', '-o', '-type', 'd', '-o', '-type', 'l', ')', '-print', '-quit']);
    if (unsupported !== '') {
        throw Error('The Quickie update contains an unsupported filesystem object.');
    }
}

function verifyCandidate(path, version, hash) {
    if (!realDirectory(path) || codeHash(path) !== hash) {
        throw Error('The verified Quickie update changed during replacement.');
    }
    system('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--all-architectures', path]);
    const plist = path + '/Contents/Info.plist';
    const value = key => system('/usr/bin/plutil', ['-extract', key, 'raw', plist]);
    if (value('CFBundleIdentifier') !== 'dev.quickie.Quickie'
        || value('CFBundleVersion') !== version || value('CFBundleShortVersionString') !== version) {
        throw Error('The protected update failed Quickie bundle identity verification.');
    }
}

function verifyOriginal(path, ownership, hash) {
    if (!realDirectory(path) || stat(path, '%u:%g') !== ownership || codeHash(path) !== hash) {
        throw Error('The protected Quickie installation changed during replacement.');
    }
    system('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--all-architectures', path]);
}

function replace(target, stagingTemplate, args, stageOwner) {
    if (args.length !== 6) {
        throw Error('Invalid Quickie replacement request.');
    }
    const [source, oldHash, newHash, version, owner, group] = args;
    if (!source.startsWith('/') || !realDirectory(source)
        || !/^[a-f0-9]{40}$/.test(oldHash) || !/^[a-f0-9]{40}$/.test(newHash)
        || !/^[0-9]+$/.test(owner) || !/^[0-9]+$/.test(group)
        || !/^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$/.test(version)) {
        throw Error('Invalid Quickie replacement parameters.');
    }
    const ownership = owner + ':' + group;
    const parent = target.slice(0, target.lastIndexOf('/'));
    if (!realDirectory(parent) || stat(parent, '%u') !== stageOwner) {
        throw Error('The Quickie installation is outside the protected Applications directory.');
    }
    verifyOriginal(target, ownership, oldHash);

    const stageParent = stagingTemplate.slice(0, stagingTemplate.lastIndexOf('/'));
    const stageMode = Number.parseInt(stat(stageParent, '%Lp'), 8);
    if (!realDirectory(stageParent) || stat(stageParent, '%u') !== stageOwner || (stageMode & 0o022) !== 0) {
        throw Error('The protected staging directory is unavailable.');
    }
    const stage = system('/usr/bin/mktemp', ['-d', stagingTemplate]);
    const replacement = stage + '/Quickie.app';
    let preserve = false;
    try {
        if (stat(stage, '%d') !== stat(target, '%d')) {
            throw Error('Quickie and the protected staging directory are on different volumes.');
        }
        verifyFileTypes(source);
        verifyCandidate(source, version, newHash);
        system('/usr/bin/ditto', ['--rsrc', '--extattr', '--qtn', '--noacl', source, replacement]);
        verifyFileTypes(replacement);
        system('/bin/chmod', ['-RP', '-N', replacement]);
        system('/bin/chmod', ['-RP', 'u-s,g-s,go-w', replacement]);
        system('/usr/sbin/chown', ['-Rh', ownership, replacement]);
        verifyCandidate(replacement, version, newHash);
        verifyOriginal(target, ownership, oldHash);
        if ($.renamex_np(replacement, target, swapDirectories) !== 0) {
            throw Error('The system could not atomically replace protected Quickie.');
        }
        preserve = true;
        try {
            verifyOriginal(replacement, ownership, oldHash);
            verifyCandidate(target, version, newHash);
            if (stat(target, '%u:%g') !== ownership) {
                throw Error('The Quickie update did not keep the original ownership.');
            }
        } catch (error) {
            if ($.renamex_np(replacement, target, swapDirectories) !== 0) {
                throw Error('Quickie replacement failed verification. The recovery copy is at ' + replacement);
            }
            preserve = false;
            throw error;
        }
        preserve = false;
        return 'QUICKIE_UPDATE_INSTALLED';
    } finally {
        if (!preserve) {
            files.removeItemAtPathError(stage, null);
        }
    }
}

function run(args) {
    return replace('/Applications/Quickie.app', '/private/var/db/quickie-update.XXXXXX',
        args, '0');
}
